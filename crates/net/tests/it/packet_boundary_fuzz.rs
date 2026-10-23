// Donor: rustls/rustls fuzz layout (fuzz corpus + decode-no-panic pattern), Apache-2.0.
// Link: https://github.com/rustls/rustls (fuzz/ layout). Pattern only, no donor bytes.
// Donor: camshaft/bolero@7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8 (lib/bolero-engine/src/test.rs and book corpus-replay/input-shrinking), MIT.
// Link: https://github.com/camshaft/bolero (LICENSE read live 2026-10-09, MIT).
// Taken: the byte-input `Test` shape for `FnMut(&[u8])` plus the truncation-end
// and byte-shrink transforms, so a refusal that goes wrong names a minimal
// input. Left out: the generator framework, the on-disk corpus export and the
// wall-clock budget; a fixed seed list plus a bounded seeded sweep is
// reproducible and stays inside the suite's time bar.
//! Boundary corpus for the newer hand-written net wire decoders: cells
//! (`cells::decode_packet`), a cut row and a cut packet (`cuts`), the STUN
//! binding and ICE candidate readers (`relay`/`relay_ice`), the handshake
//! token, seal, request/response and index-nonce readers (`handshake`) and the
//! delta rebuild (`delta::decode_delta`, fed from a wire state-delta frame).
//!
//! Each decoder is driven by a fixed seed list plus a 2,000-input engine-seeded
//! sweep: arbitrary bytes decode or return the decoder's typed error (never a
//! panic), every strict prefix of a valid seed refuses with the decoder's named
//! error, a header that claims a wild count refuses at the decoder's own cap,
//! and the same bytes decode the same twice. The session input/state pair is
//! covered by `packet_corpus.rs`; this file starts at the five newer decoders.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use engine_arena::Checkpoint;
use engine_combat::JointId;
use engine_core::Rng;
use engine_net::cells::{decode_packet as decode_cell, encode_packet, CellError, CellPacket};
use engine_net::cuts::{
    decode_cuts, encode_cuts, CutRow, WireCut, CUTS_HEADER_LEN, CUTS_ROW_BYTES, CUT_BYTES,
    MAX_CUTS_PER_PACKET,
};
use engine_net::delta::{Delta, FighterDelta, FighterSnap, FullSnap, MASK_ALL, MASK_X};
use engine_net::handshake::{
    Client, ConnectToken, HandshakeError, Sealed, Server, ServerAddress, SessionKey, SessionKeys,
    TokenGenerator, NONCE_LEN, PROTOCOL_ID,
};
use engine_net::relay::{
    decode_binding_request, decode_binding_request_signed, decode_binding_response,
    decode_binding_response_signed, encode_binding_request, encode_binding_request_signed,
    encode_binding_response, encode_binding_response_signed, Candidate, CandidateKind, RelayError,
    StunCreds,
};
use engine_net::session::{
    decode_state, encode_state_delta, NetError, KIND_CUTS, WIRE_MAGIC, WIRE_VERSION,
};
use engine_net::{decode_delta, encode_delta};

/// Inputs the seeded sweep adds per decoder. Fixed, so the run is reproducible.
const SWEEP_CASES: usize = 2_000;

/// Inline seed corpus: one valid frame per decoder plus the hostile shapes that
/// name its own cap. Fixed bytes, no generator.
mod corpus {
    use super::*;

    /// A valid cell packet: magic `C1`, wire v1, cell 3, tick 9, version 0.
    pub fn cell_valid() -> Vec<u8> {
        encode_packet(&CellPacket {
            tick: 9,
            cell: 3,
            version: 0,
        })
        .to_vec()
    }

    /// The same packet naming cell 255: outside the 4x4 id cap.
    pub fn cell_wild_id() -> Vec<u8> {
        let mut bytes = cell_valid();
        bytes[3] = u8::MAX;
        bytes
    }

    /// The same packet with the magic word flipped.
    pub fn cell_bad_magic() -> Vec<u8> {
        let mut bytes = cell_valid();
        bytes[0] ^= 0xFF;
        bytes
    }

    /// A valid cut row: swing 7, attacker 1, victim 2, a severable joint.
    pub fn cut_row() -> Vec<u8> {
        wire_cut(7, 2).encode().to_vec()
    }

    /// A valid cut packet: two ticks, one ascending row each.
    pub fn cuts_packet() -> Vec<u8> {
        let rows = [
            CutRow {
                tick: 7,
                cut: wire_cut(1, 2),
            },
            CutRow {
                tick: 9,
                cut: wire_cut(2, 3),
            },
        ];
        encode_cuts(&rows).expect("two rows frame")
    }

    /// The widest legal cut packet: 32 rows, one tick.
    pub fn cuts_packet_full() -> Vec<u8> {
        let rows: Vec<CutRow> = (0..MAX_CUTS_PER_PACKET)
            .map(|row| CutRow {
                tick: 3,
                cut: wire_cut(u32::try_from(row).expect("row fits"), 1),
            })
            .collect();
        encode_cuts(&rows).expect("32 rows frame")
    }

    /// A binding request for one fixed transaction id (RFC 8489 shape).
    pub fn binding_request() -> Vec<u8> {
        encode_binding_request([7u8; 12]).to_vec()
    }

    /// A binding response echoing it for 127.0.0.1:40000.
    pub fn binding_response() -> Vec<u8> {
        encode_binding_response([7u8; 12], v4(40_000))
    }

    /// The IPv6 response shape: a 20-byte XOR-MAPPED address.
    pub fn binding_response_v6() -> Vec<u8> {
        encode_binding_response([7u8; 12], v6(40_001))
    }

    /// A signed binding request: USERNAME, PRIORITY, USE-CANDIDATE, tag.
    pub fn signed_request() -> Vec<u8> {
        encode_binding_request_signed([7u8; 12], &creds(), 0x7E00_0000, true)
    }

    /// A signed binding response for the same transaction.
    pub fn signed_response() -> Vec<u8> {
        encode_binding_response_signed([7u8; 12], v4(40_000), &creds())
    }

    /// The short-term credential every signed seed shares.
    pub fn creds() -> StunCreds {
        StunCreds::new(0x0102_0304_0506_0708, [9, 8, 7, 6, 5, 4]).expect("nonzero username")
    }

    /// A host candidate for 127.0.0.1:40000.
    pub fn candidate_v4() -> Vec<u8> {
        Candidate {
            kind: CandidateKind::Host,
            addr: v4(40_000),
        }
        .encode()
    }

    /// A server-reflexive IPv6 candidate for [::1]:40001.
    pub fn candidate_v6() -> Vec<u8> {
        Candidate {
            kind: CandidateKind::Srflx,
            addr: v6(40_001),
        }
        .encode()
    }
}

/// One loopback IPv4 address.
fn v4(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
}

/// One loopback IPv6 address.
fn v6(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)
}

/// The first severable joint of the anatomy.
fn severable_joint() -> u8 {
    JointId::all()
        .iter()
        .copied()
        .find(|joint| joint.is_severable())
        .expect("the anatomy has a severable joint")
        .as_u8()
}

/// One valid cut row for `victim`, swing `swing`.
fn wire_cut(swing: u32, victim: u8) -> WireCut {
    WireCut {
        swing,
        attacker: victim.wrapping_add(1) % 8,
        victim,
        joint: severable_joint(),
        flags: 0,
        energy_q16: 600_000,
        protection_q16: 589_824,
    }
}

/// Bolero byte-input `Test` shape: `run` answers true while `input` still shows
/// the outcome the shrink driver minimizes; `debug_value` renders the input.
trait ByteTest {
    fn run(&mut self, input: &[u8]) -> bool;
    fn debug_value(&self, input: &[u8]) -> String;
}

impl<F> ByteTest for F
where
    F: FnMut(&[u8]) -> bool,
{
    fn run(&mut self, input: &[u8]) -> bool {
        (self)(input)
    }

    fn debug_value(&self, input: &[u8]) -> String {
        format!("{input:02x?}")
    }
}

/// Keep the shortest prefix that still shows the outcome, then lower single
/// bytes. Four bounded passes, no wall clock.
fn shrink<T: ByteTest>(test: &mut T, bytes: Vec<u8>) -> Vec<u8> {
    let mut best = bytes;
    for _ in 0..4 {
        let mut changed = false;
        let mut shortest = None;
        for end in 0..best.len() {
            if test.run(&best[..end]) {
                shortest = Some(end);
                break;
            }
        }
        if let Some(end) = shortest {
            best.truncate(end);
            changed = true;
        }
        if best.is_empty() {
            break;
        }
        for at in 0..best.len() {
            let held = best[at];
            let mut lowered = None;
            for candidate in 0..held {
                best[at] = candidate;
                if test.run(&best) {
                    lowered = Some(candidate);
                    break;
                }
            }
            best[at] = lowered.unwrap_or(held);
            if lowered.is_some() {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    best
}

/// One byte from the engine-owned stream.
fn byte(rng: &mut Rng) -> u8 {
    u8::try_from(rng.next_u64() % 256).expect("byte range fits in u8")
}

/// A value in `0..len` (`len` zero is read as one).
fn pick(rng: &mut Rng, len: usize) -> usize {
    let len = u64::try_from(len.max(1)).expect("range fits in u64");
    usize::try_from(rng.next_u64() % len).expect("index fits in usize")
}

/// Pure noise, zero to 96 bytes.
fn noise(rng: &mut Rng) -> Vec<u8> {
    let len = usize::try_from(rng.next_u64() % 97).expect("len fits in usize");
    (0..len).map(|_| byte(rng)).collect()
}

/// Keep a valid seed's prefix, then flip, truncate or extend its tail, so the
/// decoder is reached past its magic and version checks.
fn mutate(rng: &mut Rng, seeds: &[Vec<u8>]) -> Vec<u8> {
    if seeds.is_empty() {
        return noise(rng);
    }
    let seed = &seeds[pick(rng, seeds.len())];
    if seed.is_empty() {
        return noise(rng);
    }
    let keep = 1 + pick(rng, seed.len());
    let mut bytes = seed[..keep].to_vec();
    for held in seed.iter().skip(keep) {
        bytes.push(if rng.next_u64() & 1 == 0 {
            *held
        } else {
            byte(rng)
        });
    }
    match rng.next_u64() % 4 {
        0 => {
            let len = pick(rng, bytes.len() + 1);
            bytes.truncate(len);
        }
        1 => {
            let extra = pick(rng, 25);
            for _ in 0..extra {
                bytes.push(byte(rng));
            }
        }
        _ => {}
    }
    bytes
}

/// Write a hostile byte into a header-shaped prefix: a wild count, magic or
/// kind word, whatever the decoder reads there.
fn spoof(rng: &mut Rng, seeds: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = mutate(rng, seeds);
    if bytes.is_empty() {
        bytes.push(byte(rng));
    }
    let at = pick(rng, bytes.len().min(16));
    bytes[at] = byte(rng);
    bytes
}

/// The fixed seeds plus 2,000 deterministic inputs: pure noise, mutated seeds
/// and header spoofs. Same decoder id and seeds give the same bytes.
fn generate(decoder: u64, seeds: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(SWEEP_CASES);
    for index in 0..SWEEP_CASES {
        let stream =
            decoder.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::try_from(index).expect("index fits");
        let mut rng = Rng::new(2040).fork(stream);
        let bytes = match rng.next_u64() % 3 {
            0 => noise(&mut rng),
            1 => mutate(&mut rng, seeds),
            _ => spoof(&mut rng, seeds),
        };
        out.push(bytes);
    }
    out
}

/// Drive one decoder over the seeds and the sweep. Each case must decode or
/// return a typed error, and must decode identically twice. Answers
/// `(accepted, rejected)`; a corpus with no valid seed fails, since it would
/// prove nothing.
fn sweep<F, T, E>(name: &str, decoder: u64, seeds: &[Vec<u8>], mut decode: F) -> (usize, usize)
where
    F: FnMut(&[u8]) -> Result<T, E>,
    T: std::fmt::Debug,
    E: std::fmt::Debug,
{
    let cases = generate(decoder, seeds);
    assert_eq!(
        cases.len(),
        SWEEP_CASES,
        "{name}: the sweep stays at {SWEEP_CASES} inputs"
    );
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    for bytes in cases.iter().chain(seeds.iter()) {
        let first = decode(bytes);
        let again = decode(bytes);
        assert_eq!(
            format!("{first:?}"),
            format!("{again:?}"),
            "{name}: decode of {bytes:02x?} must replay identically"
        );
        if first.is_ok() {
            accepted += 1;
        } else {
            rejected += 1;
        }
    }
    assert!(
        accepted >= 1,
        "{name}: the seed corpus must hold one frame that decodes"
    );
    println!(
        "packet_boundary {name}: {} cases ({accepted} accepted, {rejected} rejected), each decoded twice",
        accepted + rejected
    );
    (accepted, rejected)
}

/// Every strict prefix of `valid` must refuse with `expected`; the whole seed
/// must decode. A prefix with the wrong outcome is shrunk and named.
fn truncations_reject<T, E>(
    name: &str,
    valid: &[u8],
    expected: E,
    mut decode: impl FnMut(&[u8]) -> Result<T, E>,
) where
    T: std::fmt::Debug,
    E: PartialEq + std::fmt::Debug,
{
    assert_eq!(
        decode(valid).map(|_| ()),
        Ok(()),
        "{name}: the valid seed must decode"
    );
    for end in 0..valid.len() {
        let got = decode(&valid[..end]);
        if matches!(&got, Err(error) if *error == expected) {
            continue;
        }
        let bad = format!("{got:?}");
        let mut still_bad = |candidate: &[u8]| format!("{:?}", decode(candidate)) == bad;
        let minimal = shrink(&mut still_bad, valid[..end].to_vec());
        let named = still_bad.debug_value(&minimal);
        panic!(
            "{name}: prefix {end} of {} bytes must refuse, got {got:?}; minimal input {named}",
            valid.len()
        );
    }
}

#[test]
fn cell_packet_boundary() {
    let valid = corpus::cell_valid();
    let seeds = vec![
        valid.clone(),
        corpus::cell_wild_id(),
        corpus::cell_bad_magic(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep("cells::decode_packet", 1, &seeds, decode_cell);
    truncations_reject(
        "cells::decode_packet",
        &valid,
        CellError::BadWire,
        decode_cell,
    );
    // The cell id is the only count-shaped field: the decoder's own id cap
    // refuses it, and a 12-byte packet with a huge version still decodes.
    assert_eq!(
        decode_cell(&corpus::cell_wild_id()),
        Err(CellError::BadCell(u8::MAX)),
        "a wild cell id must hit the 4x4 cap, not a length check"
    );
    let mut huge_version = valid.clone();
    huge_version[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_cell(&huge_version),
        Ok(CellPacket {
            tick: 9,
            cell: 3,
            version: u32::MAX,
        }),
        "the cell version is data, not a length"
    );
    println!("packet_boundary cells: {accepted} accepted, {rejected} rejected");
}

#[test]
fn cut_row_boundary() {
    let valid = corpus::cut_row();
    let seeds = vec![valid.clone(), vec![0xFF; CUT_BYTES], Vec::new()];
    let (accepted, rejected) = sweep("cuts::WireCut::decode", 2, &seeds, WireCut::decode);
    truncations_reject(
        "cuts::WireCut::decode",
        &valid,
        NetError::Truncated,
        WireCut::decode,
    );
    // A row's only count-shaped fields are its two slots: equal slots and a
    // non-severable joint refuse as payload, not as a length.
    let mut same_slot = valid.clone();
    same_slot[4] = same_slot[5];
    assert_eq!(WireCut::decode(&same_slot), Err(NetError::BadPayload));
    let mut pelvis = valid.clone();
    pelvis[6] = 0;
    assert_eq!(WireCut::decode(&pelvis), Err(NetError::BadPayload));
    println!("packet_boundary cut rows: {accepted} accepted, {rejected} rejected");
}

#[test]
fn cuts_packet_boundary() {
    let valid = corpus::cuts_packet();
    let seeds = vec![valid.clone(), corpus::cuts_packet_full(), Vec::new()];
    let (accepted, rejected) = sweep("cuts::decode_cuts", 3, &seeds, decode_cuts);
    truncations_reject(
        "cuts::decode_cuts",
        &valid,
        NetError::Truncated,
        decode_cuts,
    );
    let full = corpus::cuts_packet_full();
    assert_eq!(
        full.len(),
        CUTS_HEADER_LEN + MAX_CUTS_PER_PACKET * CUTS_ROW_BYTES,
        "the widest legal frame is header plus the row cap"
    );
    assert_eq!(
        decode_cuts(&full).map(|(last, rows)| (last, rows.len())),
        Ok((3, MAX_CUTS_PER_PACKET)),
        "the cap itself still decodes"
    );
    // A header that claims a wild count refuses at the row cap before the body
    // length is compared, so no body buffer is ever sized from the claim.
    assert_eq!(
        decode_cuts(&cuts_header(0, 0, u8::MAX)),
        Err(NetError::BadPayload),
        "count 255 must hit the 32-row cap, not a length check"
    );
    let just_over = u8::try_from(MAX_CUTS_PER_PACKET + 1).expect("33 fits");
    assert_eq!(
        decode_cuts(&cuts_header(0, 0, just_over)),
        Err(NetError::BadPayload),
        "count 33 must hit the 32-row cap"
    );
    // The legal cap with no body is a length failure, which proves the cap is
    // checked on the header byte alone.
    let at_cap = u8::try_from(MAX_CUTS_PER_PACKET).expect("32 fits");
    assert_eq!(
        decode_cuts(&cuts_header(0, 0, at_cap)),
        Err(NetError::Truncated),
        "a legal count with no rows is truncated, not rejected"
    );
    println!("packet_boundary cut packets: {accepted} accepted, {rejected} rejected");
}

/// A cut packet header only: magic, version, kind, first and last tick, count.
fn cuts_header(first: u32, last: u32, count: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(CUTS_HEADER_LEN);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(KIND_CUTS);
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&last.to_le_bytes());
    out.push(count);
    out
}

#[test]
fn binding_request_boundary() {
    let valid = corpus::binding_request();
    let seeds = vec![
        valid.clone(),
        corpus::binding_response(),
        corpus::candidate_v4(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep(
        "relay::decode_binding_request",
        4,
        &seeds,
        decode_binding_request,
    );
    truncations_reject(
        "relay::decode_binding_request",
        &valid,
        RelayError::Truncated,
        decode_binding_request,
    );
    let mut bad_length = valid.clone();
    bad_length[2] = 1;
    assert_eq!(
        decode_binding_request(&bad_length),
        Err(RelayError::BadInput),
        "a request carrying attributes is not this shape"
    );
    let mut bad_cookie = valid.clone();
    bad_cookie[4] ^= 0xFF;
    assert_eq!(
        decode_binding_request(&bad_cookie),
        Err(RelayError::BadMagic)
    );
    println!("packet_boundary binding requests: {accepted} accepted, {rejected} rejected");
}

#[test]
fn binding_response_boundary() {
    let valid = corpus::binding_response();
    let seeds = vec![
        valid.clone(),
        corpus::binding_response_v6(),
        corpus::binding_request(),
        Vec::new(),
    ];
    let tx = [7u8; 12];
    let mut decode = |bytes: &[u8]| decode_binding_response(bytes, tx);
    let (accepted, rejected) = sweep("relay::decode_binding_response", 5, &seeds, &mut decode);
    truncations_reject(
        "relay::decode_binding_response",
        &valid,
        RelayError::Truncated,
        &mut decode,
    );
    // The attribute length is the count-shaped field: a claim past the bytes
    // present is a length failure before the address is read.
    let mut wild = valid.clone();
    wild[22..24].copy_from_slice(&u16::MAX.to_be_bytes());
    assert_eq!(
        decode(&wild),
        Err(RelayError::Truncated),
        "a wild attribute length must hit the datagram length"
    );
    let mut wrong_tx = valid.clone();
    wrong_tx[8] ^= 0xFF;
    assert_eq!(decode(&wrong_tx), Err(RelayError::BadInput));
    println!("packet_boundary binding responses: {accepted} accepted, {rejected} rejected");
}

#[test]
fn candidate_boundary() {
    let valid = corpus::candidate_v4();
    let seeds = vec![
        valid.clone(),
        corpus::candidate_v6(),
        corpus::binding_request(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep("relay::Candidate::decode", 6, &seeds, Candidate::decode);
    truncations_reject(
        "relay::Candidate::decode",
        &valid,
        RelayError::Truncated,
        Candidate::decode,
    );
    // Only two address families exist: an unknown kind, an unknown family and
    // port zero refuse as payload, never as a length or an allocation.
    assert_eq!(
        Candidate::decode(&[9, 4, 0, 1, 1, 2, 3, 4]),
        Err(RelayError::BadKind(9))
    );
    assert_eq!(
        Candidate::decode(&[0, 9, 0, 1, 1, 2, 3, 4]),
        Err(RelayError::BadInput)
    );
    assert_eq!(
        Candidate::decode(&[0, 4, 0, 0, 1, 2, 3, 4]),
        Err(RelayError::BadInput)
    );
    println!("packet_boundary candidates: {accepted} accepted, {rejected} rejected");
}

#[test]
fn signed_binding_boundary() {
    let creds = corpus::creds();
    let request = corpus::signed_request();
    let response = corpus::signed_response();
    let tx = [7u8; 12];
    let seeds = vec![
        request.clone(),
        response.clone(),
        corpus::binding_request(),
        Vec::new(),
    ];
    let mut check = |bytes: &[u8]| decode_binding_request_signed(bytes, &creds);
    let (accepted, rejected) = sweep(
        "relay::decode_binding_request_signed",
        7,
        &seeds,
        &mut check,
    );
    truncations_reject(
        "relay::decode_binding_request_signed",
        &request,
        RelayError::Truncated,
        &mut check,
    );
    // Deny by default: an unsigned binding is not an authenticated one, and a
    // flipped byte inside the tagged region refuses.
    assert_eq!(
        decode_binding_request_signed(&corpus::binding_request(), &creds),
        Err(RelayError::Unauthorized)
    );
    let mut tampered = request.clone();
    tampered[20] ^= 0x01;
    assert_eq!(
        decode_binding_request_signed(&tampered, &creds),
        Err(RelayError::Unauthorized)
    );
    let mut answer = |bytes: &[u8]| decode_binding_response_signed(bytes, tx, &creds);
    let (accepted_answer, rejected_answer) = sweep(
        "relay::decode_binding_response_signed",
        8,
        &seeds,
        &mut answer,
    );
    truncations_reject(
        "relay::decode_binding_response_signed",
        &response,
        RelayError::Truncated,
        &mut answer,
    );
    println!(
        "packet_boundary signed binding: {accepted} accepted / {rejected} rejected requests, {accepted_answer} accepted / {rejected_answer} rejected responses"
    );
}

/// One fixed handshake: a minted token, a sealed request, a challenge, a
/// response and the server holding the pending session.
struct Handshake {
    token: ConnectToken,
    server: Server,
    request: Vec<u8>,
    challenge: Vec<u8>,
    response: Vec<u8>,
    sealed_token: Vec<u8>,
}

/// A fixed server key so the whole handshake replays.
fn server_key() -> SessionKey {
    let mut key = [0u8; engine_net::handshake::KEY_LEN];
    for (at, lane) in key.iter_mut().enumerate() {
        *lane = u8::try_from(at).expect("lane fits") ^ 0x5A;
    }
    key
}

/// Run one handshake to completion and keep every frame both ends saw.
fn handshake() -> Handshake {
    let key = server_key();
    let mut generator = TokenGenerator::new(PROTOCOL_ID, key, 2040);
    let generated = generator
        .generate(
            100,
            30,
            &[ServerAddress {
                ip: [127, 0, 0, 1],
                port: 40_000,
            }],
        )
        .expect("a one-server token mints");
    let sealed_token = generated.sealed.to_bytes();
    let mut client = Client::new(
        PROTOCOL_ID,
        generated.token.clone(),
        generated.sealed.clone(),
    );
    let request = client.request();
    let mut server = Server::new(PROTOCOL_ID, key, 7);
    let challenge = server
        .on_request(&request, 100)
        .expect("the minted token verifies");
    client
        .on_challenge(&challenge)
        .expect("the server challenge verifies");
    let response = client.response().expect("the challenge was accepted");
    Handshake {
        token: generated.token,
        server,
        request,
        challenge,
        response,
        sealed_token,
    }
}

#[test]
fn handshake_token_boundary() {
    let fixture = handshake();
    let valid = fixture.token.to_bytes();
    let seeds = vec![
        valid.clone(),
        fixture.sealed_token.clone(),
        fixture.request.clone(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep(
        "handshake::ConnectToken::from_bytes",
        9,
        &seeds,
        ConnectToken::from_bytes,
    );
    truncations_reject(
        "handshake::ConnectToken::from_bytes",
        &valid,
        HandshakeError::Malformed,
        ConnectToken::from_bytes,
    );
    // Two count-shaped fields: the server list and the private data. Both caps
    // are read from the header before any buffer is sized.
    let server_count_at = 1 + 8 + 8 + 8 + NONCE_LEN + 4;
    let private_len_at = server_count_at + 4 + 4 + 2;
    let mut wild_servers = valid.clone();
    wild_servers[server_count_at..server_count_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        ConnectToken::from_bytes(&wild_servers),
        Err(HandshakeError::TooLong)
    );
    let mut wild_private = valid.clone();
    wild_private[private_len_at..private_len_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        ConnectToken::from_bytes(&wild_private),
        Err(HandshakeError::TooLong)
    );
    println!("packet_boundary handshake tokens: {accepted} accepted, {rejected} rejected");
}

#[test]
fn handshake_seal_boundary() {
    let fixture = handshake();
    let valid = fixture.sealed_token.clone();
    let seeds = vec![
        valid.clone(),
        fixture.challenge.clone(),
        fixture.response.clone(),
        fixture.request.clone(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep(
        "handshake::Sealed::from_bytes",
        10,
        &seeds,
        Sealed::from_bytes,
    );
    truncations_reject(
        "handshake::Sealed::from_bytes",
        &valid,
        HandshakeError::Malformed,
        Sealed::from_bytes,
    );
    truncations_reject(
        "handshake::Sealed::from_bytes challenge",
        &fixture.challenge,
        HandshakeError::Malformed,
        Sealed::from_bytes,
    );
    // The payload length is the seal's count-shaped field: a wild claim is
    // refused against the bytes present, before any payload buffer is sized.
    let mut wild = fixture.challenge.clone();
    wild[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        Sealed::from_bytes(&wild),
        Err(HandshakeError::Malformed),
        "a wild payload length must hit the datagram length"
    );
    let mut just_over = fixture.challenge.clone();
    just_over[24..28].copy_from_slice(&0xFFFF_u32.to_le_bytes());
    assert_eq!(
        Sealed::from_bytes(&just_over),
        Err(HandshakeError::Malformed)
    );
    // A valid request truncation and a server that sees it.
    let template = Server::new(PROTOCOL_ID, server_key(), 7);
    let mut on_request = |bytes: &[u8]| -> Result<Vec<u8>, HandshakeError> {
        let mut server = template.clone();
        server.on_request(bytes, 100)
    };
    let (request_ok, request_bad) = sweep(
        "handshake::Server::on_request",
        11,
        &[
            fixture.request.clone(),
            fixture.challenge.clone(),
            Vec::new(),
        ],
        &mut on_request,
    );
    truncations_reject(
        "handshake::Server::on_request",
        &fixture.request,
        HandshakeError::Malformed,
        &mut on_request,
    );
    // A client that already accepted the challenge refuses every truncation.
    let mut on_challenge = |bytes: &[u8]| -> Result<(), HandshakeError> {
        let mut client = Client::new(PROTOCOL_ID, fixture.token.clone(), fixture.challenge_seal());
        client.on_challenge(bytes)
    };
    truncations_reject(
        "handshake::Client::on_challenge",
        &fixture.challenge,
        HandshakeError::Malformed,
        &mut on_challenge,
    );
    println!(
        "packet_boundary handshake seals: {accepted} accepted / {rejected} rejected, requests {request_ok} / {request_bad}"
    );
}

impl Handshake {
    /// The challenge as a seal the fixture's client already opened.
    fn challenge_seal(&self) -> Sealed {
        Sealed::from_bytes(&self.challenge).expect("the fixture challenge parses")
    }
}

#[test]
fn handshake_index_nonce_boundary() {
    let fixture = handshake();
    let payload = fixture.challenge_seal().payload;
    let key = fixture.token.server_to_client_key;
    let nonce = fixture.token.nonce;
    let mut decode = |bytes: &[u8]| -> Result<(), HandshakeError> {
        let sealed = Sealed::seal(&key, &nonce, bytes).expect("sealing owns the bytes");
        let mut client = Client::new(PROTOCOL_ID, fixture.token.clone(), sealed.clone());
        client.on_challenge(&sealed.to_bytes())
    };
    let seeds = vec![
        payload.clone(),
        fixture.challenge_seal().to_bytes(),
        fixture.response.clone(),
        Vec::new(),
    ];
    let (accepted, rejected) = sweep(
        "handshake::decode_index_nonce via on_challenge",
        12,
        &seeds,
        &mut decode,
    );
    // Every strict payload prefix re-seals into a frame the index-nonce reader
    // must refuse: four index bytes then exactly one nonce, no trailing bytes.
    truncations_reject(
        "handshake::decode_index_nonce",
        &payload,
        HandshakeError::Malformed,
        &mut decode,
    );
    // The same reader on the server's response path, which parses the payload
    // before the tag: arbitrary bytes never get past it and never change state.
    let template = fixture.server.clone();
    let mut on_response = |bytes: &[u8]| -> Result<SessionKeys, HandshakeError> {
        let mut server = template.clone();
        server.on_response(bytes)
    };
    let (response_ok, response_bad) = sweep(
        "handshake::Server::on_response",
        13,
        &[
            fixture.response.clone(),
            fixture.challenge.clone(),
            fixture.request.clone(),
            Vec::new(),
        ],
        &mut on_response,
    );
    truncations_reject(
        "handshake::Server::on_response",
        &fixture.response,
        HandshakeError::Malformed,
        &mut on_response,
    );
    println!(
        "packet_boundary handshake index-nonce: {accepted} accepted / {rejected} rejected, responses {response_ok} / {response_bad}"
    );
}

/// One baseline fighter at `x_mm`.
fn fighter(slot: u8, x_mm: i32) -> FighterSnap {
    FighterSnap {
        slot,
        x_mm,
        y_mm: 0,
        z_mm: 0,
        yaw_ddeg: 0,
        hp: 1000,
        stamina: 100,
    }
}

/// A valid delta: tick 12 against the tick-10 baseline, slot 0 moved.
fn delta_fixture() -> (FullSnap, FullSnap, Delta) {
    let baseline = FullSnap::new(10, vec![fighter(0, 0), fighter(1, 0)]).expect("baseline");
    let full = FullSnap::new(12, vec![fighter(0, 250), fighter(1, 0)]).expect("full");
    let delta = encode_delta(&full, &baseline).expect("the pair differs in one slot");
    (full, baseline, delta)
}

/// One changed-slot row with a valid mask for `slot`.
fn change(slot: u8) -> FighterDelta {
    FighterDelta {
        mask: MASK_X,
        x_mm: i32::from(slot) + 1,
        y_mm: 0,
        z_mm: 0,
        yaw_ddeg: 0,
        hp: 0,
        stamina: 0,
    }
}

#[test]
fn delta_wire_boundary() {
    let (full, baseline, delta) = delta_fixture();
    assert_eq!(
        decode_delta(&delta, &baseline).expect("the valid delta rebuilds"),
        full,
        "the fixture roundtrips"
    );
    let valid = encode_state_delta(
        Checkpoint {
            tick: 12,
            hash: 0x1122_3344_5566_7788,
        },
        &delta,
    )
    .expect("the delta frames");
    let seeds = vec![valid.clone(), Vec::new()];
    let mut decode = |bytes: &[u8]| -> Result<FullSnap, NetError> {
        let packet = decode_state(bytes)?;
        let Some(delta) = packet.delta else {
            return Err(NetError::BadPayload);
        };
        decode_delta(&delta, &baseline)
    };
    let (accepted, rejected) = sweep(
        "delta::decode_delta via decode_state",
        14,
        &seeds,
        &mut decode,
    );
    truncations_reject(
        "delta::decode_delta wire frame",
        &valid,
        NetError::Truncated,
        &mut decode,
    );
    // The slot count sits in the state header's count byte (delta bit set). A
    // wild claim refuses at the 32-slot cap before any row buffer is sized.
    let mut wild = valid[..17].to_vec();
    wild[16] = engine_net::session::STATE_DELTA_BIT | 127;
    assert_eq!(
        decode_state(&wild),
        Err(NetError::TooManyInputs(127)),
        "a 127-slot claim must hit the 32-slot cap"
    );
    println!("packet_boundary delta wire: {accepted} accepted, {rejected} rejected");
}

#[test]
fn delta_rebuild_boundary() {
    let (_, baseline, delta) = delta_fixture();
    // A wild changed-slot count is refused by the delta's own row cap, read
    // before any buffer for the rows is sized.
    let mut wide = delta.clone();
    wide.changed_slots = (0..33u32)
        .map(|slot| (u8::try_from(slot).expect("slot fits"), change(0)))
        .collect();
    assert_eq!(
        decode_delta(&wide, &baseline),
        Err(NetError::TooManyInputs(33)),
        "33 changed slots must hit the 32-slot cap"
    );
    let mut wider = delta.clone();
    wider.changed_slots = (0..200u32)
        .map(|slot| (u8::try_from(slot % 32).expect("slot fits"), change(0)))
        .collect();
    assert_eq!(
        decode_delta(&wider, &baseline),
        Err(NetError::TooManyInputs(200)),
        "200 changed slots must hit the 32-slot cap"
    );
    // Fields outside the mask set, masked-difference lies and duplicate slots
    // are payload failures, not lengths.
    let mut hidden_bit = delta.clone();
    hidden_bit.changed_slots[0].1.mask |= 0x80;
    assert_eq!(
        decode_delta(&hidden_bit, &baseline),
        Err(NetError::BadState)
    );
    let mut zero_mask = delta.clone();
    zero_mask.changed_slots[0].1.mask = 0;
    assert_eq!(decode_delta(&zero_mask, &baseline), Err(NetError::BadState));
    let mut lie = delta.clone();
    lie.changed_slots[0].1.hp = 7;
    assert_eq!(decode_delta(&lie, &baseline), Err(NetError::BadState));
    let mut twice = delta.clone();
    twice.changed_slots = vec![(0, change(0)), (0, change(0))];
    assert_eq!(decode_delta(&twice, &baseline), Err(NetError::BadState));
    // A baseline truncated by one slot must refuse a change naming the lost
    // slot; the rebuild never drops a change on the floor.
    let short = FullSnap::new(10, vec![fighter(0, 0)]).expect("one-slot baseline");
    let mut orphan = delta.clone();
    orphan.changed_slots = vec![(1, change(1))];
    assert_eq!(
        decode_delta(&orphan, &short),
        Err(NetError::BadState),
        "a change outside the baseline must refuse"
    );
    // The mask set is closed: six bits fill the low bits, so anything above
    // them is a payload failure.
    assert_eq!(MASK_ALL, 0x3F, "six mask bits fill the low six bits");
    // The empty delta is legal: unchanged slots are omitted by design.
    let same = FullSnap::new(10, vec![fighter(0, 0), fighter(1, 0)]).expect("same");
    let empty = encode_delta(&same, &baseline).expect("identical pair");
    assert!(empty.changed_slots.is_empty());
    assert_eq!(decode_delta(&empty, &baseline), Ok(same));
    println!("packet_boundary delta rebuild: caps and fields named");
}
