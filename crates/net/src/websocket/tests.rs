//! In-process WebSocket proofs: a loopback client plays a scripted exchange
//! against [`WsServer`], and the bytes it receives equal the bytes the same
//! script produces over an in-memory transport pair and straight from the
//! codec.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use engine_arena::Checkpoint;
use engine_core::Fnv64;
use engine_input::fighter::{Buttons, ATTACK};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};

use super::wire::{self, Class, MAX_MESSAGE_BYTES};
use super::{DepartReason, Departure, WsServer, MAX_SESSIONS, SESSION_PATH};
use crate::delta::BodySnap;
use crate::lobby::{BuildId, Msg};
use crate::quic::{
    check_budget, send_lobby, send_snapshot, send_state_delta, ControlKind, Inbound, InboundFrame,
    PeerId, Transport,
};
use crate::session::{
    decode_delta, decode_input, decode_snapshot, decode_state, encode_delta, encode_input,
    encode_snapshot, encode_state_delta, BodyId, BodyInput, FullSnap, NetError, MAX_PACKET_BYTES,
};

/// Ticks of the scripted exchange.
const TICKS: u32 = 120;
/// Fighters in every scripted snapshot.
const FIGHTERS: u8 = 8;
/// The tick whose full snapshot also rides the reliable lane.
const KEYFRAME_TICK: u32 = 60;
/// Polling interval of the synchronous sides.
const POLL_STEP: Duration = Duration::from_millis(1);
/// Polling rounds that add up to ten seconds: a stall fails loudly, named.
const POLL_ROUNDS: u32 = 10_000;

const BUILD: BuildId = BuildId {
    version: 1,
    rules: 0x1234_5678,
};
const TOKEN: u64 = 7;

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
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

/// The checkpoint-and-delta datagram the server sends for `tick`, straight
/// from the codec: the reference every transport must deliver unchanged.
fn state_datagram(tick: u32) -> Vec<u8> {
    let (previous, current) = (snap_at(tick - 1), snap_at(tick));
    let delta = encode_delta(&current, &previous).expect("scripted ticks diff");
    let checkpoint = Checkpoint {
        tick,
        hash: snap_digest(&current),
    };
    encode_state_delta(checkpoint, &delta).expect("encodes")
}

/// What each end saw of the exchange; the two sides must agree field by field.
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

/// Frames pulled off a transport, split by lane.
#[derive(Default)]
struct Mailbox {
    peer: Option<PeerId>,
    datagrams: VecDeque<Vec<u8>>,
    streams: VecDeque<(ControlKind, Vec<u8>)>,
}

impl Mailbox {
    fn pull<T: Transport>(&mut self, transport: &mut T) {
        for Inbound { from, frame } in transport.recv() {
            self.peer = Some(from);
            match frame {
                InboundFrame::Datagram(bytes) => self.datagrams.push_back(bytes),
                InboundFrame::Stream { kind, bytes } => self.streams.push_back((kind, bytes)),
            }
        }
    }

    fn datagram<T: Transport>(&mut self, what: &str, transport: &mut T) -> Vec<u8> {
        for _ in 0..POLL_ROUNDS {
            self.pull(transport);
            if let Some(bytes) = self.datagrams.pop_front() {
                return bytes;
            }
            thread::sleep(POLL_STEP);
        }
        panic!("no datagram arrived inside the wait: {what}");
    }

    fn stream<T: Transport>(&mut self, what: &str, transport: &mut T) -> (ControlKind, Vec<u8>) {
        for _ in 0..POLL_ROUNDS {
            self.pull(transport);
            if let Some(frame) = self.streams.pop_front() {
                return frame;
            }
            thread::sleep(POLL_STEP);
        }
        panic!("no reliable frame arrived inside the wait: {what}");
    }
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

/// The client half of the exchange, over any transport: join over the
/// reliable lane, rebuild every state from the datagram deltas, answer each
/// tick with an input datagram.
fn play_exchange<T: Transport>(client: &mut T, server: PeerId) -> Seen {
    let mut mail = Mailbox::default();
    let hello = Msg::JoinRequest {
        token: TOKEN,
        build: BUILD,
    };
    send_lobby(client, server, hello).expect("hello is queued");
    let grant = mail.stream("the grant", client);
    let first_state = mail.stream("the baseline snapshot", client);
    let mut reliable = vec![grant, first_state];
    assert_eq!(reliable[0].0, ControlKind::Lobby, "the grant comes first");
    assert_eq!(reliable[1].0, ControlKind::Snapshot, "then the baseline");
    let mut baseline = decode_snapshot(&reliable[1].1).expect("baseline decodes");
    assert_eq!(baseline.tick, 0);

    let (mut state, mut states_on_wire, mut inputs_on_wire) =
        (Fnv64::new(), Fnv64::new(), Fnv64::new());
    for tick in 1..=TICKS {
        let bytes = mail.datagram(&format!("the state datagram of tick {tick}"), client);
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
            let frame = mail.stream("the keyframe", client);
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
        client
            .send_unreliable(server, &answer)
            .expect("input datagram fits");
    }
    // The server's last event is the barrier: it is sent after the final
    // answer arrived, so hanging up before it would race that answer.
    reliable.push(mail.stream("the closing event", client));
    Seen {
        state: state.finish(),
        states_on_wire: states_on_wire.finish(),
        inputs_on_wire: inputs_on_wire.finish(),
        reliable,
    }
}

/// The server half, over any transport: the tick loop talks to the trait only.
fn serve_exchange<T: Transport>(server: &mut T) -> (PeerId, Seen) {
    let mut mail = Mailbox::default();
    let (kind, bytes) = mail.stream("the hello", server);
    let peer = mail.peer.expect("a frame names its sender");
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
        states_on_wire.write(&state_datagram(tick));
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
        let bytes = mail.datagram(&format!("the answer to tick {tick}"), server);
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

type Queue = Arc<Mutex<VecDeque<Inbound>>>;

fn locked(queue: &Queue) -> std::sync::MutexGuard<'_, VecDeque<Inbound>> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One end of a lossless in-memory link: the reference path the WebSocket
/// bytes are compared against.
struct MemoryEnd {
    me: PeerId,
    peer: PeerId,
    inbox: Queue,
    outbox: Queue,
}

fn memory_pair() -> (MemoryEnd, MemoryEnd) {
    let (server, client) = (
        PeerId(SocketAddr::from(([127, 0, 0, 1], 40_001))),
        PeerId(SocketAddr::from(([127, 0, 0, 1], 40_002))),
    );
    let (to_server, to_client): (Queue, Queue) = Default::default();
    (
        MemoryEnd {
            me: server,
            peer: client,
            inbox: Arc::clone(&to_server),
            outbox: Arc::clone(&to_client),
        },
        MemoryEnd {
            me: client,
            peer: server,
            inbox: to_client,
            outbox: to_server,
        },
    )
}

impl Transport for MemoryEnd {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        if to != self.peer {
            return Err(NetError::BadInput);
        }
        locked(&self.outbox).push_back(Inbound {
            from: self.me,
            frame: InboundFrame::Datagram(bytes.to_vec()),
        });
        Ok(())
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        check_budget(bytes)?;
        if to != self.peer {
            return Err(NetError::BadInput);
        }
        locked(&self.outbox).push_back(Inbound {
            from: self.me,
            frame: InboundFrame::Stream {
                kind,
                bytes: bytes.to_vec(),
            },
        });
        Ok(())
    }

    fn recv(&mut self) -> Vec<Inbound> {
        locked(&self.inbox).drain(..).collect()
    }
}

/// A loopback WebSocket client as a [`Transport`]: it runs on its own thread
/// and runtime, like a browser would, and speaks the wire layout.
struct WsTestClient {
    server: PeerId,
    outgoing: Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
    inbox: Queue,
    worker: Option<JoinHandle<()>>,
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("client runtime")
}

/// Upgrade a TCP connection to `addr` at `path`.
async fn upgrade(
    addr: SocketAddr,
    path: &str,
) -> Result<WebSocketStream<TcpStream>, tokio_tungstenite::tungstenite::Error> {
    let tcp = TcpStream::connect(addr).await.expect("tcp connects");
    tcp.set_nodelay(true).expect("nodelay");
    let (socket, _response) = client_async(format!("ws://{addr}{path}"), tcp).await?;
    Ok(socket)
}

impl WsTestClient {
    fn connect(addr: SocketAddr) -> Self {
        let inbox = Queue::default();
        let (outgoing, mut queued) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (ready_tx, ready_rx) = std_mpsc::channel();
        let server = PeerId(addr);
        let worker = thread::spawn({
            let inbox = Arc::clone(&inbox);
            move || {
                runtime().block_on(async move {
                    let socket = match upgrade(addr, SESSION_PATH).await {
                        Ok(socket) => socket,
                        Err(error) => {
                            ready_tx.send(Err(error.to_string())).expect("ready");
                            return;
                        }
                    };
                    ready_tx.send(Ok(())).expect("ready");
                    let (mut sink, mut source) = socket.split();
                    loop {
                        tokio::select! {
                            message = queued.recv() => match message {
                                Some(bytes) => sink.send(Message::Binary(bytes.into())).await.expect("send"),
                                None => break,
                            },
                            incoming = source.next() => match incoming {
                                Some(Ok(Message::Binary(bytes))) => {
                                    let (class, payload) = wire::decode(&bytes).expect("the server obeys the wire");
                                    let frame = match class {
                                        Class::Datagram => InboundFrame::Datagram(payload.to_vec()),
                                        Class::Reliable(kind) => InboundFrame::Stream { kind, bytes: payload.to_vec() },
                                    };
                                    locked(&inbox).push_back(Inbound { from: server, frame });
                                }
                                Some(Ok(_)) => {}
                                Some(Err(_)) | None => break,
                            },
                        }
                    }
                    // Say goodbye and let the close frame leave.
                    let _ = sink.send(Message::Close(None)).await;
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        while source.next().await.is_some() {}
                    })
                    .await;
                });
            }
        });
        ready_rx
            .recv()
            .expect("the client thread reports")
            .expect("the upgrade succeeds");
        Self {
            server,
            outgoing: Some(outgoing),
            inbox,
            worker: Some(worker),
        }
    }

    fn queue(&self, to: PeerId, message: Vec<u8>) -> Result<(), NetError> {
        if to != self.server {
            return Err(NetError::BadInput);
        }
        self.outgoing
            .as_ref()
            .ok_or(NetError::BadInput)?
            .send(message)
            .map_err(|_| NetError::BadInput)
    }
}

impl Transport for WsTestClient {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        self.queue(to, wire::encode(Class::Datagram, bytes)?)
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        self.queue(to, wire::encode(Class::Reliable(kind), bytes)?)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        locked(&self.inbox).drain(..).collect()
    }
}

impl Drop for WsTestClient {
    fn drop(&mut self) {
        self.outgoing = None;
        if let Some(worker) = self.worker.take() {
            worker.join().expect("the client thread ends cleanly");
        }
    }
}

/// Run the exchange over the WebSocket server; returns what each end saw and
/// the departures the server recorded for the client's hang-up.
fn over_websocket() -> (Seen, Seen, Vec<Departure>, PeerId) {
    let mut server = WsServer::bind(loopback()).expect("bind the server");
    let addr = server.local_addr();
    let client = thread::spawn(move || {
        let mut client = WsTestClient::connect(addr);
        let server_peer = client.server;
        play_exchange(&mut client, server_peer)
    });
    let (peer, served) = serve_exchange(&mut server);
    let played = client.join().expect("the client finished the exchange");
    assert_eq!(server.dropped_datagrams(), 0);
    let mut departures = Vec::new();
    wait_for("the client's hang-up is reported", || {
        departures.extend(server.take_departures());
        !departures.is_empty()
    });
    assert!(server.peers().is_empty());
    assert_eq!(
        server.send_unreliable(peer, b"gone"),
        Err(NetError::BadInput),
        "a departed peer is refused"
    );
    (served, played, departures, peer)
}

/// Run the same exchange over the in-memory pair.
fn over_memory() -> (Seen, Seen) {
    let (mut server, mut client) = memory_pair();
    let server_peer = server.me;
    let player = thread::spawn(move || play_exchange(&mut client, server_peer));
    let (_, served) = serve_exchange(&mut server);
    (served, player.join().expect("the memory client finished"))
}

#[test]
fn ws_transport_delivers_the_same_bytes_as_the_in_memory_path() {
    let (served, played, departures, peer) = over_websocket();
    let (memory_served, memory_played) = over_memory();

    assert_eq!(
        served, played,
        "both ends of the socket saw the same states"
    );
    assert_eq!(
        memory_served, memory_played,
        "both ends of the memory link saw the same states"
    );
    assert_eq!(
        served, memory_served,
        "the WebSocket carried exactly the bytes and digests of the in-memory path"
    );
    assert_eq!(
        played.states_on_wire,
        {
            let mut hasher = Fnv64::new();
            for tick in 1..=TICKS {
                hasher.write(&state_datagram(tick));
            }
            hasher.finish()
        },
        "the state datagrams equal what the codec encodes"
    );
    assert_eq!(
        served.reliable.len(),
        4,
        "grant, baseline, keyframe and closing event"
    );
    assert_eq!(
        departures,
        vec![Departure {
            peer,
            reason: DepartReason::PeerClosed
        }]
    );
}

/// Bind a server and the address its sessions reach it at.
fn server() -> (WsServer, SocketAddr) {
    let server = WsServer::bind(loopback()).expect("bind the server");
    let addr = server.local_addr();
    (server, addr)
}

#[test]
fn ws_transport_answers_404_to_the_wrong_path() {
    let (mut server, addr) = server();
    let outcome = runtime().block_on(upgrade(addr, "/other"));
    let error = outcome.expect_err("the upgrade is refused");
    assert!(
        error.to_string().contains("404"),
        "the refusal names the status: {error}"
    );
    assert!(server.peers().is_empty());
    assert!(server.take_departures().is_empty(), "no session ever began");
}

#[test]
fn ws_transport_answers_429_beyond_the_session_cap() {
    let (server, addr) = server();
    let runtime = runtime();
    let mut held = Vec::new();
    for _ in 0..MAX_SESSIONS {
        held.push(
            runtime
                .block_on(upgrade(addr, SESSION_PATH))
                .expect("a seat is free"),
        );
    }
    wait_for("all sessions are seated", || {
        server.peers().len() == MAX_SESSIONS
    });
    assert_eq!(
        held.len(),
        MAX_SESSIONS,
        "every client still holds its seat"
    );
    let error = runtime
        .block_on(upgrade(addr, SESSION_PATH))
        .expect_err("the table is full");
    assert!(error.to_string().contains("429"), "{error}");
}

/// Send `message` as the first thing on a fresh session and return why the
/// server ended it.
fn departure_after(message: Message) -> DepartReason {
    let (mut server, addr) = server();
    let runtime = runtime();
    let mut socket = runtime
        .block_on(upgrade(addr, SESSION_PATH))
        .expect("upgrade");
    runtime
        .block_on(socket.send(message))
        .expect("the message leaves");
    let mut departures = Vec::new();
    wait_for("the bad message ends the session", || {
        departures.extend(server.take_departures());
        !departures.is_empty()
    });
    // The server closes with a protocol code before it lets go.
    let closing = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(frame) = socket.next().await {
                if let Ok(Message::Close(frame)) = frame {
                    return frame;
                }
            }
            None
        })
        .await
    });
    let frame = closing
        .expect("the server answers inside the wait")
        .expect("with a close frame");
    assert_eq!(frame.code, CloseCode::Protocol);
    departures[0].reason
}

#[test]
fn ws_transport_ends_a_session_on_a_message_the_wire_refuses() {
    assert_eq!(
        departure_after(Message::Text("hello".into())),
        DepartReason::BadFrame(NetError::BadKind(b'T')),
        "text has no meaning here"
    );
    assert_eq!(
        departure_after(Message::Binary(vec![9, 1, 2].into())),
        DepartReason::BadFrame(NetError::BadKind(9)),
        "an unknown class"
    );
    assert_eq!(
        departure_after(Message::Binary(Vec::new().into())),
        DepartReason::BadFrame(NetError::Truncated),
        "an empty message"
    );
    assert_eq!(
        departure_after(Message::Binary(vec![0; MAX_MESSAGE_BYTES + 1].into())),
        DepartReason::BadFrame(NetError::TooLarge),
        "a payload over the budget"
    );
}

#[test]
fn ws_transport_refuses_bad_sends_before_they_touch_the_socket() {
    let (mut server, addr) = server();
    let _client = WsTestClient::connect(addr);
    wait_for("the session is seated", || !server.peers().is_empty());
    let peer = server.peers()[0];
    let big = vec![0u8; MAX_PACKET_BYTES + 1];
    assert_eq!(server.send_unreliable(peer, &big), Err(NetError::TooLarge));
    assert_eq!(
        server.send_reliable(peer, ControlKind::Snapshot, &big),
        Err(NetError::TooLarge)
    );
    let stranger = PeerId(SocketAddr::from(([127, 0, 0, 1], 9)));
    assert_eq!(
        server.send_unreliable(stranger, b"x"),
        Err(NetError::BadInput)
    );
    assert_eq!(
        server.send_reliable(stranger, ControlKind::Lobby, b"x"),
        Err(NetError::BadInput)
    );
    assert!(server.send_unreliable(peer, &[1; MAX_PACKET_BYTES]).is_ok());
}

#[test]
fn ws_transport_says_goodbye_to_open_sessions_on_shutdown() {
    let (server, addr) = server();
    let runtime = runtime();
    let mut socket = runtime
        .block_on(upgrade(addr, SESSION_PATH))
        .expect("upgrade");
    wait_for("the session is seated", || !server.peers().is_empty());
    drop(server);
    let frame = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(message) = socket.next().await {
                if let Ok(Message::Close(frame)) = message {
                    return frame;
                }
            }
            None
        })
        .await
    });
    let frame = frame
        .expect("the goodbye arrives inside the wait")
        .expect("with a close frame");
    assert_eq!(frame.code, CloseCode::Away);
}

/// WEB9 + WEB12: the whole scripted duel over real sockets. The host is the
/// `net_serve` state machine on [`WsServer`], the client is the page's
/// [`DuelClient`] on a WebSocket; the hash they end on is the one the offline
/// reference computes, and the departure of the client is reported.
#[test]
fn ws_duel_plays_600_ticks_over_a_socket_and_ends_on_the_reference_hash() {
    use crate::webduel::{
        duel_spec, net_line, reference_hash, DuelClient, DuelHost, DUEL_TICKS,
    };

    let mut server = WsServer::bind(loopback()).expect("bind the server");
    let addr = server.local_addr();
    let mut client_end = WsTestClient::connect(addr);
    let mut host = DuelHost::new(duel_spec().expect("spec")).expect("host");
    let mut client = DuelClient::new(0xD0E1).expect("client");
    let at = PeerId(addr);
    let mut rounds = 0u32;
    while !client.poll(&mut client_end, at) {
        host.pump(&mut [&mut server]);
        host.tick(&mut [&mut server], false).expect("tick");
        rounds += 1;
        assert!(rounds < POLL_ROUNDS * 4, "the duel stalled at tick {}", host.tick_now());
        thread::sleep(Duration::from_micros(200));
    }
    let outcome = client.outcome().expect("ended");
    let reference = reference_hash(0).expect("reference");
    let done = outcome.as_ref().expect("the duel converged");
    assert_eq!((done.ticks, done.hash), (DUEL_TICKS, reference));
    assert_eq!(host.final_hash(), Some(reference));
    assert_eq!(
        net_line("websocket", outcome),
        format!("net: transport=websocket ticks=600 hash={reference:016x} peers=1")
    );

    drop(client_end);
    let mut departures = Vec::new();
    wait_for("the client's hang-up is reported", || {
        departures.extend(server.take_departures());
        !departures.is_empty()
    });
    assert_eq!(departures.len(), 1);
}
