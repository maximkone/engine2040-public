#![cfg(feature = "quic")]

//! QUIC parity: loopback tapes converge over the QUIC channel mapping,
//! and every wire kind rides both transports byte-identical.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use engine_arena::{ArenaConfig, Checkpoint};
use engine_input::fighter::{Buttons, ATTACK};
use engine_input::{FighterId, FighterInput};
use engine_net::lobby::{BuildId, Msg, RefuseReason};
use engine_net::quic::{
    send_input, send_lobby, send_snapshot, send_state, send_state_delta, ControlKind, Inbound,
    InboundFrame, PeerId, SimQuicFabric, Transport, UdpTransport,
};
use engine_net::session::{
    self, Client, FighterSnap, FullSnap, Interpolator, NetError, Server, SnapshotDelta, REDUNDANCY,
    WIRE_VERSION,
};

const TICKS: u32 = 90;
const LATENCY: u32 = 3;
const STATE_COVER: usize = 8;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn test_config() -> ArenaConfig {
    ArenaConfig {
        seed: 7,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
    }
}

fn local_input(tick: u32) -> FighterInput {
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

/// Outcome of one loopback scenario run through the QUIC channel mapping.
struct ScenarioOutcome {
    server: Vec<Checkpoint>,
    client: Vec<Checkpoint>,
    server_tick: Option<u32>,
    mispredicts: u64,
    states_applied: u32,
    dropped: u64,
    delivered: u64,
    bytes_up: (u64, u64),
    bytes_down: (u64, u64),
}

/// Loopback, tick-for-tick like the UDP session test, but every byte
/// travels through the Transport trait over the deterministic QUIC link:
/// inputs and states as datagrams with the same redundancy windows.
fn run_mp1_scenario() -> ScenarioOutcome {
    let config = test_config();
    // One link, one seed: both directions share the loss matrix.
    let fabric = SimQuicFabric::new(LATENCY, 0xC10C_0000_D0ED);
    let client_id = PeerId(addr(10001));
    let server_id = PeerId(addr(10002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let mut server = Server::new(config).expect("server opens");
    let mut client = Client::new(config, FighterId::new(0).expect("slot 0")).expect("client opens");
    let mut inbox: BTreeMap<(u32, u8), FighterInput> = BTreeMap::new();
    let mut predicted: Vec<FighterInput> = Vec::new();
    // Client-side accumulation of the authoritative tape, mirroring the
    // server-tape merge in the session harness.
    let mut known: Vec<FighterInput> = Vec::new();
    let mut states_applied = 0u32;
    let steps = TICKS + 2 * LATENCY + 1;
    for step in 0..steps {
        if step < TICKS {
            let input = local_input(step);
            client.predict(input).expect("prediction validates");
            predicted.push(input);
            let window = session::redundancy_window(&predicted, step, REDUNDANCY);
            send_input(&mut client_t, server_id, step, &window)
                .expect("input window fits the datagram budget");
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
        while server.tick_now() < TICKS && server.tick_now() + LATENCY <= step {
            let want = server.tick_now();
            let mut due_inputs: Vec<FighterInput> = inbox
                .iter()
                .filter(|(key, _)| key.0 == want)
                .map(|(_, input)| *input)
                .collect();
            due_inputs.sort_by_key(|input| input.fighter.0);
            let (checkpoint, delta) = server.tick(&due_inputs).expect("server authors");
            inbox.retain(|key, _| key.0 > want);
            let window = session::redundancy_window(&delta.tape, checkpoint.tick, STATE_COVER);
            send_state(&mut server_t, client_id, checkpoint, &window)
                .expect("state window fits the datagram budget");
        }
        for inbound in client_t.recv() {
            if let InboundFrame::Datagram(bytes) = inbound.frame {
                if let Ok(packet) = session::decode_state(&bytes) {
                    known.extend_from_slice(&packet.inputs);
                    known.sort_by_key(|input| (input.tick, input.fighter.0));
                    known.dedup_by_key(|input| (input.tick, input.fighter.0));
                    let delta = SnapshotDelta {
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
    ScenarioOutcome {
        server: server.checkpoints().to_vec(),
        client: client.checkpoints().to_vec(),
        server_tick: client.server_tick(),
        mispredicts: client.total_mispredicts(),
        states_applied,
        dropped: fabric.dropped(),
        delivered: fabric.delivered(),
        bytes_up: fabric.bytes_sent(client_id),
        bytes_down: fabric.bytes_sent(server_id),
    }
}

#[test]
fn quic_mp1_loopback_converges() {
    let run = run_mp1_scenario();
    assert_eq!(run.server.len(), TICKS as usize);
    assert_eq!(run.client.len(), TICKS as usize);
    assert_eq!(run.server_tick, Some(TICKS - 1));
    assert!(
        run.states_applied > 0,
        "the client must have applied server states"
    );
    assert!(
        run.delivered > 0,
        "frames must have crossed the link in both directions"
    );
    assert!(
        run.dropped > 0,
        "the loss matrix must have dropped at least one datagram"
    );
    assert!(
        run.mispredicts > 0,
        "loss must have forced at least one rollback resimulation"
    );
    assert_eq!(
        run.client, run.server,
        "client must converge to server truth over QUIC datagrams"
    );
    // Same seed twice is the same session: determinism twin.
    let twin = run_mp1_scenario();
    assert_eq!(twin.server, run.server, "server truth repeats exactly");
    assert_eq!(
        twin.client, run.client,
        "client convergence repeats exactly"
    );
    assert_eq!(twin.dropped, run.dropped, "loss matrix repeats exactly");
}

#[test]
fn quic_kbps_chart_within_cap() {
    let run = run_mp1_scenario();
    // Sim seconds at the 60 Hz tick rate; production sends at 20 Hz, so the
    // harness number is ~3x the deployed rate by construction.
    let seconds = f64::from(TICKS) / 60.0;
    let up_bps = run.bytes_up.0 as f64 * 8.0 / seconds;
    let down_bps = run.bytes_down.0 as f64 * 8.0 / seconds;
    assert!(
        up_bps <= 24_000.0,
        "input path {up_bps:.0} bps over 60 Hz harness cap 24000"
    );
    assert!(
        down_bps <= 96_000.0,
        "state path {down_bps:.0} bps over 60 Hz harness cap 96000"
    );
    assert_eq!(
        (run.bytes_up.1, run.bytes_down.1),
        (0, 0),
        "the MP1 path sends no reliable bytes"
    );
}

/// Drain one frame from a real UDP end, spinning boundedly: loopback
/// delivery is immediate, so arrival within the bound is guaranteed
/// without any timing assertion.
fn recv_frame_udp(transport: &mut UdpTransport) -> Inbound {
    for _ in 0..1000 {
        let mut frames = transport.recv();
        if frames.len() == 1 {
            return frames.pop().expect("one frame");
        }
        assert!(frames.is_empty(), "at most one frame per send here");
    }
    panic!("loopback datagram must arrive");
}

#[test]
fn quic_wire_identical_on_udp() {
    let mut a = UdpTransport::bind_loopback().expect("bind a");
    let mut b = UdpTransport::bind_loopback().expect("bind b");
    let a_id = PeerId(a.local_addr().expect("addr a"));
    let b_id = PeerId(b.local_addr().expect("addr b"));

    // Inputs ride datagrams on both transports.
    let input = local_input(3);
    send_input(&mut a, b_id, 3, &[input]).expect("send input");
    let inbound = recv_frame_udp(&mut b);
    assert_eq!(inbound.from, a_id);
    let InboundFrame::Datagram(bytes) = inbound.frame else {
        panic!("inputs ride datagrams");
    };
    assert_eq!(bytes, session::encode_input(3, &[input]).expect("encode"));
    assert_eq!(
        session::decode_input(&bytes).expect("decode").inputs,
        vec![input]
    );

    // Full states ride datagrams too.
    let checkpoint = Checkpoint {
        tick: 3,
        hash: 0xABCD,
    };
    send_state(&mut a, b_id, checkpoint, &[input]).expect("send state");
    let inbound = recv_frame_udp(&mut b);
    let InboundFrame::Datagram(bytes) = inbound.frame else {
        panic!("full states ride datagrams");
    };
    let state = session::decode_state(&bytes).expect("decode state");
    assert_eq!((state.checkpoint, state.inputs), (checkpoint, vec![input]));
    assert!(state.delta.is_none());

    // Deltas ride datagrams.
    let baseline = FullSnap::new(
        50,
        (0..2)
            .map(|slot| {
                FighterSnap::quantize(slot, [slot as f32, 0.0, 0.0], 0.0, 1000, 100).expect("slot")
            })
            .collect(),
    )
    .expect("baseline");
    let mut full = baseline.clone();
    full.tick = 51;
    full.fighters[1].x_mm += 250;
    let delta = session::encode_delta(&full, &baseline).expect("delta");
    let delta_cp = Checkpoint { tick: 51, hash: 7 };
    send_state_delta(&mut a, b_id, delta_cp, &delta).expect("send delta");
    let inbound = recv_frame_udp(&mut b);
    let InboundFrame::Datagram(bytes) = inbound.frame else {
        panic!("deltas ride datagrams");
    };
    let packet = session::decode_state(&bytes).expect("decode delta");
    assert_eq!(packet.delta.as_ref(), Some(&delta));
    assert_eq!(
        session::decode_delta(packet.delta.as_ref().expect("delta"), &baseline).expect("rebuild"),
        full
    );

    // Snapshots ride reliable streams and arrive byte-identical.
    send_snapshot(&mut a, b_id, &full).expect("send snapshot");
    let inbound = recv_frame_udp(&mut b);
    let InboundFrame::Stream { kind, bytes } = inbound.frame else {
        panic!("snapshots ride streams");
    };
    assert_eq!(kind, ControlKind::Snapshot);
    assert_eq!(bytes, session::encode_snapshot(&full).expect("encode"));
    assert_eq!(session::decode_snapshot(&bytes).expect("decode"), full);

    // Every lobby message rides a reliable stream byte-identical.
    let build = BuildId {
        version: 1,
        rules: 0x1234_5678,
    };
    let messages = [
        Msg::JoinRequest { token: 7, build },
        Msg::JoinGrant {
            token: 7,
            slot: 1,
            fighters: 2,
            build,
        },
        Msg::JoinRefuse {
            token: 7,
            reason: RefuseReason::SlotFull,
            want: build,
            got: build,
        },
        Msg::Ready {
            token: 7,
            ready: true,
        },
        Msg::Start {
            fighters: 2,
            seed: 9,
            build,
            ai_mask: 0b10,
        },
    ];
    for message in messages {
        send_lobby(&mut a, b_id, message).expect("send lobby");
        let inbound = recv_frame_udp(&mut b);
        let InboundFrame::Stream { kind, bytes } = inbound.frame else {
            panic!("lobby rides streams");
        };
        assert_eq!(kind, ControlKind::Lobby);
        assert_eq!(bytes, message.encode().expect("encode lobby"));
        assert_eq!(Msg::decode(&bytes).expect("decode lobby"), message);
    }
}

#[test]
fn quic_lobby_handshake_over_streams() {
    let fabric = SimQuicFabric::new(2, 0xBEEF);
    let client_id = PeerId(addr(20001));
    let server_id = PeerId(addr(20002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let build = BuildId {
        version: 3,
        rules: 0xDEAD_BEEF,
    };
    let exchange = [
        (false, Msg::JoinRequest { token: 11, build }),
        (
            true,
            Msg::JoinGrant {
                token: 11,
                slot: 0,
                fighters: 2,
                build,
            },
        ),
        (
            false,
            Msg::Ready {
                token: 11,
                ready: true,
            },
        ),
        (
            true,
            Msg::Start {
                fighters: 2,
                seed: 77,
                build,
                ai_mask: 0b10,
            },
        ),
    ];
    for (from_server, message) in exchange {
        // A lossy datagram alongside every step proves streams are
        // unaffected by datagram loss.
        let noise = vec![0xFFu8; 24];
        if from_server {
            send_lobby(&mut server_t, client_id, message).expect("stream fits");
            server_t
                .send_unreliable(client_id, &noise)
                .expect("noise fits");
        } else {
            send_lobby(&mut client_t, server_id, message).expect("stream fits");
            client_t
                .send_unreliable(server_id, &noise)
                .expect("noise fits");
        }
        for _ in 0..=LATENCY {
            fabric.tick();
        }
        let (receiver, want_id) = if from_server {
            (&mut client_t, server_id)
        } else {
            (&mut server_t, client_id)
        };
        let mut streams: Vec<Msg> = Vec::new();
        for inbound in receiver.recv() {
            assert_eq!(inbound.from, want_id);
            if let InboundFrame::Stream { kind, bytes } = inbound.frame {
                assert_eq!(kind, ControlKind::Lobby);
                streams.push(Msg::decode(&bytes).expect("lobby decodes"));
            }
        }
        assert_eq!(streams, vec![message], "one ordered message per step");
    }
}

#[test]
fn quic_snapshot_stream_feeds_interpolator() {
    // The stream snapshot consumer is the delta interpolator: two streamed
    // ticks blend at the midpoint.
    let fabric = SimQuicFabric::new(1, 5);
    let client_id = PeerId(addr(30001));
    let server_id = PeerId(addr(30002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let snap = |tick: u32, x: f32| {
        FullSnap::new(
            tick,
            (0..2)
                .map(|slot| {
                    FighterSnap::quantize(slot, [x + slot as f32, 0.0, 0.0], 0.0, 1000, 100)
                        .expect("slot")
                })
                .collect(),
        )
        .expect("snapshot")
    };
    let mut interpolator = Interpolator::new(100);
    for (tick, time_ms) in [(10u32, 0u64), (11, 16)] {
        let full = snap(tick, tick as f32);
        send_snapshot(&mut server_t, client_id, &full).expect("stream fits");
        fabric.tick();
        fabric.tick();
        let mut arrived = Vec::new();
        for inbound in client_t.recv() {
            if let InboundFrame::Stream { kind, bytes } = inbound.frame {
                assert_eq!(kind, ControlKind::Snapshot);
                arrived.push(session::decode_snapshot(&bytes).expect("decodes"));
            }
        }
        assert_eq!(arrived, vec![full.clone()]);
        assert!(interpolator.push(time_ms, full), "valid sample stores");
    }
    let blended = interpolator.at(8).expect("midpoint blends");
    assert!(!blended.clamped, "midpoint is a blend, not a clamp");
    assert_eq!(blended, interpolator.at(8).expect("queries repeat"));
}

#[test]
fn quic_state_delta_datagram_roundtrip() {
    let fabric = SimQuicFabric::new(1, 11);
    let client_id = PeerId(addr(40001));
    let server_id = PeerId(addr(40002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let baseline = FullSnap::new(
        60,
        (0..4)
            .map(|slot| {
                FighterSnap::quantize(slot, [slot as f32, 0.0, 0.0], 0.0, 1000, 100).expect("slot")
            })
            .collect(),
    )
    .expect("baseline");
    let mut full = baseline.clone();
    full.tick = 61;
    full.fighters[0].stamina -= 25;
    let delta = session::encode_delta(&full, &baseline).expect("delta");
    let checkpoint = Checkpoint { tick: 61, hash: 99 };
    send_state_delta(&mut server_t, client_id, checkpoint, &delta).expect("datagram fits");
    fabric.tick();
    fabric.tick();
    let mut packets = Vec::new();
    for inbound in client_t.recv() {
        if let InboundFrame::Datagram(bytes) = inbound.frame {
            packets.push(session::decode_state(&bytes).expect("decodes"));
        }
    }
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].checkpoint, checkpoint);
    assert_eq!(packets[0].delta.as_ref(), Some(&delta));
    assert_eq!(
        session::decode_delta(packets[0].delta.as_ref().expect("delta"), &baseline)
            .expect("rebuild"),
        full
    );
}

#[test]
fn quic_garbage_and_version_rejected_through_transport() {
    // The transport delivers bytes faithfully; the codecs reject loudly.
    let fabric = SimQuicFabric::new(1, 3);
    let client_id = PeerId(addr(50001));
    let server_id = PeerId(addr(50002));
    let mut client_t = fabric.endpoint(client_id.0);
    let mut server_t = fabric.endpoint(server_id.0);
    let input = local_input(0);
    let mut good = session::encode_input(0, &[input]).expect("encode");
    let mut bad_magic = good.clone();
    bad_magic[0] ^= 0xFF;
    let mut bad_version = good.clone();
    bad_version[2] = WIRE_VERSION + 1;
    let truncated = good[..5].to_vec();
    let bad_kind = session::encode_snapshot(
        &FullSnap::new(
            1,
            vec![FighterSnap::quantize(0, [0.0, 0.0, 0.0], 0.0, 100, 100).expect("slot")],
        )
        .expect("snapshot"),
    )
    .expect("encode");
    assert_eq!(bad_kind[3], session::KIND_SNAPSHOT);
    for payload in [&good, &bad_magic, &bad_version, &truncated, &bad_kind] {
        client_t
            .send_unreliable(server_id, payload)
            .expect("fits budget");
    }
    // One extra tick flushes the 1-tick latency for every send above.
    fabric.tick();
    fabric.tick();
    // Append a marker so the end of the stream is observable.
    good[4] ^= 0x01;
    client_t
        .send_unreliable(server_id, &good)
        .expect("fits budget");
    fabric.tick();
    fabric.tick();
    let mut frames = Vec::new();
    for inbound in server_t.recv() {
        if let InboundFrame::Datagram(bytes) = inbound.frame {
            frames.push(bytes);
        }
    }
    assert_eq!(frames.len(), 6, "every datagram arrives faithfully");
    assert_eq!(
        session::decode_input(&frames[0])
            .expect("control decodes")
            .inputs,
        vec![input],
        "the clean packet is the control"
    );
    assert_eq!(
        session::decode_input(&frames[1]),
        Err(NetError::BadMagic),
        "corrupt magic refuses"
    );
    assert_eq!(
        session::decode_input(&frames[2]),
        Err(NetError::VersionMismatch {
            got: WIRE_VERSION + 1
        }),
        "future version refuses"
    );
    assert_eq!(
        session::decode_input(&frames[3]),
        Err(NetError::Truncated),
        "short packet refuses"
    );
    assert_eq!(
        session::decode_input(&frames[4]),
        Err(NetError::BadKind(session::KIND_SNAPSHOT)),
        "snapshot kind refuses on the input path"
    );
    // The mutated tick byte arrives mutated: the transport never normalizes.
    assert_ne!(frames[5], frames[0], "mutated byte arrives mutated");
    assert!(
        client_t.recv().is_empty(),
        "no cross-talk back to the sender"
    );
}
