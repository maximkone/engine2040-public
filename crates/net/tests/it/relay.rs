//! Relay acceptance: invite-code join plus hole punch with relay fallback.
//!
//! The session join path rides the QUIC Transport behind SimQuicNet: invites
//! and signals as reliable streams, inputs and states as datagrams direct or
//! as relay forwards when the punch machine says so. Seeded twice throughout.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[cfg(feature = "quic")]
use engine_net::relay::{decide_join, RelayStats};
use engine_net::relay::{decode_binding_response, encode_binding_request, gather_candidates};
use engine_net::relay::{
    CandidateKind, HolePunch, NatKind, PunchState, RefuseCode, RelayError, RelayMsg, StunServer,
    INVITE_BYTES, INVITE_TTL_TICKS, MAX_RELAY_BYTES, MAX_RELAY_PEERS, MAX_RELAY_QUEUE,
    PUNCH_TIMEOUT_TICKS, RELAY_WIRE_MAGIC, RELAY_WIRE_VERSION, STUN_ATTR_XOR_MAPPED,
    STUN_BINDING_REQUEST, STUN_BINDING_RESPONSE, STUN_MAGIC_COOKIE,
};

#[cfg(feature = "quic")]
use engine_input::fighter::{Buttons, ATTACK};
#[cfg(feature = "quic")]
use engine_input::{FighterId, FighterInput};
#[cfg(feature = "quic")]
use engine_net::relay::{InviteCode, InviteTicket, RelayConfig, RelayServer, Signal};
#[cfg(feature = "quic")]
use std::collections::BTreeMap;

#[cfg(feature = "quic")]
use engine_arena::{ArenaConfig, Checkpoint};
#[cfg(feature = "quic")]
use engine_net::quic::{ControlKind, InboundFrame, PeerId, SimQuicFabric, Transport};
#[cfg(feature = "quic")]
use engine_net::session::REDUNDANCY;
#[cfg(feature = "quic")]
use engine_net::session::{self, Client, Server, WIRE_VERSION};

fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
}

/// Static budgets hold: every bound the relay enforces, checked at compile
/// time, so the default (no-quic) run still proves the contract.
const _RELAY_BUDGETS: () = {
    assert!(RELAY_WIRE_MAGIC == 0x4C52, "relay magic pins the wire");
    assert!(RELAY_WIRE_VERSION == 1);
    assert!(MAX_RELAY_BYTES == 1200, "1200 B wins everywhere");
    assert!(MAX_RELAY_QUEUE <= 256 && MAX_RELAY_QUEUE > 0);
    assert!(MAX_RELAY_PEERS <= 64 && MAX_RELAY_PEERS > 0);
    assert!(INVITE_BYTES == 6, "plan sketch: six-byte invite codes");
    assert!(INVITE_TTL_TICKS > 0 && PUNCH_TIMEOUT_TICKS > 0);
    assert!(STUN_MAGIC_COOKIE == 0x2112_A442);
    assert!(STUN_BINDING_REQUEST == 0x0001);
    assert!(STUN_BINDING_RESPONSE == 0x0101);
    assert!(STUN_ATTR_XOR_MAPPED == 0x0020);
    assert!(CandidateKind::preference(CandidateKind::Host) == 126);
};

#[test]
fn relay_static_budgets_hold() {
    // Compile-time asserts above pin the numbers; here the errors stay human
    // AND exact: each family renders its pinned message, and bad wire bytes
    // trigger the matching variant (not just a non-empty string).
    assert_eq!(format!("{}", RelayError::BadInput), "invalid relay input");
    assert_eq!(format!("{}", RelayError::BadMagic), "bad relay magic");
    assert_eq!(
        format!("{}", RelayError::BadKind(9)),
        "unknown relay kind 9"
    );
    assert_eq!(
        format!("{}", RelayError::Truncated),
        "truncated relay packet"
    );
    assert_eq!(
        format!("{}", RelayError::TooLarge),
        "relay payload exceeds size bound"
    );
    assert_eq!(
        format!("{}", RelayError::UnknownPeer(7)),
        "unknown relay peer 7"
    );
    assert_eq!(
        format!("{}", RelayError::Unauthorized),
        "relay token rejected"
    );
    assert_eq!(format!("{}", RelayError::SlotFull), "relay peer slots full");
    assert_eq!(
        format!("{}", RelayError::QueueFull),
        "relay forward queue full"
    );
    assert_eq!(format!("{}", RelayError::Expired), "invite ticket expired");
    // Bad wire bytes trigger the matching variant, not just any error.
    assert_eq!(
        RelayMsg::decode(&[]).expect_err("empty packet fails"),
        RelayError::Truncated
    );
    assert_eq!(
        RelayMsg::decode(&[0x00, 0x00, RELAY_WIRE_VERSION, 0x00]).expect_err("bad magic fails"),
        RelayError::BadMagic
    );
    let oversize = vec![0u8; MAX_RELAY_BYTES + 23];
    assert_eq!(
        RelayMsg::decode(&oversize).expect_err("oversize fails"),
        RelayError::TooLarge
    );
    assert_eq!(
        format!("{:?}", RefuseCode::VersionMismatch),
        "VersionMismatch"
    );
}

/// Path decision without sockets: STUN learns the reflexive address,
/// candidates order host-first, and the NAT shape picks direct or relay.
#[test]
fn relay_hole_punch_decides_path() {
    let stun = StunServer::new(loopback(3478));
    let tx = [5u8; 12];
    let mapped = stun
        .serve(&encode_binding_request(tx))
        .and_then(|response| decode_binding_response(&response, tx))
        .expect("stun learns the public address");
    assert_eq!(mapped, loopback(3478));
    let set =
        gather_candidates(loopback(5000), Some(mapped), Some(loopback(6000))).expect("gather");
    assert_eq!(
        set.iter().map(|c| c.kind).collect::<Vec<_>>(),
        vec![
            CandidateKind::Host,
            CandidateKind::Srflx,
            CandidateKind::Relay
        ]
    );
    for nat in [NatKind::Open, NatKind::Cone] {
        let mut punch = HolePunch::new(nat);
        punch.on_gathered();
        punch.tick(true);
        assert_eq!(punch.state(), PunchState::Direct, "open networks go direct");
        assert!(punch.is_direct());
    }
    let mut punch = HolePunch::new(NatKind::Symmetric);
    punch.on_gathered();
    for _ in 0..PUNCH_TIMEOUT_TICKS {
        punch.tick(true);
    }
    assert!(punch.needs_relay(), "symmetric NAT always relays");
}

#[cfg(feature = "quic")]
const JOIN_TICKS: u32 = 30;
#[cfg(feature = "quic")]
const JOIN_LATENCY: u32 = 2;
#[cfg(feature = "quic")]
const STATE_COVER: usize = 8;
#[cfg(feature = "quic")]
const JOIN_VERSION: u16 = 3;
#[cfg(feature = "quic")]
const JOIN_RULES: u64 = 0x1234_5678;

#[cfg(feature = "quic")]
fn join_config() -> ArenaConfig {
    ArenaConfig {
        seed: 7,
        ticks: JOIN_TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        table_version: 1,
    }
}

#[cfg(feature = "quic")]
fn join_input(tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(0).expect("slot 0"),
        mx: if tick % 20 < 10 { 100 } else { -100 },
        mz: 0,
        facing: 0,
        buttons: if tick.is_multiple_of(30) {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        },
    }
}

/// Invite-handshake parameters: one struct so the wire helper stays small.
#[cfg(feature = "quic")]
struct JoinWire<'a> {
    ticket: &'a InviteTicket,
    version: u16,
    rules: u64,
    token: u64,
    now: u32,
}

/// Invite handshake over reliable streams: client presents the ticket, the
/// host decides, the client reads accept or typed refuse.
#[cfg(feature = "quic")]
fn wire_handshake(
    fabric: &SimQuicFabric,
    client_t: &mut impl Transport,
    server_t: &mut impl Transport,
    client_id: PeerId,
    server_id: PeerId,
    join: JoinWire<'_>,
) -> Signal {
    let request = Signal::Request {
        code: join.ticket.code.as_bytes(),
        version: join.version,
        rules: join.rules,
        token: join.token,
    };
    let bytes = request.encode().expect("request encodes");
    assert!(bytes.len() <= MAX_RELAY_BYTES, "signals fit the relay cap");
    client_t
        .send_reliable(server_id, ControlKind::Lobby, &bytes)
        .expect("signal fits");
    for _ in 0..=JOIN_LATENCY {
        fabric.tick();
    }
    let mut seen: Vec<Signal> = Vec::new();
    for inbound in server_t.recv() {
        if let InboundFrame::Stream { kind, bytes } = inbound.frame {
            assert_eq!(kind, ControlKind::Lobby);
            assert_eq!(inbound.from, client_id);
            seen.push(Signal::decode(&bytes).expect("signal decodes"));
        }
    }
    assert_eq!(seen.len(), 1, "one handshake request arrives");
    let Signal::Request {
        code,
        version,
        rules,
        token,
    } = seen[0]
    else {
        panic!("server reads a join request");
    };
    let reply = decide_join(join.ticket, code, version, rules, token, 1, join.now);
    server_t
        .send_reliable(
            client_id,
            ControlKind::Lobby,
            &reply.encode().expect("reply encodes"),
        )
        .expect("reply fits");
    for _ in 0..=JOIN_LATENCY {
        fabric.tick();
    }
    let mut back: Vec<Signal> = Vec::new();
    for inbound in client_t.recv() {
        if let InboundFrame::Stream { kind, bytes } = inbound.frame {
            assert_eq!(kind, ControlKind::Lobby);
            assert_eq!(inbound.from, server_id);
            back.push(Signal::decode(&bytes).expect("reply decodes"));
        }
    }
    assert_eq!(back.len(), 1, "one handshake reply arrives");
    back.pop().expect("reply present")
}

#[cfg(feature = "quic")]
struct JoinOutcome {
    server: Vec<Checkpoint>,
    client: Vec<Checkpoint>,
    states_applied: u32,
    mispredicts: u64,
    dropped: u64,
    delivered: u64,
}

/// Direct path: hole punch wins, so inputs and states ride fabric datagrams
/// exactly like the QUIC parity run, after an invite-code handshake.
#[cfg(feature = "quic")]
fn run_direct_join(seed: u64) -> JoinOutcome {
    let mut rng = seed;
    let ticket = InviteTicket::issue(InviteCode::mint(&mut rng), JOIN_VERSION, JOIN_RULES, 0)
        .expect("ticket issues");
    // The human code on the wire names the same secret the host issued.
    let text = ticket.code.encode_str();
    assert_eq!(InviteCode::decode_str(&text), Ok(ticket.code));
    let fabric = SimQuicFabric::new(JOIN_LATENCY, seed);
    let client_id = PeerId(loopback(11001));
    let server_id = PeerId(loopback(11002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let reply = wire_handshake(
        &fabric,
        &mut client_t,
        &mut server_t,
        client_id,
        server_id,
        JoinWire {
            ticket: &ticket,
            version: JOIN_VERSION,
            rules: JOIN_RULES,
            token: 0xA11CE,
            now: 10,
        },
    );
    assert_eq!(
        reply,
        Signal::Accept {
            token: 0xA11CE,
            slot: 1
        },
        "valid invites join"
    );
    let mut punch = HolePunch::new(NatKind::Cone);
    punch.on_gathered();
    punch.tick(true);
    assert!(punch.is_direct(), "cone NAT goes direct");

    let config = join_config();
    let mut server = Server::new(config).expect("server opens");
    let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client opens");
    let mut inbox: BTreeMap<(u32, u8), FighterInput> = BTreeMap::new();
    let mut predicted: Vec<FighterInput> = Vec::new();
    let mut known: Vec<FighterInput> = Vec::new();
    let mut states_applied = 0u32;
    let steps = JOIN_TICKS + 2 * JOIN_LATENCY + 1;
    for step in 0..steps {
        if step < JOIN_TICKS {
            let input = join_input(step);
            client.predict(input).expect("prediction validates");
            predicted.push(input);
            let window = session::redundancy_window(&predicted, step, REDUNDANCY);
            let bytes = session::encode_input(step, &window).expect("input encodes");
            client_t
                .send_unreliable(server_id, &bytes)
                .expect("input fits");
        }
        fabric.tick();
        for inbound in server_t.recv() {
            if let InboundFrame::Datagram(bytes) = inbound.frame {
                if let Ok(packet) = session::decode_input(&bytes) {
                    for input in packet.inputs {
                        let key = (input.tick, input.fighter.0);
                        if input.tick >= server.tick_now() {
                            inbox.entry(key).or_insert(input);
                        }
                    }
                }
            }
        }
        while server.tick_now() < JOIN_TICKS && server.tick_now() + JOIN_LATENCY <= step {
            let want = server.tick_now();
            let mut due: Vec<FighterInput> = inbox
                .iter()
                .filter(|(key, _)| key.0 == want)
                .map(|(_, input)| *input)
                .collect();
            due.sort_by_key(|input| input.fighter.0);
            let (checkpoint, delta) = server.tick(&due).expect("server authors");
            inbox.retain(|key, _| key.0 > want);
            let window = session::redundancy_window(&delta.tape, checkpoint.tick, STATE_COVER);
            let bytes = session::encode_state(checkpoint, &window).expect("state encodes");
            server_t
                .send_unreliable(client_id, &bytes)
                .expect("state fits");
        }
        for inbound in client_t.recv() {
            if let InboundFrame::Datagram(bytes) = inbound.frame {
                if let Ok(packet) = session::decode_state(&bytes) {
                    known.extend_from_slice(&packet.inputs);
                    known.sort_by_key(|input| (input.tick, input.fighter.0));
                    known.dedup_by_key(|input| (input.tick, input.fighter.0));
                    let delta = session::SnapshotDelta {
                        version: WIRE_VERSION,
                        tick: packet.checkpoint.tick,
                        hash: packet.checkpoint.hash,
                        tape: known.clone(),
                    };
                    if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                        states_applied += 1;
                        client.resimulate();
                    }
                }
            }
        }
    }
    client.resimulate();
    JoinOutcome {
        server: server.checkpoints().to_vec(),
        client: client.checkpoints().to_vec(),
        states_applied,
        mispredicts: client.total_mispredicts(),
        dropped: fabric.dropped(),
        delivered: fabric.delivered(),
    }
}

/// Relay path: the punch machine falls back, so the same session bytes ride
/// relay forwards while the fabric carries no game datagrams at all.
#[cfg(feature = "quic")]
fn run_relay_join(seed: u64) -> (JoinOutcome, RelayStats) {
    let mut rng = seed;
    let ticket = InviteTicket::issue(InviteCode::mint(&mut rng), JOIN_VERSION, JOIN_RULES, 0)
        .expect("ticket issues");
    let fabric = SimQuicFabric::new(JOIN_LATENCY, seed);
    let client_id = PeerId(loopback(12001));
    let server_id = PeerId(loopback(12002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let reply = wire_handshake(
        &fabric,
        &mut client_t,
        &mut server_t,
        client_id,
        server_id,
        JoinWire {
            ticket: &ticket,
            version: JOIN_VERSION,
            rules: JOIN_RULES,
            token: 0xBE11,
            now: 10,
        },
    );
    assert_eq!(
        reply,
        Signal::Accept {
            token: 0xBE11,
            slot: 1
        },
        "valid invites join before fallback"
    );
    let mut punch = HolePunch::new(NatKind::Symmetric);
    punch.on_gathered();
    for _ in 0..PUNCH_TIMEOUT_TICKS {
        punch.tick(true);
    }
    assert!(punch.needs_relay(), "symmetric NAT always relays");

    const RELAY_CLIENT: u64 = 12001;
    const RELAY_SERVER: u64 = 12002;
    let mut relay_srv = RelayServer::new(RelayConfig::default());
    let token = relay_srv.config().token;
    relay_srv
        .register(RELAY_CLIENT, token)
        .expect("client registers");
    relay_srv
        .register(RELAY_SERVER, token)
        .expect("server registers");

    let config = join_config();
    let mut server = Server::new(config).expect("server opens");
    let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client opens");
    let mut inbox: BTreeMap<(u32, u8), FighterInput> = BTreeMap::new();
    let mut predicted: Vec<FighterInput> = Vec::new();
    let mut known: Vec<FighterInput> = Vec::new();
    let mut states_applied = 0u32;
    let steps = JOIN_TICKS + 2 * JOIN_LATENCY + 1;
    for step in 0..steps {
        if step < JOIN_TICKS {
            let input = join_input(step);
            client.predict(input).expect("prediction validates");
            predicted.push(input);
            let window = session::redundancy_window(&predicted, step, REDUNDANCY);
            let bytes = session::encode_input(step, &window).expect("input encodes");
            relay_srv
                .forward(RELAY_CLIENT, RELAY_SERVER, &bytes)
                .expect("relay carries inputs");
        }
        fabric.tick();
        for payload in relay_srv.drain(RELAY_SERVER) {
            if let Ok(packet) = session::decode_input(&payload) {
                for input in packet.inputs {
                    let key = (input.tick, input.fighter.0);
                    if input.tick >= server.tick_now() {
                        inbox.entry(key).or_insert(input);
                    }
                }
            }
        }
        while server.tick_now() < JOIN_TICKS && server.tick_now() + JOIN_LATENCY <= step {
            let want = server.tick_now();
            let mut due: Vec<FighterInput> = inbox
                .iter()
                .filter(|(key, _)| key.0 == want)
                .map(|(_, input)| *input)
                .collect();
            due.sort_by_key(|input| input.fighter.0);
            let (checkpoint, delta) = server.tick(&due).expect("server authors");
            inbox.retain(|key, _| key.0 > want);
            let window = session::redundancy_window(&delta.tape, checkpoint.tick, STATE_COVER);
            let bytes = session::encode_state(checkpoint, &window).expect("state encodes");
            relay_srv
                .forward(RELAY_SERVER, RELAY_CLIENT, &bytes)
                .expect("relay carries states");
        }
        for payload in relay_srv.drain(RELAY_CLIENT) {
            if let Ok(packet) = session::decode_state(&payload) {
                known.extend_from_slice(&packet.inputs);
                known.sort_by_key(|input| (input.tick, input.fighter.0));
                known.dedup_by_key(|input| (input.tick, input.fighter.0));
                let delta = session::SnapshotDelta {
                    version: WIRE_VERSION,
                    tick: packet.checkpoint.tick,
                    hash: packet.checkpoint.hash,
                    tape: known.clone(),
                };
                if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                    states_applied += 1;
                    client.resimulate();
                }
            }
        }
    }
    client.resimulate();
    let outcome = JoinOutcome {
        server: server.checkpoints().to_vec(),
        client: client.checkpoints().to_vec(),
        states_applied,
        mispredicts: client.total_mispredicts(),
        dropped: fabric.dropped(),
        delivered: fabric.delivered(),
    };
    (outcome, relay_srv.stats())
}

#[cfg(feature = "quic")]
#[test]
fn relay_invite_join_converges_direct() {
    let run = run_direct_join(0x0A11_CE00_D0ED);
    assert_eq!(run.server.len(), JOIN_TICKS as usize);
    assert_eq!(run.client.len(), JOIN_TICKS as usize);
    assert!(run.states_applied > 0, "server states must land");
    assert!(run.delivered > 0, "frames must cross the link");
    assert_eq!(run.client, run.server, "invite join converges direct");
    let twin = run_direct_join(0x0A11_CE00_D0ED);
    assert_eq!(twin.server, run.server, "same seed, same server truth");
    assert_eq!(twin.client, run.client, "same seed, same convergence");
    assert_eq!(twin.dropped, run.dropped, "same seed, same loss");
    assert_eq!(
        twin.mispredicts, run.mispredicts,
        "same seed, same rollback count"
    );
}

#[cfg(feature = "quic")]
#[test]
fn relay_fallback_carries_session() {
    let (run, stats) = run_relay_join(0x8E1A_4E00_D0ED);
    assert_eq!(run.server.len(), JOIN_TICKS as usize);
    assert_eq!(run.client.len(), JOIN_TICKS as usize);
    assert!(run.states_applied > 0, "relayed states must land");
    assert_eq!(run.client, run.server, "relay fallback converges");
    assert!(stats.forwarded > 0, "game bytes rode the relay");
    assert!(stats.bytes_forwarded > 0, "relay bytes counted");
    assert_eq!(stats.dropped_full, 0, "bounded queues never fill here");
    let (twin, twin_stats) = run_relay_join(0x8E1A_4E00_D0ED);
    assert_eq!(twin.server, run.server, "same seed, same server truth");
    assert_eq!(twin.client, run.client, "same seed, same convergence");
    assert_eq!(twin_stats, stats, "same seed, same relay counters");
    assert_eq!(
        twin.mispredicts, run.mispredicts,
        "same seed, same rollback count"
    );
}

#[cfg(feature = "quic")]
#[test]
fn relay_invite_refusals_over_wire() {
    let mut rng = 0x5EED_1234_5678_9ABC;
    let now = 1000u32;
    let ticket = InviteTicket::issue(InviteCode::mint(&mut rng), JOIN_VERSION, JOIN_RULES, now)
        .expect("ticket issues");
    let fabric = SimQuicFabric::new(JOIN_LATENCY, 0xF005);
    let client_id = PeerId(loopback(13001));
    let server_id = PeerId(loopback(13002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    // Expired tickets refuse with the Expired code, never a slot.
    let stale = wire_handshake(
        &fabric,
        &mut client_t,
        &mut server_t,
        client_id,
        server_id,
        JoinWire {
            ticket: &ticket,
            version: JOIN_VERSION,
            rules: JOIN_RULES,
            token: 0xE1,
            now: now + INVITE_TTL_TICKS + 1,
        },
    );
    assert!(
        matches!(
            stale,
            Signal::Refuse {
                reason: RefuseCode::Expired,
                ..
            }
        ),
        "stale invites refuse as expired"
    );
    // Version drift refuses with both digest pairs on the wire.
    let drifted = wire_handshake(
        &fabric,
        &mut client_t,
        &mut server_t,
        client_id,
        server_id,
        JoinWire {
            ticket: &ticket,
            version: JOIN_VERSION + 1,
            rules: JOIN_RULES,
            token: 0xE2,
            now: now + 5,
        },
    );
    match drifted {
        Signal::Refuse {
            reason,
            want_version,
            got_version,
            want_rules,
            got_rules,
            ..
        } => {
            assert_eq!(reason, RefuseCode::VersionMismatch);
            assert_eq!(
                (want_version, got_version),
                (JOIN_VERSION, JOIN_VERSION + 1)
            );
            assert_eq!((want_rules, got_rules), (JOIN_RULES, JOIN_RULES));
        }
        other => panic!("drift must refuse with digests, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Typed closes: quinn ApplicationClose shape adapted as idea-only
// (Apache-2.0, no vendored bytes). Every close carries a typed code, never
// a bare string: relay denies (0 token, 1 slots full, 2 bad input) and
// join refuses (expired, version mismatch, bad input) roundtrip through
// the wire and reject unknown codes loudly.
// ---------------------------------------------------------------------------

/// Denied codes roundtrip and mean one thing each.
#[test]
fn relay_typed_close_denied_roundtrips() {
    for code in [0u8, 1, 2] {
        let msg = RelayMsg::Denied { client_id: 7, code };
        let bytes = msg.encode().expect("denied encodes");
        assert_eq!(
            RelayMsg::decode(&bytes),
            Ok(msg),
            "typed close survives the wire"
        );
    }
    // Unknown deny codes never encode: the sender refuses before bytes exist.
    assert!(
        RelayMsg::Denied {
            client_id: 7,
            code: 3
        }
        .encode()
        .is_err(),
        "unknown close codes refuse at encode"
    );
    // Denies can name unknown peers, so id zero still encodes.
    let unknown = RelayMsg::Denied {
        client_id: 0,
        code: 0,
    };
    let raw = unknown.encode().expect("denies name unknown peers");
    assert_eq!(
        RelayMsg::decode(&raw),
        Ok(unknown),
        "zero-id denies survive the wire"
    );
}

/// Refuse codes roundtrip through their wire values.
#[test]
fn relay_typed_close_refuse_codes_roundtrip() {
    let codes = [
        (RefuseCode::Expired, 0u8),
        (RefuseCode::VersionMismatch, 1),
        (RefuseCode::BadInput, 2),
    ];
    for (reason, wire) in codes {
        assert_eq!(reason.as_u8(), wire, "wire value pinned");
        assert_eq!(RefuseCode::from_u8(wire), Some(reason), "wire decodes");
    }
    assert_eq!(RefuseCode::from_u8(3), None, "unknown refuse codes fail");
}

/// Denied wire junk refuses loudly: bad magic, bad kind, truncation.
#[test]
fn relay_typed_close_rejects_wire_junk() {
    let good = RelayMsg::Denied {
        client_id: 9,
        code: 1,
    }
    .encode()
    .expect("denied encodes");
    let mut bad_magic = good.clone();
    bad_magic[0] ^= 0xFF;
    assert_eq!(
        RelayMsg::decode(&bad_magic),
        Err(RelayError::BadMagic),
        "bad magic refuses"
    );
    let mut bad_kind = good.clone();
    bad_kind[3] = 0x7F;
    assert_eq!(
        RelayMsg::decode(&bad_kind),
        Err(RelayError::BadKind(0x7F)),
        "unknown kinds refuse"
    );
    assert_eq!(
        RelayMsg::decode(&good[..good.len() - 1]),
        Err(RelayError::Truncated),
        "truncation refuses"
    );
    assert_eq!(
        RelayMsg::decode(&[]),
        Err(RelayError::Truncated),
        "empty closes refuse"
    );
}
