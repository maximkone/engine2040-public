//! In-process WebTransport proofs: a real `wtransport` client pinned to the
//! server certificate hash plays against [`WtServer`] over loopback.

use std::collections::VecDeque;
use std::future::Future;
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;

use engine_arena::Checkpoint;
use engine_core::Fnv64;
use engine_input::fighter::{Buttons, ATTACK};
use wtransport::endpoint::endpoint_side;
use wtransport::error::ConnectingError;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, VarInt};

use super::cert::{PinnedCert, MAX_VALIDITY_DAYS};
use super::{
    decode_header, encode_message, DepartReason, Departure, WtServer, FRAME_HEADER_LEN,
    MAX_SESSIONS, RELIABLE_FRAMES, SESSION_PATH,
};
use crate::delta::BodySnap;
use crate::lobby::{BuildId, Msg};
use crate::quic::{
    send_lobby, send_snapshot, send_state_delta, ControlKind, Inbound, InboundFrame, PeerId,
    Transport,
};
use crate::session::{
    decode_delta, decode_input, decode_snapshot, decode_state, encode_delta, encode_input,
    encode_snapshot, encode_state_delta, BodyId, BodyInput, FullSnap, NetError, MAX_PACKET_BYTES,
};

/// Ticks of the scripted duel.
const TICKS: u32 = 600;
/// Fighters in every scripted snapshot.
const FIGHTERS: u8 = 8;
/// The tick whose full snapshot also rides the reliable stream.
const KEYFRAME_TICK: u32 = 300;
/// Longest the client waits for one answer before the test fails loudly. It
/// is shorter than the server's polling budget, so a stalled client names
/// what it waited for before the server side gives up.
const WAIT: Duration = Duration::from_secs(10);
/// Polling interval of the synchronous server side.
const POLL_STEP: Duration = Duration::from_millis(1);
/// Polling rounds that add up to [`WAIT`].
const POLL_ROUNDS: u32 = 20_000;

const BUILD: BuildId = BuildId {
    version: 1,
    rules: 0x1234_5678,
};
const TOKEN: u64 = 7;

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn mint() -> PinnedCert {
    PinnedCert::mint(["localhost", "127.0.0.1"], MAX_VALIDITY_DAYS).expect("mint a certificate")
}

/// Scripted snapshot of one tick: every fighter moves, some fields change on
/// few ticks only, so the deltas carry varied masks.
fn snap_at(tick: u32) -> FullSnap {
    let t = i32::try_from(tick).expect("script ticks fit i32");
    let fighters = (0..FIGHTERS)
        .map(|slot| {
            let s = i32::from(slot);
            let lane = if (t / (10 + s)) % 2 == 0 { 1 } else { -1 };
            BodySnap {
                slot,
                x_mm: s * 2_000 + t * 13,
                y_mm: if t % 40 < 5 { 150 } else { 0 },
                z_mm: lane * t * 5,
                yaw_ddeg: i16::try_from((t * 7 + s * 90) % 3_600 - 1_800).expect("yaw fits"),
                hp: u16::try_from(1_000 - t / 3 - s * 10).expect("hp fits"),
                stamina: if slot % 2 == 0 {
                    100
                } else {
                    u16::try_from(100 - t % 50).expect("stamina fits")
                },
            }
        })
        .collect();
    FullSnap::new(tick, fighters).expect("scripted snapshots are valid")
}

/// The state digest both ends fold per tick.
fn snap_digest(snap: &FullSnap) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(snap.tick);
    for fighter in &snap.fighters {
        hasher.write_u8(fighter.slot);
        hasher.write_i32(fighter.x_mm);
        hasher.write_i32(fighter.y_mm);
        hasher.write_i32(fighter.z_mm);
        hasher.write(&fighter.yaw_ddeg.to_le_bytes());
        hasher.write_u16(fighter.hp);
        hasher.write_u16(fighter.stamina);
    }
    hasher.finish()
}

/// The input the client sends back after it applied `tick`.
fn scripted_input(tick: u32) -> BodyInput {
    BodyInput {
        tick,
        fighter: BodyId::new(0).expect("slot 0"),
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

/// What each end saw of the duel; the two sides must agree field by field.
#[derive(PartialEq, Eq, Debug)]
struct Seen {
    /// Fold of the per-tick state digests.
    state: u64,
    /// Digest of every state datagram, server to client.
    states_on_wire: u64,
    /// Digest of every input datagram, client to server.
    inputs_on_wire: u64,
    /// Every reliable frame the server sent, in order.
    reliable: Vec<(ControlKind, Vec<u8>)>,
}

/// The client side runs on its own runtime, like a separate process would.
fn client_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("client runtime")
}

fn block_on<F: Future>(future: F) -> F::Output {
    client_runtime().block_on(future)
}

async fn within<F: Future>(what: &str, future: F) -> F::Output {
    match tokio::time::timeout(WAIT, future).await {
        Ok(output) => output,
        Err(_) => panic!("no answer inside the wait: {what}"),
    }
}

/// A native `wtransport` client pinned to `hash`, as a browser's
/// `serverCertificateHashes` pins.
async fn connect_to(
    server: SocketAddr,
    path: &str,
    hash: [u8; 32],
) -> Result<(Endpoint<endpoint_side::Client>, Connection), ConnectingError> {
    let config = ClientConfig::builder()
        .with_bind_address(loopback())
        .with_server_certificate_hashes([Sha256Digest::new(hash)])
        .build();
    let endpoint = Endpoint::client(config).expect("client endpoint");
    let connection = endpoint
        .connect(format!("https://127.0.0.1:{}{path}", server.port()))
        .await?;
    Ok((endpoint, connection))
}

/// Close the session and let the close frame leave before the runtime dies.
async fn hang_up(endpoint: &Endpoint<endpoint_side::Client>, connection: &Connection) {
    connection.close(VarInt::from_u32(0), b"done");
    endpoint.wait_idle().await;
}

async fn read_frame(what: &str, recv: &mut RecvStream) -> (ControlKind, Vec<u8>) {
    let mut header = [0u8; FRAME_HEADER_LEN];
    within(what, recv.read_exact(&mut header))
        .await
        .expect("frame header");
    let (kind, length) = decode_header(header).expect("the server obeys the framing");
    let mut payload = vec![0u8; length];
    within(what, recv.read_exact(&mut payload))
        .await
        .expect("frame payload");
    (kind, payload)
}

/// The next frame the server's tick loop would see.
fn next_frame(what: &str, server: &mut WtServer, backlog: &mut VecDeque<Inbound>) -> Inbound {
    for _ in 0..POLL_ROUNDS {
        if let Some(frame) = backlog.pop_front() {
            return frame;
        }
        backlog.extend(server.recv());
        if backlog.is_empty() {
            thread::sleep(POLL_STEP);
        }
    }
    panic!(
        "no frame reached the server inside the wait: {what} (peers {:?}, dropped datagrams {}, departures {:?})",
        server.peers(),
        server.dropped_datagrams(),
        server.take_departures()
    );
}

fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..POLL_ROUNDS {
        if ready() {
            return;
        }
        thread::sleep(POLL_STEP);
    }
    panic!("{what}: still false after the wait");
}

/// The client half of the duel: join over the stream, rebuild every state
/// from the datagram deltas, answer each tick with an input datagram.
async fn play(server: SocketAddr, hash: [u8; 32]) -> Seen {
    let (endpoint, connection) = connect_to(server, SESSION_PATH, hash)
        .await
        .expect("a client pinned to the right hash connects");
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .expect("open the stream")
        .await
        .expect("the stream opens");
    let hello = Msg::JoinRequest {
        token: TOKEN,
        build: BUILD,
    }
    .encode()
    .expect("hello encodes");
    send.write_all(&encode_message(ControlKind::Lobby, &hello).expect("hello fits"))
        .await
        .expect("hello is written");

    let grant = read_frame("the grant", &mut recv).await;
    let first_state = read_frame("the baseline snapshot", &mut recv).await;
    let mut reliable = vec![grant, first_state];
    assert_eq!(reliable[0].0, ControlKind::Lobby, "the grant comes first");
    let (kind, baseline_bytes) = &reliable[1];
    assert_eq!(*kind, ControlKind::Snapshot, "then the baseline");
    let mut baseline = decode_snapshot(baseline_bytes).expect("baseline decodes");
    assert_eq!(baseline.tick, 0);

    let (mut state, mut states_on_wire, mut inputs_on_wire) =
        (Fnv64::new(), Fnv64::new(), Fnv64::new());
    for tick in 1..=TICKS {
        let datagram = within(
            &format!("the state datagram of tick {tick}"),
            connection.receive_datagram(),
        )
        .await
        .expect("state datagram");
        let bytes = datagram.payload();
        states_on_wire.write(&bytes);
        let packet = decode_state(&bytes).expect("state codec");
        let delta = packet.delta.expect("scripted ticks are deltas");
        let rebuilt = decode_delta(&delta, &baseline).expect("delta applies to the baseline");
        assert_eq!(rebuilt.tick, tick);
        assert_eq!(
            packet.checkpoint,
            Checkpoint {
                tick,
                hash: snap_digest(&rebuilt)
            },
            "the rebuilt state matches the server checkpoint"
        );
        state.write_u64(snap_digest(&rebuilt));
        if tick == KEYFRAME_TICK {
            let frame = read_frame("the keyframe", &mut recv).await;
            assert_eq!(
                frame.1,
                encode_snapshot(&rebuilt).expect("encodes"),
                "the keyframe equals the state rebuilt from deltas"
            );
            reliable.push(frame);
        }
        baseline = rebuilt;
        let answer = encode_input(tick, &[scripted_input(tick)]).expect("input encodes");
        inputs_on_wire.write(&answer);
        connection
            .send_datagram(&answer)
            .expect("input datagram fits");
    }
    // The server's last event is the barrier: it is sent after the final
    // answer arrived, so hanging up before it would race that answer.
    reliable.push(read_frame("the closing event", &mut recv).await);
    hang_up(&endpoint, &connection).await;
    Seen {
        state: state.finish(),
        states_on_wire: states_on_wire.finish(),
        inputs_on_wire: inputs_on_wire.finish(),
        reliable,
    }
}

/// The server half: the tick loop talks to the trait only.
fn serve_duel(server: &mut WtServer) -> (PeerId, Seen) {
    let mut backlog = VecDeque::new();
    let hello = next_frame("the hello", server, &mut backlog);
    let peer = hello.from;
    let InboundFrame::Stream { kind, bytes } = hello.frame else {
        panic!("the first frame is the hello on the stream");
    };
    assert_eq!(kind, ControlKind::Lobby);
    assert_eq!(
        Msg::decode(&bytes),
        Ok(Msg::JoinRequest {
            token: TOKEN,
            build: BUILD
        })
    );

    let grant = Msg::JoinGrant {
        token: TOKEN,
        slot: 0,
        fighters: FIGHTERS,
        build: BUILD,
    };
    send_lobby(server, peer, grant).expect("grant is queued");
    let first = snap_at(0);
    send_snapshot(server, peer, &first).expect("baseline is queued");
    let mut reliable = vec![
        (ControlKind::Lobby, grant.encode().expect("grant encodes")),
        (
            ControlKind::Snapshot,
            encode_snapshot(&first).expect("baseline encodes"),
        ),
    ];

    let (mut state, mut states_on_wire, mut inputs_on_wire) =
        (Fnv64::new(), Fnv64::new(), Fnv64::new());
    for tick in 1..=TICKS {
        let (previous, current) = (snap_at(tick - 1), snap_at(tick));
        let delta = encode_delta(&current, &previous).expect("scripted ticks diff");
        let checkpoint = Checkpoint {
            tick,
            hash: snap_digest(&current),
        };
        send_state_delta(server, peer, checkpoint, &delta).expect("delta datagram fits");
        states_on_wire.write(&encode_state_delta(checkpoint, &delta).expect("encodes"));
        state.write_u64(checkpoint.hash);
        if tick == KEYFRAME_TICK {
            send_snapshot(server, peer, &current).expect("keyframe is queued");
            reliable.push((
                ControlKind::Snapshot,
                encode_snapshot(&current).expect("keyframe encodes"),
            ));
        }
        // Lockstep: the client answers each tick before the next is sent, so
        // the unreliable lane sees no loss to heal in this proof.
        let answer = next_frame(&format!("the answer to tick {tick}"), server, &mut backlog);
        assert_eq!(answer.from, peer);
        let InboundFrame::Datagram(bytes) = answer.frame else {
            panic!("inputs ride datagrams");
        };
        inputs_on_wire.write(&bytes);
        let packet = decode_input(&bytes).expect("input codec");
        assert_eq!(
            (packet.tick, packet.inputs),
            (tick, vec![scripted_input(tick)])
        );
    }
    let closing = Msg::Ready {
        token: TOKEN,
        ready: true,
    };
    send_lobby(server, peer, closing).expect("closing event is queued");
    reliable.push((
        ControlKind::Lobby,
        closing.encode().expect("closing event encodes"),
    ));
    (
        peer,
        Seen {
            state: state.finish(),
            states_on_wire: states_on_wire.finish(),
            inputs_on_wire: inputs_on_wire.finish(),
            reliable,
        },
    )
}

#[test]
fn webtransport_plays_a_600_tick_snapshot_duel_with_a_pinned_client() {
    let cert = mint();
    let mut server = WtServer::bind(loopback(), &cert).expect("bind the server");
    let (addr, hash) = (server.local_addr(), cert.hash());
    let client = thread::spawn(move || block_on(play(addr, hash)));

    let (peer, served) = serve_duel(&mut server);
    let played = client.join().expect("the client finished the duel");

    assert_eq!(served, played, "both ends saw the same bytes and states");
    assert_eq!(
        served.reliable.len(),
        4,
        "grant, baseline, keyframe and closing event"
    );
    assert_eq!(server.dropped_datagrams(), 0);
    let mut departures = Vec::new();
    wait_for("the client's hang-up is reported", || {
        departures.extend(server.take_departures());
        !departures.is_empty()
    });
    assert_eq!(
        departures,
        vec![Departure {
            peer,
            reason: DepartReason::PeerClosed
        }]
    );
    assert!(server.peers().is_empty());
    assert_eq!(
        server.send_unreliable(peer, b"gone"),
        Err(NetError::BadInput),
        "a departed peer is refused loudly"
    );
}

#[test]
fn webtransport_refuses_a_client_pinned_to_the_wrong_hash() {
    let served = mint();
    let impostor = mint();
    assert_ne!(served.hash(), impostor.hash());
    let mut server = WtServer::bind(loopback(), &served).expect("bind the server");
    let addr = server.local_addr();

    let wrong = block_on(connect_to(addr, SESSION_PATH, impostor.hash()));
    let error = wrong.err().expect("a wrong pin must not connect");
    assert!(
        matches!(error, ConnectingError::ConnectionError(_)),
        "the handshake fails at the connection, not at the URL: {error}"
    );
    assert!(!error.to_string().is_empty(), "the failure names a reason");
    assert!(server.peers().is_empty(), "no session was seated");
    assert!(server.take_departures().is_empty());
    assert!(server.recv().is_empty());

    // The same path with the right pin connects and is seated.
    let runtime = client_runtime();
    let (endpoint, connection) = runtime
        .block_on(connect_to(addr, SESSION_PATH, served.hash()))
        .expect("the right pin connects");
    wait_for_peer_count(&server, 1);
    runtime.block_on(hang_up(&endpoint, &connection));
}

fn wait_for_peer_count(server: &WtServer, count: usize) {
    wait_for("the session is seated", || server.peers().len() == count);
}

#[test]
fn webtransport_answers_the_wrong_path_with_a_rejection() {
    let cert = mint();
    let server = WtServer::bind(loopback(), &cert).expect("bind the server");
    let outcome = block_on(connect_to(server.local_addr(), "/other", cert.hash()));
    assert!(
        matches!(outcome, Err(ConnectingError::SessionRejected)),
        "only {SESSION_PATH} is served"
    );
    assert!(server.peers().is_empty());
}

#[test]
fn webtransport_ends_a_session_whose_stream_breaks_the_framing() {
    let cert = mint();
    let mut server = WtServer::bind(loopback(), &cert).expect("bind the server");
    let addr = server.local_addr();
    let oversize = u16::try_from(MAX_PACKET_BYTES + 1)
        .expect("the budget fits u16")
        .to_le_bytes();
    let cases = [
        ([9, 0, 0], NetError::BadKind(9)),
        ([1, oversize[0], oversize[1]], NetError::TooLarge),
    ];
    for (header, fault) in cases {
        let closed = block_on(async {
            let (endpoint, connection) = connect_to(addr, SESSION_PATH, cert.hash())
                .await
                .expect("connect");
            let (mut send, _recv) = connection
                .open_bi()
                .await
                .expect("open")
                .await
                .expect("stream");
            send.write_all(&header).await.expect("header is written");
            let closed = within("the server closing the session", connection.closed()).await;
            endpoint.wait_idle().await;
            closed
        });
        assert!(
            matches!(
                closed,
                wtransport::error::ConnectionError::ApplicationClosed(_)
            ),
            "the server closes the session with a code: {closed}"
        );
        let mut departures = Vec::new();
        wait_for("the violation is reported", || {
            departures.extend(server.take_departures());
            !departures.is_empty()
        });
        assert_eq!(departures.len(), 1);
        assert_eq!(departures[0].reason, DepartReason::BadFrame(fault));
        assert!(server.recv().is_empty(), "the bad frame never surfaced");
    }
}

#[test]
fn webtransport_refuses_reliable_frames_when_the_client_never_opens_its_stream() {
    let cert = mint();
    let mut server = WtServer::bind(loopback(), &cert).expect("bind the server");
    let runtime = client_runtime();
    let (endpoint, connection) = runtime
        .block_on(connect_to(server.local_addr(), SESSION_PATH, cert.hash()))
        .expect("connect");
    wait_for_peer_count(&server, 1);
    let peer = server.peers()[0];
    let payload = [0u8; 8];
    // The stream is not accepted yet, so nothing drains: the queue holds
    // exactly its cap and then refuses instead of growing.
    for queued in 0..RELIABLE_FRAMES {
        server
            .send_reliable(peer, ControlKind::Lobby, &payload)
            .unwrap_or_else(|error| panic!("frame {queued} fits the queue: {error}"));
    }
    assert_eq!(
        server.send_reliable(peer, ControlKind::Lobby, &payload),
        Err(NetError::BadInput)
    );
    // Datagrams are a separate lane and stay open.
    server
        .send_unreliable(peer, &payload)
        .expect("datagrams are unaffected");
    runtime.block_on(hang_up(&endpoint, &connection));
}

#[test]
fn webtransport_server_refuses_oversize_and_unknown_peers_before_the_network() {
    let cert = mint();
    let mut server = WtServer::bind(loopback(), &cert).expect("bind the server");
    let stranger = PeerId(SocketAddr::from(([127, 0, 0, 1], 9)));
    let big = vec![0u8; MAX_PACKET_BYTES + 1];
    assert_eq!(
        server.send_unreliable(stranger, &big),
        Err(NetError::TooLarge)
    );
    assert_eq!(
        server.send_reliable(stranger, ControlKind::Snapshot, &big),
        Err(NetError::TooLarge)
    );
    assert_eq!(
        server.send_unreliable(stranger, b"x"),
        Err(NetError::BadInput),
        "no session at that address"
    );
    assert_eq!(
        server.send_reliable(stranger, ControlKind::Lobby, b"x"),
        Err(NetError::BadInput)
    );
    assert!(server.recv().is_empty());
    assert!(server.peers().is_empty());
    assert_eq!(
        MAX_SESSIONS,
        usize::from(engine_input::MAX_PARTICIPANTS),
        "one session per participant slot"
    );
}
