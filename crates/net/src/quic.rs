//! quic: QUIC transport behind the Transport trait.
//!
//! Purpose: carry the session wire over QUIC datagrams (inputs, states,
//! deltas) and QUIC streams (snapshots, lobby) while the UDP impl stays.
//! Owner: net lane. Units: ticks at 60 Hz, bytes on the wire.
//! Determinism: SimQuicNet is seeded and tick-ordered; real sockets and the
//! std QUIC runtime never enter sim hashes. Errors: session NetError for
//! wire faults, std io for socket setup, rustls for crypto setup.

use std::collections::BTreeMap;
#[cfg(feature = "quic")]
use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};

use crate::courier::RelayLeg;
use crate::lobby::{self, LobbyError, Msg};
use crate::netcode::Channel;
use crate::relay::{
    decode_binding_response, encode_binding_request, RelayError, RelayMsg, MAX_RELAY_BYTES,
};
use crate::session::{self, Delta, FullSnap, NetError, MAX_PACKET_BYTES};
use engine_arena::Checkpoint;
use engine_core::mix64;
use engine_input::FighterInput;

/// Peer handle: the socket address of the remote end. Client-server QUIC
/// connections map one-to-one onto addresses, exactly like UDP peers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PeerId(pub SocketAddr);

/// Reliable-stream payload class. Inputs, states and deltas ride
/// unreliable datagrams; full snapshots (the resync primitive) and lobby
/// bytes ride ordered streams.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ControlKind {
    /// Full fighter snapshot for resync and interpolation.
    Snapshot,
    /// Lobby handshake bytes (the lobby message wire).
    Lobby,
}

/// Delivery promise of one channel: what the sender guarantees, not how
/// the link carries it. Resend timing lives in the transport; no clock
/// enters this table, so sim hashes never see it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChannelDelivery {
    /// May drop or reorder; inputs and states ride here.
    Unreliable,
    /// Arrives once, in send order; snapshots ride here.
    ReliableOrdered,
    /// Ordered by the caller, not the link: sequenced frames ride
    /// datagrams and the receiver drops stale ones, so the link stays
    /// lossy while the netcode layer keeps the order.
    ReliableUnordered,
}

/// One channel row: id plus memory cap plus promise plus send priority.
/// PORT(cBournhonesque/lightyear@03797122da484969a38cfd64d3e2f99ccea6d90e:crates/replication/replication/src/channels.rs)
/// shape only, std-only, integer priorities: reliable prompts outrank lossy
/// mutations, and lossy rows never requeue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ChannelConfig {
    /// Channel id, unique in the table.
    pub channel_id: u8,
    /// Bytes a channel may hold before the sender must stop.
    pub max_memory_usage_bytes: usize,
    /// Delivery promise of this channel.
    pub delivery: ChannelDelivery,
    /// Send priority, unitless weight: higher sends first and sheds last.
    pub priority: u8,
    /// True when the encoder may requeue an unsent frame; false drops it
    /// instead, so stale mutations never build a backlog.
    pub retry_unsent: bool,
}

/// Memory cap per channel: 64 KiB holds about 53 full frames.
pub const CHANNEL_MEMORY_BYTES: usize = 64 * 1024;

/// Table: unreliable 0, sequenced 1, reliable 2. Reliable prompts carry
/// priority 10 with a backlog; lossy mutations sit at 1 with no requeue.
pub const CHANNEL_TABLE: [ChannelConfig; 3] = [
    ChannelConfig {
        channel_id: 0,
        max_memory_usage_bytes: CHANNEL_MEMORY_BYTES,
        delivery: ChannelDelivery::Unreliable,
        priority: 1,
        retry_unsent: false,
    },
    ChannelConfig {
        channel_id: 1,
        max_memory_usage_bytes: CHANNEL_MEMORY_BYTES,
        delivery: ChannelDelivery::ReliableUnordered,
        priority: 1,
        retry_unsent: false,
    },
    ChannelConfig {
        channel_id: 2,
        max_memory_usage_bytes: CHANNEL_MEMORY_BYTES,
        delivery: ChannelDelivery::ReliableOrdered,
        priority: 10,
        retry_unsent: true,
    },
];

/// Table row for a channel hint.
pub fn channel_config(ch: Channel) -> ChannelConfig {
    match ch {
        Channel::Unreliable => CHANNEL_TABLE[0],
        Channel::Sequenced => CHANNEL_TABLE[1],
        Channel::Reliable => CHANNEL_TABLE[2],
    }
}

/// Channel id of a hint.
pub fn channel_id(ch: Channel) -> u8 {
    channel_config(ch).channel_id
}

/// Send priority of a channel hint: combat-critical Reliable (10) outranks
/// lossy Sequenced/Unreliable (1). Pure table lookup, so the encoder sheds
/// stale mutations first with no clock and no sim-hash input.
pub fn channel_priority(ch: Channel) -> u8 {
    channel_config(ch).priority
}

/// True when the channel rides a reliable stream: only ordered snapshots
/// need stream ordering today. Sequenced frames promise caller-side order
/// but ride datagrams, so they answer false here with Unreliable.
pub fn channel_is_reliable(ch: Channel) -> bool {
    matches!(
        channel_config(ch).delivery,
        ChannelDelivery::ReliableOrdered
    )
}

/// Send bytes on the path the channel promises: the reliable channel rides
/// ordered streams with `kind`, the rest ride datagrams. Over-budget
/// frames fail before the network, exactly like the transport sends.
/// `kind` is ignored on the datagram path.
pub fn send_on_channel<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    ch: Channel,
    kind: ControlKind,
    bytes: &[u8],
) -> Result<(), NetError> {
    // Read the whole row: routing stays promise-based, while the priority
    // plus `retry_unsent` flag on the same row tell the encoder which
    // backlog to shed first (stale mutations) and never to requeue it.
    let cfg = channel_config(ch);
    // The encoder sheds by `channel_priority` while this send routes by the
    // same row: pin the two to one row so a future table edit cannot split
    // the route from the shed order.
    debug_assert_eq!(channel_priority(ch), cfg.priority, "one row per channel");
    if matches!(cfg.delivery, ChannelDelivery::ReliableOrdered) {
        transport.send_reliable(to, kind, bytes)
    } else {
        transport.send_unreliable(to, bytes)
    }
}

/// One received frame with its sender.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Inbound {
    /// Sender of this frame.
    pub from: PeerId,
    /// Datagram bytes or one ordered stream message.
    pub frame: InboundFrame,
}

/// Received payload: unreliable datagram bytes or one reliable message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InboundFrame {
    /// Unreliable datagram bytes (inputs, states, deltas).
    Datagram(Vec<u8>),
    /// One ordered reliable message with its stream class.
    Stream {
        /// Stream class of this message.
        kind: ControlKind,
        /// Message bytes.
        bytes: Vec<u8>,
    },
}

/// Transport behind the netcode: the UDP impl stays, the quinn impl is
/// added. Sends are validated and bounded; receives drain all frames due.
pub trait Transport {
    /// Send unreliable bytes (inputs, states, deltas). Over budget fails
    /// before touching the network; a failed UDP send drops like loss and
    /// the input redundancy window recovers it.
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError>;
    /// Send one ordered reliable message (snapshots, lobby). Same budget.
    /// Over raw UDP this rides a datagram (documented downgrade); over
    /// QUIC it rides a stream.
    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError>;
    /// Drain every frame due for this end, oldest first.
    fn recv(&mut self) -> Vec<Inbound>;
}

/// Reject oversize frames before they touch any socket: 1200 B wins
/// everywhere, streams included; the transport never fragments.
pub(crate) fn check_budget(bytes: &[u8]) -> Result<(), NetError> {
    if bytes.len() > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    Ok(())
}

/// Translate a lobby codec fault into the net wire error with the same
/// shape so stream payloads report through one error type.
fn lobby_err(err: LobbyError) -> NetError {
    match err {
        LobbyError::BadInput
        | LobbyError::SlotFull
        | LobbyError::UnknownToken { .. }
        | LobbyError::VersionMismatch { .. }
        | LobbyError::NotReady
        | LobbyError::AlreadyStarted => NetError::BadInput,
        LobbyError::Truncated => NetError::Truncated,
        LobbyError::BadMagic => NetError::BadMagic,
        LobbyError::BadKind(kind) => NetError::BadKind(kind),
        LobbyError::BadVersion { got } => NetError::VersionMismatch { got },
    }
}

/// Send one input window as an unreliable datagram.
pub fn send_input<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    tick: u32,
    window: &[FighterInput],
) -> Result<(), NetError> {
    let bytes = session::encode_input(tick, window)?;
    send_on_channel(
        transport,
        to,
        Channel::Unreliable,
        ControlKind::Lobby,
        &bytes,
    )
}

/// Send one full state window as an unreliable datagram.
pub fn send_state<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    checkpoint: Checkpoint,
    window: &[FighterInput],
) -> Result<(), NetError> {
    let bytes = session::encode_state(checkpoint, window)?;
    send_on_channel(
        transport,
        to,
        Channel::Unreliable,
        ControlKind::Lobby,
        &bytes,
    )
}

/// Send one state delta as an unreliable datagram.
pub fn send_state_delta<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    checkpoint: Checkpoint,
    delta: &Delta,
) -> Result<(), NetError> {
    let bytes = session::encode_state_delta(checkpoint, delta)?;
    send_on_channel(
        transport,
        to,
        Channel::Unreliable,
        ControlKind::Lobby,
        &bytes,
    )
}

/// Send one full snapshot as a reliable Snapshot message.
pub fn send_snapshot<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    full: &FullSnap,
) -> Result<(), NetError> {
    let bytes = session::encode_snapshot(full)?;
    send_on_channel(
        transport,
        to,
        Channel::Reliable,
        ControlKind::Snapshot,
        &bytes,
    )
}

/// Send one lobby message as a reliable Lobby message.
pub fn send_lobby<T: Transport + ?Sized>(
    transport: &mut T,
    to: PeerId,
    msg: Msg,
) -> Result<(), NetError> {
    let bytes = msg.encode().map_err(lobby_err)?;
    send_on_channel(transport, to, Channel::Reliable, ControlKind::Lobby, &bytes)
}

/// UDP transport: the default path. Reliable control rides a datagram
/// here (documented downgrade); the QUIC impl upgrades it to a stream.
pub struct UdpTransport {
    sock: UdpSocket,
}

impl UdpTransport {
    /// Wrap an owned socket and move it to nonblocking receive.
    pub fn new(sock: UdpSocket) -> io::Result<Self> {
        sock.set_nonblocking(true)?;
        Ok(Self { sock })
    }

    /// Bind an ephemeral localhost socket for loopback sessions.
    pub fn bind_loopback() -> io::Result<Self> {
        Self::new(UdpSocket::bind("127.0.0.1:0")?)
    }

    /// Local address this end receives on.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }
}

impl Transport for UdpTransport {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.sock
            .send_to(bytes, to.0)
            .map(|_| ())
            .map_err(|_| NetError::BadInput)
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        _kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.sock
            .send_to(bytes, to.0)
            .map(|_| ())
            .map_err(|_| NetError::BadInput)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        let mut out = Vec::new();
        let mut buf = [0u8; MAX_PACKET_BYTES];
        // Bound one drain so a burst never starves the caller.
        for _ in 0..64 {
            match self.sock.recv_from(&mut buf) {
                Ok((n, from)) => out.push(Inbound {
                    from: PeerId(from),
                    frame: classify(&buf[..n]),
                }),
                Err(_) => break,
            }
        }
        out
    }
}

/// Classify a UDP datagram into the QUIC frame it would have ridden:
/// snapshots and lobby bytes map to reliable streams, everything else to
/// datagrams. Unknown bytes stay datagrams so codecs reject them loudly.
fn classify(bytes: &[u8]) -> InboundFrame {
    if bytes.len() >= 4 {
        if u16::from_le_bytes([bytes[0], bytes[1]]) == lobby::LOBBY_MAGIC {
            return InboundFrame::Stream {
                kind: ControlKind::Lobby,
                bytes: bytes.to_vec(),
            };
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) == session::WIRE_MAGIC
            && bytes[3] == session::KIND_SNAPSHOT
        {
            return InboundFrame::Stream {
                kind: ControlKind::Snapshot,
                bytes: bytes.to_vec(),
            };
        }
    }
    InboundFrame::Datagram(bytes.to_vec())
}

/// Translate a relay codec fault into the net wire error with the same
/// shape so relay forwards report through one error type.
fn relay_err(err: RelayError) -> NetError {
    match err {
        RelayError::TooLarge => NetError::TooLarge,
        RelayError::Truncated => NetError::Truncated,
        RelayError::BadMagic => NetError::BadMagic,
        RelayError::BadKind(kind) => NetError::BadKind(kind),
        RelayError::VersionMismatch { got_version, .. } => NetError::VersionMismatch {
            got: u8::try_from(got_version).unwrap_or(u8::MAX),
        },
        RelayError::BadInput
        | RelayError::UnknownPeer(_)
        | RelayError::Unauthorized
        | RelayError::SlotFull
        | RelayError::QueueFull
        | RelayError::Expired => NetError::BadInput,
    }
}

/// Relay datagram cap: the game cap plus the forward header (magic 2,
/// version 1, kind 1, two ids 16, length 2). The relay never fragments.
pub const MAX_RELAY_DATAGRAM: usize = MAX_RELAY_BYTES + 22;

/// QUIC through the relay: the socket adapter for the real-NAT leg. Every
/// game frame rides a [`RelayMsg::Forward`] between this machine's relay id
/// and the peer's, so the punch only decides the road while bytes always
/// have a path. Registration and the STUN echo are best-effort sends plus
/// flags the caller polls; no wall clock lives here, so sim hashes never
/// see it. On receive, stream classes come back through [`classify`], the
/// same downgrade the UDP path uses.
pub struct RelayTransport {
    sock: UdpSocket,
    leg: RelayLeg,
    registered: bool,
    denied: Option<u8>,
    stun_tx: [u8; 12],
    srflx: Option<SocketAddr>,
}

impl RelayTransport {
    /// Wrap an owned socket and a relay leg, moving the socket to
    /// nonblocking receive.
    pub fn new(sock: UdpSocket, leg: RelayLeg) -> io::Result<Self> {
        sock.set_nonblocking(true)?;
        let (a, b) = (mix64(leg.me).to_le_bytes(), mix64(!leg.me).to_le_bytes());
        let mut stun_tx = [0u8; 12];
        stun_tx[..8].copy_from_slice(&a);
        stun_tx[8..].copy_from_slice(&b[..4]);
        Ok(Self {
            sock,
            leg,
            registered: false,
            denied: None,
            stun_tx,
            srflx: None,
        })
    }

    /// Bind an ephemeral localhost socket for loopback sessions.
    pub fn bind_loopback(leg: RelayLeg) -> io::Result<Self> {
        Self::new(UdpSocket::bind("127.0.0.1:0")?, leg)
    }

    /// This machine's registration at the relay.
    #[must_use]
    pub fn leg(&self) -> RelayLeg {
        self.leg
    }

    /// Local address this end receives on.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    /// True once the relay acknowledged this machine's registration.
    #[must_use]
    pub fn registered(&self) -> bool {
        self.registered
    }

    /// Deny code when the relay refused registration.
    #[must_use]
    pub fn denied(&self) -> Option<u8> {
        self.denied
    }

    /// Public address from the relay's STUN echo, once heard.
    #[must_use]
    pub fn srflx(&self) -> Option<SocketAddr> {
        self.srflx
    }

    /// Offer registration plus a STUN binding request to the relay. A
    /// failed send is loss; the caller repeats this until [`Self::registered`]
    /// flips or [`Self::denied`] names the refusal.
    pub fn send_register(&mut self) {
        let register = RelayMsg::Register {
            client_id: self.leg.me,
            token: self.leg.token,
        };
        if let Ok(bytes) = register.encode() {
            let _ = self.sock.send_to(&bytes, self.leg.addr);
        }
        if self.srflx.is_none() {
            let probe = encode_binding_request(self.stun_tx);
            let _ = self.sock.send_to(&probe, self.leg.addr);
        }
    }

    /// Wrap game bytes as one relay forward from this machine to its peer.
    /// Over-budget frames fail before the network, exactly like the other
    /// transports.
    pub fn wrap(leg: &RelayLeg, bytes: &[u8]) -> Result<Vec<u8>, NetError> {
        check_budget(bytes)?;
        RelayMsg::Forward {
            from: leg.me,
            to: leg.peer,
            payload: bytes.to_vec(),
        }
        .encode()
        .map_err(relay_err)
    }

    /// Unwrap one relay datagram into game bytes when it is a forward from
    /// the peer to this machine; `None` for anything else (acks, denies,
    /// spoofed senders, junk).
    #[must_use]
    pub fn unwrap(leg: &RelayLeg, msg: &[u8]) -> Option<Vec<u8>> {
        match RelayMsg::decode(msg) {
            Ok(RelayMsg::Forward { from, to, payload }) if (from, to) == (leg.peer, leg.me) => {
                Some(payload)
            }
            _ => None,
        }
    }

    fn send_wrapped(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        if to != PeerId(self.leg.addr) {
            return Err(NetError::BadInput);
        }
        let body = Self::wrap(&self.leg, bytes)?;
        self.sock
            .send_to(&body, self.leg.addr)
            .map(|_| ())
            .map_err(|_| NetError::BadInput)
    }
}

impl Transport for RelayTransport {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        self.send_wrapped(to, bytes)
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        _kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        self.send_wrapped(to, bytes)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        let mut out = Vec::new();
        let mut buf = [0u8; MAX_RELAY_DATAGRAM];
        // Bound one drain so a burst never starves the caller.
        for _ in 0..64 {
            match self.sock.recv_from(&mut buf) {
                Ok((n, src)) => {
                    if src != self.leg.addr {
                        continue;
                    }
                    match RelayMsg::decode(&buf[..n]) {
                        Ok(RelayMsg::Forward { from, to, payload })
                            if (from, to) == (self.leg.peer, self.leg.me) =>
                        {
                            out.push(Inbound {
                                from: PeerId(self.leg.addr),
                                frame: classify(&payload),
                            });
                        }
                        Ok(RelayMsg::Registered { client_id }) if client_id == self.leg.me => {
                            self.registered = true;
                        }
                        Ok(RelayMsg::Denied { client_id, code }) if client_id == self.leg.me => {
                            self.denied = Some(code);
                        }
                        Ok(_) => {}
                        Err(_) => {
                            if let Ok(addr) = decode_binding_response(&buf[..n], self.stun_tx) {
                                self.srflx = Some(addr);
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        out
    }
}

/// Deterministic xorshift64 for seeded loss injection (same shape as the
/// session loopback harness: no new dependencies).
fn xorshift(mut x: u64) -> u64 {
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

/// One frame in flight on the deterministic link. Push order is delivery
/// order: every push stamps deliver_at as now + fixed latency while now only
/// rises, so pending is always ordered and receives never sort.
struct Flight {
    deliver_at: u32,
    from: PeerId,
    to: PeerId,
    frame: SimFrame,
}

/// Channel-accurate payload: lossy datagrams mirror QUIC DATAGRAM frames
/// (RFC 9221), ordered lossless messages mirror QUIC streams (RFC 9000).
enum SimFrame {
    Datagram(Vec<u8>),
    Stream { kind: ControlKind, bytes: Vec<u8> },
}

/// Deterministic QUIC channel model and WAN soak harness: datagrams ride a
/// seeded loss plus latency matrix (the loopback shape), streams ride ordered
/// reliable messages with latency only. No sockets, no clock, tick-ordered
/// delivery, so the same seed always yields the same frames.
pub struct SimQuicNet {
    now: u32,
    latency: u32,
    rng: u64,
    members: Vec<PeerId>,
    pending: Vec<Flight>,
    dropped: u64,
    delivered: u64,
    bytes_unreliable: BTreeMap<PeerId, u64>,
    bytes_reliable: BTreeMap<PeerId, u64>,
}

impl SimQuicNet {
    /// Open a link with `latency` ticks of delay and a seeded 10% datagram
    /// loss matrix on both directions.
    fn new(latency: u32, seed: u64) -> Self {
        Self {
            now: 0,
            latency,
            rng: seed | 1,
            members: Vec::new(),
            pending: Vec::new(),
            dropped: 0,
            delivered: 0,
            bytes_unreliable: BTreeMap::new(),
            bytes_reliable: BTreeMap::new(),
        }
    }

    /// Current link tick.
    pub fn now(&self) -> u32 {
        self.now
    }

    /// Datagrams dropped by the loss matrix so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Frames delivered to receivers so far.
    pub fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Bytes sent by one peer as `(unreliable, reliable)`: the per-client
    /// kbps chart source.
    pub fn bytes_sent(&self, peer: PeerId) -> (u64, u64) {
        (
            self.bytes_unreliable.get(&peer).copied().unwrap_or(0),
            self.bytes_reliable.get(&peer).copied().unwrap_or(0),
        )
    }

    fn check_member(&self, to: PeerId) -> Result<(), NetError> {
        if self.members.contains(&to) {
            Ok(())
        } else {
            Err(NetError::BadInput)
        }
    }

    fn send_unreliable_from(
        &mut self,
        from: PeerId,
        to: PeerId,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.check_member(to)?;
        *self.bytes_unreliable.entry(from).or_insert(0) += bytes.len() as u64;
        self.rng = xorshift(self.rng);
        if self.rng.is_multiple_of(10) {
            self.dropped += 1;
            return Ok(());
        }
        self.pending.push(Flight {
            deliver_at: self.now + self.latency,
            from,
            to,
            frame: SimFrame::Datagram(bytes.to_vec()),
        });
        Ok(())
    }

    fn send_reliable_from(
        &mut self,
        from: PeerId,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        check_budget(bytes)?;
        self.check_member(to)?;
        *self.bytes_reliable.entry(from).or_insert(0) += bytes.len() as u64;
        // Streams never lose: QUIC retransmits, the harness delivers with
        // latency only, in send order per connection.
        self.pending.push(Flight {
            deliver_at: self.now + self.latency,
            from,
            to,
            frame: SimFrame::Stream {
                kind,
                bytes: bytes.to_vec(),
            },
        });
        Ok(())
    }

    fn recv_for(&mut self, id: PeerId) -> Vec<Inbound> {
        let now = self.now;
        let mut due = Vec::new();
        let mut rest = Vec::new();
        for flight in self.pending.drain(..) {
            if flight.to == id && flight.deliver_at <= now {
                due.push(flight);
            } else {
                rest.push(flight);
            }
        }
        self.pending = rest;
        // No sort: pending is always in delivery order (see Flight), so due
        // frames inherit the order free.
        self.delivered += due.len() as u64;
        due.into_iter()
            .map(|flight| Inbound {
                from: flight.from,
                frame: match flight.frame {
                    SimFrame::Datagram(bytes) => InboundFrame::Datagram(bytes),
                    SimFrame::Stream { kind, bytes } => InboundFrame::Stream { kind, bytes },
                },
            })
            .collect()
    }

    fn tick(&mut self) {
        self.now += 1;
    }
}

/// Shared owner of one deterministic link; hands out endpoint handles.
pub struct SimQuicFabric {
    net: Arc<Mutex<SimQuicNet>>,
}

impl SimQuicFabric {
    /// Open a link: `latency` ticks of delay, seeded 10% datagram loss.
    pub fn new(latency: u32, seed: u64) -> Self {
        Self {
            net: Arc::new(Mutex::new(SimQuicNet::new(latency, seed))),
        }
    }

    /// Attach an endpoint handle with this address. Unknown destinations
    /// fail sends, so every peer registers before the first tick.
    pub fn endpoint(&self, addr: SocketAddr) -> SimTransport {
        let id = PeerId(addr);
        let mut net = self.net.lock().unwrap_or_else(|poison| poison.into_inner());
        if !net.members.contains(&id) {
            net.members.push(id);
        }
        SimTransport {
            id,
            net: Arc::clone(&self.net),
        }
    }

    /// Advance the link one tick; frames due surface on the next receive.
    pub fn tick(&self) {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .tick();
    }

    /// Current link tick.
    pub fn now(&self) -> u32 {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .now()
    }

    /// Datagrams dropped by the loss matrix so far.
    pub fn dropped(&self) -> u64 {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .dropped()
    }

    /// Frames delivered to receivers so far.
    pub fn delivered(&self) -> u64 {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .delivered()
    }

    /// Bytes sent by one peer as `(unreliable, reliable)`.
    pub fn bytes_sent(&self, peer: PeerId) -> (u64, u64) {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .bytes_sent(peer)
    }
}

/// One endpoint handle on a deterministic link. Handles clone so the
/// session harness can hold both ends; the link stays single-threaded.
#[derive(Clone)]
pub struct SimTransport {
    id: PeerId,
    net: Arc<Mutex<SimQuicNet>>,
}

impl SimTransport {
    /// Address this handle receives on.
    pub fn id(&self) -> PeerId {
        self.id
    }
}

impl Transport for SimTransport {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .send_unreliable_from(self.id, to, bytes)
    }

    fn send_reliable(
        &mut self,
        to: PeerId,
        kind: ControlKind,
        bytes: &[u8],
    ) -> Result<(), NetError> {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .send_reliable_from(self.id, to, kind, bytes)
    }

    fn recv(&mut self) -> Vec<Inbound> {
        self.net
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .recv_for(self.id)
    }
}

// ---------------------------------------------------------------------------
// Real quinn backend: handshake discipline, rustls pin, std runtime.
// ---------------------------------------------------------------------------

/// ALPN pin: only `e2040-net/1` peers complete the handshake; anything else
/// refuses at the TLS layer (RFC 9000 handshake discipline).
#[cfg(feature = "quic")]
pub const QUIC_ALPN: &[u8] = b"e2040-net/1";

/// Server name on the dev and self-hosted path. The invite/relay layer
/// replaces it with per-deployment names.
#[cfg(feature = "quic")]
pub const QUIC_SERVER_NAME: &str = "e2040.local";

/// Keepalive keeps NAT bindings warm without touching sim timing.
#[cfg(feature = "quic")]
pub const QUIC_KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(5);

/// Stream budget per connection: snapshot resync plus lobby, with headroom
/// for cheat reports and voice control. Game data never rides streams.
#[cfg(feature = "quic")]
pub const QUIC_MAX_STREAMS: u32 = 16;

#[cfg(feature = "quic")]
use quinn::rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    version::TLS13,
    DigitallySignedStruct, SignatureScheme,
};
#[cfg(feature = "quic")]
use quinn::{crypto::rustls::QuicClientConfig, rustls};
#[cfg(feature = "quic")]
use quinn::{
    AsyncTimer, AsyncUdpSocket, ConnectError, Connecting, EndpointConfig, Runtime, TransportConfig,
    UdpPoller, VarInt,
};
#[cfg(feature = "quic")]
use std::future::Future;
#[cfg(feature = "quic")]
use std::io::IoSliceMut;
#[cfg(feature = "quic")]
use std::net::IpAddr;
#[cfg(feature = "quic")]
use std::pin::Pin;
#[cfg(feature = "quic")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "quic")]
use std::task::{Context, Poll, Waker};
#[cfg(feature = "quic")]
use std::thread;
#[cfg(feature = "quic")]
#[expect(
    clippy::disallowed_types,
    reason = "transport wall-clock deadline, never hashed."
)]
use std::time::{Duration, Instant};

/// Small QUIC varint from a budget constant. The stream budgets are far below
/// the 62-bit ceiling, so failure here is a programming error, not input.
#[cfg(feature = "quic")]
fn varint(value: u32) -> VarInt {
    VarInt::from_u64(value as u64).expect("stream budgets fit the QUIC varint range")
}

/// QUIC transport shape for game traffic: datagrams for inputs/snapshots,
/// streams for lobby. Keepalive plus stream budgets mirror the channel
/// table; congestion control stays quinn default until a WAN soak measures it.
#[cfg(feature = "quic")]
pub fn quinn_transport() -> TransportConfig {
    let mut transport = TransportConfig::default();
    transport
        .keep_alive_interval(Some(QUIC_KEEPALIVE))
        .datagram_receive_buffer_size(Some(64 * 1024))
        .max_concurrent_bidi_streams(varint(QUIC_MAX_STREAMS))
        .max_concurrent_uni_streams(varint(QUIC_MAX_STREAMS))
        .send_window(512 * 1024)
        .stream_receive_window(varint(256 * 1024));
    transport
}

/// Test-only certificate verifier: accepts any server certificate so
/// loopback handshakes run with no CA. Never ships: the relay layer replaces
/// it with a pinned self-signed CA, and the name marks every use site.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct AcceptAnyCert;

#[cfg(feature = "quic")]
impl ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

/// Client crypto with the rustls pin: explicit ring provider, TLS 1.3 only,
/// our ALPN. A rustls drift that drops TLS 1.3 fails here loudly instead of
/// negotiating down: that `?` is the drift pin.
#[cfg(feature = "quic")]
pub fn client_crypto() -> Result<quinn::ClientConfig, rustls::Error> {
    let provider = Arc::new(default_provider());
    let mut inner = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    inner.alpn_protocols = vec![QUIC_ALPN.to_vec()];
    let quic_crypto =
        QuicClientConfig::try_from(inner).expect("the ring provider ships the QUIC initial suite");
    let mut client = quinn::ClientConfig::new(Arc::new(quic_crypto));
    client.transport_config(Arc::new(quinn_transport()));
    Ok(client)
}

/// Server crypto from caller-owned DER: chain plus private key. Empty
/// chains refuse before touching rustls; malformed DER fails inside
/// `with_single_cert`. Minting the deployment certificate belongs to the
/// relay layer (pinned self-signed CA); this constructor already enforces the
/// shape.
#[cfg(feature = "quic")]
pub fn server_crypto(
    chain_der: Vec<CertificateDer<'static>>,
    key_der: PrivateKeyDer<'static>,
) -> Result<quinn::ServerConfig, rustls::Error> {
    if chain_der.is_empty() {
        return Err(rustls::Error::NoCertificatesPresented);
    }
    quinn::ServerConfig::with_single_cert(chain_der, key_der)
}

/// Drive one future to completion on the calling thread: a thread-parker
/// executor for the std QUIC runtime. IO tasks only, never the sim tick.
#[cfg(feature = "quic")]
fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    struct ThreadParker {
        thread: thread::Thread,
    }
    impl std::task::Wake for ThreadParker {
        fn wake(self: Arc<Self>) {
            self.thread.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.thread.unpark();
        }
    }
    let parker = Arc::new(ThreadParker {
        thread: thread::current(),
    });
    let waker = Waker::from(parker);
    let mut context = Context::from_waker(&waker);
    let mut boxed = Box::pin(future);
    loop {
        match boxed.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

/// Shared timer state between the async handle and its sleeper thread.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct TimerState {
    #[expect(
        clippy::disallowed_types,
        reason = "transport wall-clock deadline, never hashed."
    )]
    deadline: Instant,
    generation: u64,
    waker: Option<Waker>,
}

/// Async timer over a sleeper thread: correct wakeups with no executor
/// dependency. One short-lived thread per arm; IO paths only.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct StdTimer {
    shared: Arc<Mutex<TimerState>>,
    spawn_failures: Arc<AtomicU64>,
}

#[cfg(feature = "quic")]
#[expect(
    clippy::disallowed_types,
    reason = "transport wall-clock deadline, never hashed."
)]
fn spawn_sleeper(
    shared: std::sync::Weak<Mutex<TimerState>>,
    failures: &Arc<AtomicU64>,
    generation: u64,
    deadline: Instant,
) {
    let failures = Arc::clone(failures);
    if thread::Builder::new()
        .name("quinn-std-timer".into())
        .spawn(move || {
            #[expect(
                clippy::disallowed_methods,
                reason = "transport wall-clock deadline, never hashed."
            )]
            let now = Instant::now(); // det-ok: transport wall-clock deadline, never hashed.
            if deadline > now {
                thread::sleep(deadline - now);
            }
            if let Some(shared) = shared.upgrade() {
                let mut state = shared.lock().unwrap_or_else(|poison| poison.into_inner());
                if state.generation == generation {
                    if let Some(waker) = state.waker.take() {
                        waker.wake();
                    }
                }
            }
        })
        .is_err()
    {
        failures.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(feature = "quic")]
impl AsyncTimer for StdTimer {
    #[expect(
        clippy::disallowed_types,
        reason = "transport wall-clock deadline, never hashed."
    )]
    fn reset(self: Pin<&mut Self>, deadline: Instant) {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.generation += 1;
        state.deadline = deadline;
        let generation = state.generation;
        #[expect(
            clippy::disallowed_methods,
            reason = "transport wall-clock deadline, never hashed."
        )]
        let now = Instant::now(); // det-ok: transport wall-clock deadline, never hashed.
        if deadline <= now {
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
            return;
        }
        drop(state);
        spawn_sleeper(
            Arc::downgrade(&self.shared),
            &self.spawn_failures,
            generation,
            deadline,
        );
    }

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        #[expect(
            clippy::disallowed_types,
            reason = "transport readiness check, never hashed."
        )]
        #[expect(
            clippy::disallowed_methods,
            reason = "transport readiness check, never hashed."
        )]
        let now = Instant::now(); // det-ok: transport readiness check, never hashed.
        if now >= state.deadline {
            return Poll::Ready(());
        }
        state.waker = Some(context.waker().clone());
        // Re-check after registering: the sleeper may have fired first.
        #[expect(
            clippy::disallowed_types,
            reason = "transport readiness check, never hashed."
        )]
        #[expect(
            clippy::disallowed_methods,
            reason = "transport readiness check, never hashed."
        )]
        let now = Instant::now(); // det-ok: transport readiness check, never hashed.
        if now >= state.deadline {
            state.waker = None;
            return Poll::Ready(());
        }
        Poll::Pending
    }
}

/// One datagram buffered from the UDP socket with its metadata.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct SocketDatagram {
    data: Vec<u8>,
    addr: SocketAddr,
    ecn: Option<quinn::udp::EcnCodepoint>,
    dst_ip: Option<IpAddr>,
}

/// Queue plus registered wakers shared with the reader thread.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct SocketShared {
    queue: VecDeque<SocketDatagram>,
    wakers: Vec<Waker>,
}

/// std UDP socket adapted to quinn: ECN and segmentation come from
/// quinn-udp, readiness from a reader thread, so no async runtime crate is
/// needed in this crate. Test and loopback use; the relay server binary may
/// use the Tokio runtime instead.
#[cfg(feature = "quic")]
struct StdUdpSocket {
    local: SocketAddr,
    state: Arc<quinn::udp::UdpSocketState>,
    send_sock: UdpSocket,
    block_sock: UdpSocket,
    shared: Arc<Mutex<SocketShared>>,
    poll_wakers: Mutex<Vec<Waker>>,
}

#[cfg(feature = "quic")]
impl std::fmt::Debug for StdUdpSocket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StdUdpSocket")
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "quic")]
fn udp_reader(
    state: &quinn::udp::UdpSocketState,
    sock: &UdpSocket,
    weak: std::sync::Weak<Mutex<SocketShared>>,
) {
    let mut buf = vec![0u8; 65_536];
    let mut meta = quinn::udp::RecvMeta::default();
    loop {
        let Some(shared) = weak.upgrade() else {
            break;
        };
        let mut slices = [IoSliceMut::new(&mut buf)];
        match state.recv(
            quinn::udp::UdpSockRef::from(sock),
            &mut slices,
            std::slice::from_mut(&mut meta),
        ) {
            Ok(_) => {
                // Split GRO-coalesced segments on stride boundaries.
                let stride = if meta.stride == 0 || meta.stride > meta.len {
                    meta.len
                } else {
                    meta.stride
                };
                if meta.len == 0 || stride == 0 {
                    continue;
                }
                let mut datagrams = Vec::new();
                let mut offset = 0;
                while offset < meta.len {
                    let end = (offset + stride).min(meta.len);
                    datagrams.push(SocketDatagram {
                        data: buf[offset..end].to_vec(),
                        addr: meta.addr,
                        ecn: meta.ecn,
                        dst_ip: meta.dst_ip,
                    });
                    offset = end;
                }
                let mut shared = shared.lock().unwrap_or_else(|poison| poison.into_inner());
                shared.queue.extend(datagrams);
                for waker in shared.wakers.drain(..) {
                    waker.wake();
                }
            }
            Err(_) => {
                // Timeouts pace the liveness check; transient IO errors drop
                // like loss. The loop exits when the owner drops.
                continue;
            }
        }
    }
}

#[cfg(feature = "quic")]
impl StdUdpSocket {
    fn wrap(sock: UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        sock.set_nonblocking(true)?;
        let state = Arc::new(quinn::udp::UdpSocketState::new(
            quinn::udp::UdpSockRef::from(&sock),
        )?);
        let local = sock.local_addr()?;
        let reader = sock.try_clone()?;
        reader.set_nonblocking(false)?;
        reader.set_read_timeout(Some(Duration::from_millis(20)))?;
        let blocking = sock.try_clone()?;
        blocking.set_nonblocking(false)?;
        blocking.set_write_timeout(Some(Duration::from_millis(5)))?;
        let shared = Arc::new(Mutex::new(SocketShared {
            queue: VecDeque::new(),
            wakers: Vec::new(),
        }));
        let weak = Arc::downgrade(&shared);
        let reader_state = Arc::clone(&state);
        thread::Builder::new()
            .name("quinn-std-udp".into())
            .spawn(move || udp_reader(&reader_state, &reader, weak))?;
        Ok(Arc::new(Self {
            local,
            state,
            send_sock: sock,
            block_sock: blocking,
            shared,
            poll_wakers: Mutex::new(Vec::new()),
        }))
    }
}

#[cfg(feature = "quic")]
impl AsyncUdpSocket for StdUdpSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(StdPoller { socket: self })
    }

    fn try_send(&self, transmit: &quinn::udp::Transmit<'_>) -> io::Result<()> {
        match self
            .state
            .try_send(quinn::udp::UdpSockRef::from(&self.send_sock), transmit)
        {
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                // Brief blocking retry so the driver never spin-loops on a
                // congested socket; the 5 ms cap bounds the stall.
                self.state
                    .try_send(quinn::udp::UdpSockRef::from(&self.block_sock), transmit)
            }
            other => other,
        }
    }

    fn poll_recv(
        &self,
        context: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [quinn::udp::RecvMeta],
    ) -> Poll<io::Result<usize>> {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut filled = 0;
        while filled < bufs.len() && filled < meta.len() {
            let Some(datagram) = shared.queue.pop_front() else {
                break;
            };
            if datagram.data.len() > bufs[filled].len() {
                // Driver buffers always exceed the path MTU; drop rather
                // than corrupt.
                continue;
            }
            bufs[filled][..datagram.data.len()].copy_from_slice(&datagram.data);
            meta[filled] = quinn::udp::RecvMeta {
                addr: datagram.addr,
                len: datagram.data.len(),
                stride: datagram.data.len(),
                ecn: datagram.ecn,
                dst_ip: datagram.dst_ip,
            };
            filled += 1;
        }
        if filled > 0 {
            return Poll::Ready(Ok(filled));
        }
        shared.wakers.push(context.waker().clone());
        Poll::Pending
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local)
    }

    fn max_transmit_segments(&self) -> usize {
        1
    }

    fn max_receive_segments(&self) -> usize {
        1
    }

    fn may_fragment(&self) -> bool {
        self.state.may_fragment()
    }
}

/// Write-readiness poller: re-wakes the driver after 1 ms so a congested
/// send retries without spinning. Rare path; test traffic never hits it.
#[cfg(feature = "quic")]
#[derive(Debug)]
struct StdPoller {
    socket: Arc<StdUdpSocket>,
}

#[cfg(feature = "quic")]
impl UdpPoller for StdPoller {
    fn poll_writable(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let waker = context.waker().clone();
        {
            self.socket
                .poll_wakers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(waker);
        }
        let socket = Arc::clone(&self.socket);
        let _ = thread::Builder::new()
            .name("quinn-std-poll".into())
            .spawn(move || {
                thread::sleep(Duration::from_millis(1));
                let waker = socket
                    .poll_wakers
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .pop();
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        Poll::Pending
    }
}

/// std-only quinn runtime: timers on sleeper threads, driver tasks on
/// parked threads, UDP on quinn-udp plus a reader thread. No executor
/// dependency in this crate; IO paths only, never the sim tick.
#[cfg(feature = "quic")]
#[derive(Debug, Default)]
pub struct StdRuntime {
    spawn_failures: AtomicU64,
    timer_failures: Arc<AtomicU64>,
}

#[cfg(feature = "quic")]
impl StdRuntime {
    /// Open a runtime handle for endpoint construction.
    pub fn new() -> Self {
        Self::default()
    }

    /// Background spawns that failed outright; tests pin this at zero.
    pub fn spawn_failures(&self) -> u64 {
        self.spawn_failures.load(Ordering::Relaxed) + self.timer_failures.load(Ordering::Relaxed)
    }
}

#[cfg(feature = "quic")]
impl Runtime for StdRuntime {
    #[expect(
        clippy::disallowed_types,
        reason = "transport wall-clock deadline, never hashed."
    )]
    fn new_timer(&self, deadline: Instant) -> Pin<Box<dyn AsyncTimer>> {
        let shared = Arc::new(Mutex::new(TimerState {
            deadline,
            generation: 0,
            waker: None,
        }));
        #[expect(
            clippy::disallowed_methods,
            reason = "transport wall-clock deadline, never hashed."
        )]
        let now = Instant::now(); // det-ok: transport wall-clock deadline, never hashed.
        if deadline > now {
            spawn_sleeper(Arc::downgrade(&shared), &self.timer_failures, 0, deadline);
        }
        Box::pin(StdTimer {
            shared,
            spawn_failures: Arc::clone(&self.timer_failures),
        })
    }

    fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        if thread::Builder::new()
            .name("quinn-std-driver".into())
            .spawn(move || block_on(future))
            .is_err()
        {
            self.spawn_failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn wrap_udp_socket(&self, sock: UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        StdUdpSocket::wrap(sock)
    }
}

/// Live quinn endpoint on the std runtime: real socket, real driver, no
/// Tokio dependency in this crate. Handshake discipline: dialing without
/// client crypto refuses immediately; connecting without a server waits on
/// the future, which tests drop instead of awaiting.
#[cfg(feature = "quic")]
pub struct QuinnEndpoint {
    inner: quinn::Endpoint,
}

#[cfg(feature = "quic")]
impl QuinnEndpoint {
    /// Bind a client-only endpoint on an ephemeral port.
    pub fn bind(addr: SocketAddr) -> io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        let inner = quinn::Endpoint::new(
            EndpointConfig::default(),
            None,
            sock,
            Arc::new(StdRuntime::new()),
        )?;
        Ok(Self { inner })
    }

    /// Local address this endpoint receives on.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Install the client crypto built by [`client_crypto`]; required
    /// before [`Self::connect`] or it refuses with `NoDefaultClientConfig`.
    pub fn set_client_config(&mut self, config: quinn::ClientConfig) {
        self.inner.set_default_client_config(config);
    }

    /// Start a handshake future. Fails fast without client crypto; without
    /// a live server the future stays pending, which is the relay layer's
    /// handshake to run.
    pub fn connect(&self, addr: SocketAddr, server_name: &str) -> Result<Connecting, ConnectError> {
        self.inner.connect(addr, server_name)
    }

    /// Close the endpoint; driver tasks unwind after this.
    pub fn close(&self) {
        let code = VarInt::from_u64(0).expect("zero is a valid QUIC varint");
        self.inner.close(code, b"done");
    }
}

#[cfg(all(test, feature = "quic"))]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn quic_channel_caps_reject_oversize() {
        let fabric = SimQuicFabric::new(3, 0xC10C);
        let mut client = fabric.endpoint(addr(1));
        let server = addr(2);
        let _server = fabric.endpoint(server);
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(
            client.send_unreliable(PeerId(server), &big),
            Err(NetError::TooLarge)
        );
        assert_eq!(
            client.send_reliable(PeerId(server), ControlKind::Lobby, &big),
            Err(NetError::TooLarge)
        );
        assert!(client.recv().is_empty());
        assert_eq!(fabric.bytes_sent(PeerId(addr(1))), (0, 0));
    }

    #[test]
    fn quic_streams_stay_ordered_while_datagrams_drop() {
        let fabric = SimQuicFabric::new(3, 0xC10C);
        let mut client = fabric.endpoint(addr(1));
        assert_eq!(client.id(), PeerId(addr(1)), "handle names its address");
        let server = PeerId(addr(2));
        let mut server_end = fabric.endpoint(addr(2));
        // Interleave lossy datagrams with reliable control until the matrix
        // has dropped at least once; the round count is seed-determined.
        let mut rounds = 0u8;
        while fabric.dropped() == 0 && rounds < 200 {
            let datagram = vec![rounds; 32];
            client
                .send_unreliable(server, &datagram)
                .expect("datagram fits");
            let control = vec![rounds; 8];
            client
                .send_reliable(server, ControlKind::Lobby, &control)
                .expect("control fits");
            fabric.tick();
            rounds += 1;
        }
        assert!(
            fabric.dropped() > 0,
            "the loss matrix must have dropped a datagram"
        );
        for _ in 0..4 {
            fabric.tick();
        }
        assert_eq!(fabric.now(), u32::from(rounds) + 4, "link ticks advance");
        let frames = server_end.recv();
        let streams: Vec<Vec<u8>> = frames
            .iter()
            .filter_map(|inbound| match &inbound.frame {
                InboundFrame::Stream { kind, bytes } => {
                    assert_eq!(*kind, ControlKind::Lobby);
                    assert_eq!(inbound.from, PeerId(addr(1)));
                    Some(bytes.clone())
                }
                InboundFrame::Datagram(_) => None,
            })
            .collect();
        // Every stream message arrives exactly once, in send order, even
        // though the loss matrix drops datagrams around them.
        assert_eq!(streams.len(), rounds as usize, "streams never lose");
        for (index, bytes) in streams.iter().enumerate() {
            assert_eq!(*bytes, vec![index as u8; 8]);
        }
    }

    #[test]
    fn quic_misaddressed_send_rejected() {
        let fabric = SimQuicFabric::new(1, 7);
        let mut client = fabric.endpoint(addr(1));
        let _server = fabric.endpoint(addr(2));
        let payload = vec![1u8; 16];
        assert_eq!(
            client.send_unreliable(PeerId(addr(9)), &payload),
            Err(NetError::BadInput)
        );
        assert_eq!(
            client.send_reliable(PeerId(addr(9)), ControlKind::Lobby, &payload),
            Err(NetError::BadInput)
        );
        fabric.tick();
        fabric.tick();
        assert!(client.recv().is_empty());
    }

    #[test]
    fn quic_test_verifier_accepts_anything() {
        // Documents the insecurity: the test-only verifier accepts garbage
        // DER under any name, which is why the relay layer must replace it.
        let verifier = AcceptAnyCert;
        let garbage = CertificateDer::from(vec![0x30, 0x82, 0x01, 0x02]);
        for name in ["e2040.local", "wrong.invalid"] {
            let server_name = ServerName::try_from(name).expect("test names are valid DNS names");
            verifier
                .verify_server_cert(&garbage, &[], &server_name, &[], UnixTime::now())
                .expect("accept-any accepts");
        }
        assert_eq!(verifier.supported_verify_schemes().len(), 4);
    }

    #[test]
    fn quic_client_crypto_builds_with_pin() {
        // Fails loudly on rustls drift (no TLS 1.3) instead of building.
        client_crypto().expect("ring plus TLS 1.3 plus ALPN builds");
    }

    #[test]
    fn quic_server_crypto_rejects_bad_material() {
        assert!(
            matches!(
                server_crypto(Vec::new(), PrivateKeyDer::Pkcs8(vec![1, 2, 3].into())),
                Err(rustls::Error::NoCertificatesPresented)
            ),
            "empty chains refuse before parsing"
        );
        let garbage_chain = vec![CertificateDer::from(vec![0x30, 0x82, 0x01, 0x02])];
        assert!(
            server_crypto(garbage_chain, PrivateKeyDer::Pkcs8(vec![9; 32].into())).is_err(),
            "malformed DER never becomes a server config"
        );
    }

    #[test]
    fn quic_endpoint_binds_and_closes() {
        let runtime = Arc::new(StdRuntime::new());
        let sock = UdpSocket::bind("127.0.0.1:0").expect("loopback bind");
        let endpoint = quinn::Endpoint::new(
            EndpointConfig::default(),
            None,
            sock,
            Arc::clone(&runtime) as Arc<dyn Runtime>,
        )
        .expect("endpoint builds on the std runtime");
        let local = endpoint.local_addr().expect("bound address");
        assert_ne!(local.port(), 0, "ephemeral port assigned");
        endpoint.close(VarInt::from_u64(0).expect("zero varint"), b"done");
        drop(endpoint);
        assert_eq!(runtime.spawn_failures(), 0, "no background task failed");
    }

    #[test]
    fn quic_connect_needs_client_crypto() {
        let mut endpoint =
            QuinnEndpoint::bind("127.0.0.1:0".parse().expect("loopback")).expect("loopback bind");
        assert_ne!(
            endpoint.local_addr().expect("bound address").port(),
            0,
            "ephemeral port assigned"
        );
        let peer: SocketAddr = "127.0.0.1:1".parse().expect("closed port");
        // Handshake discipline: dialing without crypto refuses immediately.
        assert!(matches!(
            endpoint.connect(peer, QUIC_SERVER_NAME),
            Err(ConnectError::NoDefaultClientConfig)
        ));
        endpoint.set_client_config(client_crypto().expect("client crypto"));
        // With crypto the handshake starts; no server answers, so the test
        // drops the future instead of awaiting it (no wall clock).
        let connecting = endpoint
            .connect(peer, QUIC_SERVER_NAME)
            .expect("handshake starts");
        drop(connecting);
        endpoint.close();
    }
}

#[cfg(all(test, feature = "quic"))]
mod channel_route_tests {
    use super::*;
    use crate::netcode::Channel;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn channel_route_table_has_unique_ids_and_caps() {
        assert_eq!(CHANNEL_TABLE.len(), 3);
        assert_eq!(channel_id(Channel::Unreliable), 0);
        assert_eq!(channel_id(Channel::Sequenced), 1);
        assert_eq!(channel_id(Channel::Reliable), 2);
        for row in CHANNEL_TABLE {
            assert_eq!(row.max_memory_usage_bytes, CHANNEL_MEMORY_BYTES);
            assert!(row.max_memory_usage_bytes > MAX_PACKET_BYTES);
        }
        // Link mapping: only the ordered channel rides streams; sequenced
        // frames keep caller-side order over datagrams.
        assert!(!channel_is_reliable(Channel::Unreliable));
        assert!(!channel_is_reliable(Channel::Sequenced));
        assert!(channel_is_reliable(Channel::Reliable));
        // Promise mapping stays put: sequenced is unordered, reliable is
        // ordered; the two never swap.
        assert_eq!(
            channel_config(Channel::Sequenced).delivery,
            ChannelDelivery::ReliableUnordered
        );
        assert_eq!(
            channel_config(Channel::Reliable).delivery,
            ChannelDelivery::ReliableOrdered
        );
    }

    #[test]
    fn channel_route_reliable_rides_streams_rest_ride_datagrams() {
        // Reliable leg: streams never lose, so the frame always arrives.
        let fabric = SimQuicFabric::new(1, 0xC4A1);
        let mut from = fabric.endpoint(addr(11));
        let to = PeerId(addr(12));
        let mut recv = fabric.endpoint(addr(12));
        let payload = vec![9u8; 32];
        send_on_channel(
            &mut from,
            to,
            Channel::Reliable,
            ControlKind::Snapshot,
            &payload,
        )
        .expect("stream fits");
        fabric.tick();
        fabric.tick();
        let frames = recv.recv();
        assert_eq!(frames.len(), 1, "streams never lose");
        match &frames[0].frame {
            InboundFrame::Stream { kind, bytes } => {
                assert_eq!(*kind, ControlKind::Snapshot);
                assert_eq!(*bytes, payload);
            }
            InboundFrame::Datagram(_) => panic!("reliable channels never ride datagrams"),
        }
        // Lossy legs: the frame arrives or the drop is counted, and an
        // arrived frame is always a datagram, whatever kind was passed.
        for ch in [Channel::Unreliable, Channel::Sequenced] {
            let link = SimQuicFabric::new(1, 0xC4A1);
            let mut send = link.endpoint(addr(31));
            let dest = PeerId(addr(32));
            let mut take = link.endpoint(addr(32));
            send_on_channel(&mut send, dest, ch, ControlKind::Snapshot, &payload)
                .expect("datagram fits");
            link.tick();
            link.tick();
            let got = take.recv();
            assert_eq!(
                got.len() as u64 + link.dropped(),
                1,
                "sent frame is delivered or counted"
            );
            for inbound in &got {
                match &inbound.frame {
                    InboundFrame::Datagram(bytes) => assert_eq!(*bytes, payload),
                    InboundFrame::Stream { .. } => {
                        panic!("unreliable channels never ride streams")
                    }
                }
            }
        }
    }

    #[test]
    fn channel_route_frame_cap_holds_on_both_paths() {
        let fabric = SimQuicFabric::new(1, 3);
        let mut from = fabric.endpoint(addr(21));
        let to = PeerId(addr(22));
        let _recv = fabric.endpoint(addr(22));
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(
            send_on_channel(&mut from, to, Channel::Unreliable, ControlKind::Lobby, &big),
            Err(NetError::TooLarge)
        );
        assert_eq!(
            send_on_channel(&mut from, to, Channel::Sequenced, ControlKind::Lobby, &big),
            Err(NetError::TooLarge)
        );
        assert_eq!(
            send_on_channel(
                &mut from,
                to,
                Channel::Reliable,
                ControlKind::Snapshot,
                &big
            ),
            Err(NetError::TooLarge)
        );
        assert_eq!(fabric.delivered(), 0, "refused frames never enter the link");
    }

    #[test]
    fn channel_route_priorities_shed_mutations_first() {
        // Donor priority map: reliable prompts at 10 and keep a backlog,
        // lossy mutations at 1 with no backlog and no requeue.
        assert_eq!(channel_priority(Channel::Reliable), 10, "snapshots outrank");
        assert_eq!(
            channel_priority(Channel::Sequenced),
            1,
            "mutations shed first"
        );
        assert_eq!(
            channel_priority(Channel::Unreliable),
            1,
            "inputs shed first"
        );
        assert!(
            channel_config(Channel::Reliable).retry_unsent,
            "backlog retries"
        );
        assert!(
            !channel_config(Channel::Sequenced).retry_unsent,
            "mutations never requeue"
        );
        assert!(
            !channel_config(Channel::Unreliable).retry_unsent,
            "inputs never requeue"
        );
        // Full-budget shed over the wire cap: lowest priority drops first
        // (ties keep table order), so a full budget drops mutations before
        // snapshots. The pick is a pure table read, so two runs keep
        // identical bytes.
        fn shed_keep(pending: &[(Channel, Vec<u8>)], budget: usize) -> Vec<Vec<u8>> {
            let mut order: Vec<usize> = (0..pending.len()).collect();
            order.sort_by_key(|&i| (channel_priority(pending[i].0), channel_id(pending[i].0)));
            let mut over: usize = pending.iter().map(|(_, b)| b.len()).sum();
            let mut shed = vec![false; pending.len()];
            for &i in &order {
                if over <= budget {
                    break;
                }
                over -= pending[i].1.len();
                shed[i] = true;
            }
            pending
                .iter()
                .enumerate()
                .filter(|(i, _)| !shed[*i])
                .map(|(_, (_, b))| b.clone())
                .collect()
        }
        let pending = [
            (Channel::Sequenced, vec![1u8; 900]),
            (Channel::Reliable, vec![2u8; 900]),
            (Channel::Unreliable, vec![3u8; 900]),
        ];
        let first = shed_keep(&pending, MAX_PACKET_BYTES);
        let second = shed_keep(&pending, MAX_PACKET_BYTES);
        assert_eq!(first, second, "shed pick is deterministic");
        assert_eq!(
            first,
            vec![vec![2u8; 900]],
            "mutations drop before snapshots"
        );
    }
}

#[cfg(all(test, feature = "quic"))]
mod relay_transport_tests {
    use super::*;
    use crate::relay::{InviteCode, RelayConfig, RelayHub};
    use crate::split::invite_ids;

    fn relay() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 7041))
    }

    /// Test-only leg pair: the same invite code maps the same host/join id
    /// pair on both sides via `split::invite_ids` (mirrored roles).
    fn legs_at(relay_addr: SocketAddr) -> (RelayLeg, RelayLeg) {
        let mut rng = 0x0A11_CE00_1234_5678u64;
        let code = InviteCode::mint(&mut rng);
        let token = RelayConfig::default().token;
        let (host_id, join_id) = invite_ids(&code);
        let mk = |me: u64, peer: u64| RelayLeg {
            addr: relay_addr,
            token,
            me,
            peer,
        };
        (mk(host_id, join_id), mk(join_id, host_id))
    }

    fn legs() -> (RelayLeg, RelayLeg) {
        legs_at(relay())
    }

    #[test]
    fn relay_wrap_rides_the_hub_between_invite_legs() {
        let (host_leg, join_leg) = legs();
        let mut hub = RelayHub::new(RelayConfig::default());
        let (host_src, join_src) = (
            SocketAddr::from(([127, 0, 0, 1], 5001)),
            SocketAddr::from(([127, 0, 0, 1], 5002)),
        );
        for (leg, src) in [(host_leg, host_src), (join_leg, join_src)] {
            let register = RelayMsg::Register {
                client_id: leg.me,
                token: leg.token,
            }
            .encode()
            .expect("register encodes");
            assert_eq!(hub.handle(src, &register).len(), 1, "relay acks");
        }
        let payload = vec![7u8; 32];
        let wire = RelayTransport::wrap(&host_leg, &payload).expect("fits");
        let out = hub.handle(host_src, &wire);
        assert_eq!(out.len(), 1, "one forward drains at once");
        assert_eq!(out[0].0, join_src, "delivered to the peer address");
        assert_eq!(
            RelayTransport::unwrap(&join_leg, &out[0].1),
            Some(payload),
            "the peer recovers the game bytes"
        );
        assert_eq!(
            RelayTransport::unwrap(&host_leg, &out[0].1),
            None,
            "the sender's own leg never accepts its echo"
        );
        let full = vec![9u8; MAX_PACKET_BYTES];
        let full_wire = RelayTransport::wrap(&host_leg, &full).expect("full frames fit");
        assert_eq!(
            full_wire.len(),
            MAX_RELAY_DATAGRAM,
            "a full frame plus header is the datagram cap"
        );
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(
            RelayTransport::wrap(&host_leg, &big),
            Err(NetError::TooLarge),
            "over-budget frames fail before the network"
        );
    }

    #[test]
    fn relay_frames_classify_like_udp() {
        let (host_leg, join_leg) = legs();
        let plain = vec![1u8; 16];
        let wire = RelayTransport::wrap(&host_leg, &plain).expect("fits");
        let game = RelayTransport::unwrap(&join_leg, &wire).expect("peer leg accepts");
        assert!(matches!(classify(&game), InboundFrame::Datagram(_)));
        let mut lobby_bytes = lobby::LOBBY_MAGIC.to_le_bytes().to_vec();
        lobby_bytes.extend_from_slice(&[0u8; 8]);
        assert!(
            matches!(
                classify(&lobby_bytes),
                InboundFrame::Stream {
                    kind: ControlKind::Lobby,
                    ..
                }
            ),
            "lobby bytes ride the reliable lane, as on UDP"
        );
    }

    #[test]
    fn relay_transport_names_its_relay_or_refuses() {
        let (host_leg, _) = legs();
        let mut transport = RelayTransport::bind_loopback(host_leg).expect("loopback bind");
        assert_eq!(transport.leg(), host_leg);
        assert!(!transport.registered(), "nothing acked yet");
        assert_eq!(transport.denied(), None);
        assert_eq!(transport.srflx(), None);
        transport.send_register();
        assert!(transport.recv().is_empty(), "no relay answers on loopback");
        assert!(!transport.registered());
        let payload = vec![3u8; 16];
        let stranger = PeerId(SocketAddr::from(([127, 0, 0, 1], 9)));
        assert_eq!(
            transport.send_unreliable(stranger, &payload),
            Err(NetError::BadInput),
            "only the relay address sends"
        );
        assert_eq!(
            transport.send_reliable(stranger, ControlKind::Lobby, &payload),
            Err(NetError::BadInput),
            "streams name the relay too"
        );
        transport
            .send_unreliable(PeerId(relay()), &payload)
            .expect("the relay address sends");
        transport
            .send_reliable(PeerId(relay()), ControlKind::Lobby, &payload)
            .expect("the relay address streams");
    }

    #[test]
    fn relay_recv_refuses_forged_source() {
        // Caller: `RelayTransport::recv` live relay leg.
        // Behavior: only datagrams from `leg.addr` update state; a forged
        // source with valid ids is refused and leaves state unchanged, while
        // the relay-leg source is accepted.
        use std::net::UdpSocket;
        let relay_sock = UdpSocket::bind("127.0.0.1:0").expect("fake relay binds");
        relay_sock
            .set_nonblocking(true)
            .expect("fake relay nonblocking");
        let relay_addr = relay_sock.local_addr().expect("relay addr");
        let (host_leg, join_leg) = legs_at(relay_addr);
        let mut transport = RelayTransport::bind_loopback(join_leg).expect("loopback bind");
        let target = transport.local_addr().expect("transport addr");
        let payload = vec![7u8; 16];
        let wire = RelayMsg::Forward {
            from: join_leg.peer,
            to: join_leg.me,
            payload: payload.clone(),
        }
        .encode()
        .expect("forward encodes");
        // Forged source with valid ids is refused and leaves state unchanged.
        let forger = UdpSocket::bind("127.0.0.1:0").expect("forger binds");
        forger.send_to(&wire, target).expect("forger sends");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let forged = transport.recv();
        assert!(
            forged.is_empty(),
            "forged source must be refused, got {forged:?}"
        );
        assert!(!transport.registered(), "forged forward never registers");
        assert_eq!(transport.denied(), None, "forged forward never denies");
        // Relay-leg source with the same ids is accepted.
        relay_sock.send_to(&wire, target).expect("relay sends");
        let mut got = Vec::new();
        for _ in 0..100 {
            let mut batch = transport.recv();
            if !batch.is_empty() {
                got.append(&mut batch);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.len(), 1, "relay-leg forward is accepted");
        assert_eq!(got[0].from, PeerId(relay_addr), "sender names the relay");
        match &got[0].frame {
            InboundFrame::Datagram(bytes) => assert_eq!(*bytes, payload),
            other @ InboundFrame::Stream { .. } => {
                panic!("forward rides datagrams, got {other:?}")
            }
        }
        // Forged control leaves state unchanged.
        let forged_reg = RelayMsg::Registered {
            client_id: join_leg.me,
        }
        .encode()
        .expect("registered encodes");
        forger.send_to(&forged_reg, target).expect("forger sends");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            transport.recv().is_empty(),
            "forged registered must be refused"
        );
        assert!(
            !transport.registered(),
            "forged registered leaves state unchanged"
        );
        let _ = host_leg;
    }
}

#[cfg(all(test, feature = "quic"))]
mod on30_spike_tests {
    /// ON30 spike: the relay leg through the invite/punch path converges with
    /// full checkpoint equality under the WAN soak 150 ms / 2 pct loss profile.
    /// Metric: relay-leg digest equals the one-process twin x2, punch-or-fallback
    /// inside 300 ticks, rejoin inside 300 ticks. Kill: digest divergence twice
    /// in a row, or either bound blown, kills the QUIC-relay leg and keeps the
    /// UDP courier (ON30 already counts it).
    #[test]
    fn on30_spike_relay_leg_matches_twin_with_punch_and_rejoin_inside_300() {
        use crate::online::{run_online, HOP_LATENCY_TICKS};
        use crate::relay::{NatKind, PUNCH_TIMEOUT_TICKS};
        use crate::wan_soak::{run_fault, Fault, CONVERGE_BAR_TICKS};
        // The soak half: 150 ms / 2 pct loss still converges inside 300 ticks.
        let soak = run_fault(7, Fault::Loss, 240).expect("loss must converge");
        soak.check().expect("loss report must pass its bars");
        assert!(
            soak.converge_ticks <= CONVERGE_BAR_TICKS,
            "rejoin converges inside 300 ticks, got {}",
            soak.converge_ticks
        );
        // The relay half: loopback pair through the relay hub, digest x2.
        let nats = [NatKind::Cone, NatKind::Symmetric];
        let first = run_online(2040, nats, HOP_LATENCY_TICKS).expect("online bout runs");
        let second = run_online(2040, nats, HOP_LATENCY_TICKS).expect("twin runs");
        assert!(first.passed(), "relay bout passes: {}", first.bout);
        assert_eq!(
            first.bout.digest(),
            second.bout.digest(),
            "relay-leg digest equals the one-process twin x2"
        );
        assert!(
            first.punch.decided_after <= PUNCH_TIMEOUT_TICKS,
            "punch-or-fallback inside 300 ticks, got {}",
            first.punch.decided_after
        );
        let rejoin = first
            .bout
            .rejoin_ticks()
            .expect("the scripted drop rejoins");
        assert!(
            rejoin <= CONVERGE_BAR_TICKS,
            "rejoin inside 300 ticks, got {rejoin}"
        );
    }
}
