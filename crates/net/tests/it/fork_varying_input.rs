//! Fork-fix: varying-size input encode (length-prefixed, 20/20 decode, 0 desync).
//!
//! Attribution: shape idea from caspark/ggrs commit 7802ee0 (MIT, verified):
//! length-prefixed input encode resilient to varying sizes. Fresh write
//! against our netcode API (`engine_net::netcode::Session`); no donor bytes
//! here. Each frame is `[len: u32 LE][payload]` so empty, small and large
//! inputs share one wire tape without framing drift.
//!
//! The tape holds 20 mixed-size frames. The decoder must recover all 20 and
//! two session replays over the decoded inputs must hash identically with
//! zero desyncs.

use engine_core::Fnv64;
use engine_net::netcode::{Session, Sim};

/// Mixed-size tape: 20 frames spanning empty, 1-byte, small and large payloads.
const SIZES: [usize; 20] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 1, 0, 9, 12, 3, 5, 2, 16, 4, 6, 10,
];

/// Deterministic toy sim (own code): accumulates both inputs, FNV-folds state.
#[derive(Clone, Default, PartialEq, Eq)]
struct Toy {
    a: i32,
    b: i32,
}

impl Sim for Toy {
    fn step(&mut self, inputs: [u32; 2]) {
        let la = i32::try_from(inputs[0] % 4).unwrap_or(3);
        let ra = i32::try_from(inputs[1] % 4).unwrap_or(3);
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

/// Deterministic payload bytes for `frame` with `size` bytes.
fn payload_for(frame: usize, size: usize) -> Vec<u8> {
    let f = u32::try_from(frame).expect("tape fits u32");
    (0..size)
        .map(|i| {
            let x = u32::try_from(i).expect("payload fits u32");
            (f.wrapping_mul(31)
                .wrapping_add(x.wrapping_mul(17))
                .wrapping_add(1)
                % 251) as u8
        })
        .collect()
}

/// Length-prefixed encode: appends `[len: u32 LE][payload]` to `out`.
fn encode_frame(payload: &[u8], out: &mut Vec<u8>) {
    let len = u32::try_from(payload.len()).expect("payload fits u32");
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
}

/// Decode a concatenated wire tape; `None` on truncation or a bad prefix.
fn decode_tape(wire: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut frames = Vec::new();
    let mut at = 0usize;
    while at < wire.len() {
        if wire.len() - at < 4 {
            return None;
        }
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&wire[at..at + 4]);
        let len = usize::try_from(u32::from_le_bytes(tag)).unwrap_or(usize::MAX);
        at += 4;
        if wire.len() - at < len {
            return None;
        }
        frames.push(wire[at..at + len].to_vec());
        at += len;
    }
    Some(frames)
}

/// Fold varying-size bytes into the fixed `u32` input our session steps.
fn payload_to_u32(payload: &[u8]) -> u32 {
    let mut fold = Fnv64::new();
    for b in payload {
        fold.write_u8(*b);
    }
    u32::try_from(fold.finish() & 0xffff_ffff).expect("masked to 32 bits")
}

/// Build the 20-frame mixed-size tape as one wire buffer.
fn wire_tape() -> Vec<u8> {
    let mut wire = Vec::new();
    for (f, size) in SIZES.iter().enumerate() {
        encode_frame(&payload_for(f, *size), &mut wire);
    }
    wire
}

/// Replay decoded frames through a session, returning the hash stream.
fn replay(frames: &[Vec<u8>]) -> Vec<u64> {
    let mut s = Session::new(Toy::default());
    for (f, p) in frames.iter().enumerate() {
        let v = payload_to_u32(p);
        let w = u32::try_from(f).expect("tape fits u32");
        s.advance(v, Some(v.wrapping_add(w % 4)));
    }
    s.stream()
}

#[test]
fn mixed_tape_decodes_20_of_20() {
    let wire = wire_tape();
    let frames = decode_tape(&wire).expect("full tape decodes");
    assert_eq!(frames.len(), 20, "mixed tape must decode 20/20");
    for (f, (got, size)) in frames.iter().zip(SIZES.iter()).enumerate() {
        assert_eq!(
            *got,
            payload_for(f, *size),
            "frame {f} bytes must round-trip"
        );
    }
}

#[test]
fn replay_hash_identical_x2_zero_desync() {
    let wire = wire_tape();
    let frames = decode_tape(&wire).expect("full tape decodes");
    assert_eq!(frames.len(), 20, "mixed tape must decode 20/20");

    let first = replay(&frames);
    let second = replay(&frames);
    assert_eq!(first, second, "same decoded tape replays identically x2");

    // Cross-peer check over held frames: every comparable hash must agree.
    let (mut a, mut b) = (Session::new(Toy::default()), Session::new(Toy::default()));
    let mut desyncs = 0u32;
    for (f, p) in frames.iter().enumerate() {
        let v = payload_to_u32(p);
        let w = u32::try_from(f).expect("tape fits u32");
        a.advance(v, Some(v.wrapping_add(w % 4)));
        b.advance(v, Some(v.wrapping_add(w % 4)));
    }
    for f in 0..frames.len() {
        let frame = u32::try_from(f).expect("tape fits u32");
        if let Some(peer) = b.hash_at(frame) {
            if a.check_remote(frame, peer).is_err() {
                desyncs += 1;
            }
        }
    }
    assert_eq!(desyncs, 0, "decoded replays must show 0 desync");
    assert_eq!(a.stream(), first, "audit run must match replay tape");
}

#[test]
fn truncated_wire_refuses() {
    let wire = wire_tape();
    // Drop the final byte: the last frame payload is short, decode refuses.
    let cut = decode_tape(&wire[..wire.len() - 1]);
    assert_eq!(cut, None, "truncated tape must refuse");
    // A lone 3-byte tail cannot hold a length prefix.
    assert_eq!(decode_tape(&[1, 2, 3]), None, "short prefix must refuse");
    // A lying length prefix (claims 64 bytes, holds 1) refuses.
    let mut liar = 64u32.to_le_bytes().to_vec();
    liar.push(0xaa);
    assert_eq!(decode_tape(&liar), None, "overlong prefix must refuse");
}
