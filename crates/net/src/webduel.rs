//! webduel: the scripted duel a page plays against the native dedicated server.
//!
//! Purpose: one real match, over the two browser legs. [`DuelHost`](crate::webduel::DuelHost) authors a
//! [`MatchSpec`](engine_arena::MatchSpec) duel (the window's rules v2 on the kit walls) with the
//! [`HeadlessServer`] and talks to any number of [`Transport`](crate::quic::Transport)s (the WebSocket
//! server, the WebTransport server, QUIC); [`DuelClient`](crate::webduel::DuelClient) is the page side: it
//! joins, predicts with [`Client`], plays the scripted tape and ends on the
//! server's hash. Both are plain state machines over the trait, so the same
//! code runs natively under test, in the `net_serve` binary and inside the
//! wasm page (the browser transports in [`crate::browser`]).
//!
//! Protocol: the client opens with a lobby `JoinRequest` on the reliable lane;
//! the host answers `JoinGrant` (or `JoinRefuse` with both builds) and then
//! waits, tick by tick, for the seat's input datagram (lockstep with a
//! caller-set patience, so a scripted duel is deterministic: the tape the
//! server authors is the tape the script holds). Each authored tick goes back
//! as a state datagram that repeats the last [`STATE_COVER`](crate::played::STATE_COVER) ticks of human
//! inputs. The last tick also rides the reliable lane as the whole tape, so a
//! lossy datagram path still ends on the same state. A seat that leaves is
//! AI-filled; the same token joining again reclaims its slot and is caught up
//! over the reliable lane.
//!
//! Units: fixed ticks at 60 Hz, bytes. Determinism: the script, the spec and
//! the sim are integer and fixed-point; no clock enters this module (the
//! caller owns patience and pacing), so the same spec and script give the same
//! hash on every machine, which is what the page and the native server compare.
//! Errors: [`DuelError`](crate::webduel::DuelError) for setup and server faults, [`DuelFailure`](crate::webduel::DuelFailure) for a
//! client that cannot finish, with the reason as text.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use engine_arena::{ArenaConfig, Checkpoint, MatchSpec};
use engine_input::fighter::{Buttons, ATTACK, BLOCK};

use crate::dedicated::{HeadlessServer, ServerError};
use crate::guard::SlotReceipt;
use crate::lobby::{BuildId, Msg, RefuseReason, LOBBY_MAGIC};
use crate::played::STATE_COVER;
use crate::quic::{send_input, send_lobby, ControlKind, Inbound, InboundFrame, PeerId, Transport};
use crate::session::{
    decode_state, encode_state, redundancy_window, resync_packets, BodyId, BodyInput, Client,
    NetError, Server, SnapshotDelta, REDUNDANCY, WIRE_MAGIC, WIRE_VERSION,
};

/// Match seed of the scripted duel (the game's canonical seed).
pub const DUEL_SEED: u64 = 2040;
/// Length of the scripted duel in ticks.
pub const DUEL_TICKS: u32 = 600;
/// Fighters in the duel: seat 0 is the page, the rest are AI.
pub const DUEL_FIGHTERS: u8 = 2;
/// Arena half-extent in meters.
pub const DUEL_EXTENT_M: f32 = 20.0;
/// Build version both ends present with the spec identity.
pub const DUEL_BUILD_VERSION: u16 = 1;
/// Ticks the client sends ahead of the newest state it holds.
pub const LEAD_TICKS: u32 = 8;
/// Ticks a seat may stay silent past the host's patience before it is dropped.
pub const STALL_LIMIT: u32 = 3;
/// Reliable packets the host sends a seat per round from its backlog.
pub const BACKLOG_PER_ROUND: usize = 16;
/// Rounds a finished client waits, with the tape no longer growing, for the closing history
/// before it calls a mismatch a desync.
pub const SETTLE_ROUNDS: u32 = 2_000;

/// Setup and server faults.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DuelError {
    /// The duel spec did not build (the embedded kit world is missing).
    Spec,
    /// The dedicated server refused.
    Server(ServerError),
    /// A codec or session call refused.
    Net(NetError),
}

impl fmt::Display for DuelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spec => f.write_str("the duel spec does not build"),
            Self::Server(error) => write!(f, "dedicated server: {error}"),
            Self::Net(error) => write!(f, "session: {error}"),
        }
    }
}

impl std::error::Error for DuelError {}

impl From<ServerError> for DuelError {
    fn from(error: ServerError) -> Self {
        Self::Server(error)
    }
}

impl From<NetError> for DuelError {
    fn from(error: NetError) -> Self {
        Self::Net(error)
    }
}

/// The match both ends play: rules v2 on the kit walls, 600 ticks, two fighters.
///
/// # Errors
///
/// [`DuelError::Spec`] when the embedded kit placement does not build a world.
pub fn duel_spec() -> Result<MatchSpec, DuelError> {
    duel_spec_for(DUEL_TICKS)
}

/// [`duel_spec`] with a match of `ticks` fixed ticks (a live seat plays longer than the
/// scripted proof). The length is part of the build identity, so a page and a server that
/// disagree on it are refused at the join with both builds named.
///
/// # Errors
///
/// [`DuelError::Spec`] when `ticks` is outside the arena's range or the kit world does not build.
pub fn duel_spec_for(ticks: u32) -> Result<MatchSpec, DuelError> {
    let config = ArenaConfig {
        seed: DUEL_SEED,
        ticks,
        fighters: DUEL_FIGHTERS,
        half_extent_m: DUEL_EXTENT_M,
        table_version: 1,
    };
    config.validate().map_err(|_| DuelError::Spec)?;
    MatchSpec::kit_duel(config).map_err(|_| DuelError::Spec)
}

/// The build identity of `spec`: refused at the join when the peers differ.
#[must_use]
pub fn duel_build(spec: &MatchSpec) -> BuildId {
    BuildId::of_spec(DUEL_BUILD_VERSION, spec)
}

/// The scripted input of `slot` at `tick`: walk in, strafe, back off, with an
/// attack every 30 ticks and a short block. A pure function of its arguments.
#[must_use]
pub fn scripted_input(tick: u32, slot: u8) -> BodyInput {
    let phase = tick % 120;
    let side: i8 = if (tick / 120).is_multiple_of(2) {
        60
    } else {
        -60
    };
    let (mx, mz) = match phase {
        0..=59 => (100, 0),
        60..=89 => (0, side),
        _ => (-40, 0),
    };
    let buttons = if tick % 30 == 29 {
        Buttons(ATTACK)
    } else if (90..100).contains(&phase) {
        Buttons(BLOCK)
    } else {
        Buttons::NONE
    };
    let turn = i16::try_from(i32::try_from(tick % 360).unwrap_or(0) * 100 - 18_000).unwrap_or(0);
    BodyInput {
        tick,
        // Slots above the participant cap never reach the wire; slot 0 is the fallback.
        fighter: BodyId::new(slot).unwrap_or(BodyId(0)),
        mx,
        mz,
        facing: turn,
        buttons,
    }
}

/// Where a seat's input for a tick comes from. The scripted proof uses [`Script`]; the page
/// plugs the keyboard in. Called once per tick, in tick order, when the client predicts it.
pub trait Pilot {
    /// The input of `slot` for `tick`.
    fn input(&mut self, tick: u32, slot: u8) -> BodyInput;
}

/// The pilot of the scripted duel: [`scripted_input`].
pub struct Script;

impl Pilot for Script {
    fn input(&mut self, tick: u32, slot: u8) -> BodyInput {
        scripted_input(tick, slot)
    }
}

/// The hash the duel ends on when `slot` plays the script without a stall:
/// the plain [`Server`] over the same spec and tape, no transport involved.
///
/// # Errors
///
/// [`DuelError`] when the spec or a tick is refused.
pub fn reference_hash(slot: u8) -> Result<u64, DuelError> {
    reference_hash_for(DUEL_TICKS, &[slot])
}

/// [`reference_hash`] for a match of `ticks` ticks in which every slot in `slots` plays
/// the script (two seats: a page and a native client).
///
/// # Errors
///
/// [`DuelError`] when the spec or a tick is refused.
pub fn reference_hash_for(ticks: u32, slots: &[u8]) -> Result<u64, DuelError> {
    let mut server = Server::from_spec(duel_spec_for(ticks)?)?;
    let mut hash = 0;
    for tick in 0..ticks {
        let inputs: Vec<BodyInput> = slots
            .iter()
            .map(|&slot| scripted_input(tick, slot))
            .collect();
        hash = server.tick(&inputs)?.0.hash;
    }
    Ok(hash)
}

/// Where a seat reaches the host: which transport and which peer on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeatLink {
    /// Index into the transport slice the host is driven with.
    pub via: usize,
    /// The peer on that transport.
    pub peer: PeerId,
}

/// Something the host did that an operator wants in the log.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HostEvent {
    /// A seat joined (or came back with `rejoin`).
    Joined {
        /// Slot granted.
        slot: u8,
        /// Where the seat is.
        link: SeatLink,
        /// True when the token already held the slot.
        rejoin: bool,
    },
    /// A join was refused.
    Refused {
        /// Who asked.
        link: SeatLink,
        /// Why.
        reason: RefuseReason,
    },
    /// A seat left and is AI-filled from now on.
    Dropped {
        /// Slot that left.
        slot: u8,
        /// The reason, as text.
        why: String,
    },
    /// A tick was authored without a seat's input.
    Stalled {
        /// Seat that was silent.
        slot: u8,
        /// Tick authored without it.
        tick: u32,
    },
}

struct Seat {
    token: u64,
    slot: u8,
    link: SeatLink,
    live: bool,
    quiet: u32,
    /// Reliable packets still owed to this seat (a rejoin's catch-up, the closing history),
    /// sent a few per round so a long match never overruns the transport's reliable queue.
    backlog: VecDeque<Vec<u8>>,
}

/// The dedicated server's side of the duel over any number of transports.
pub struct DuelHost {
    server: HeadlessServer,
    build: BuildId,
    seats: Vec<Seat>,
    /// Seats that must have joined before the first tick is authored.
    need: usize,
    inputs: BTreeMap<(u32, u8), BodyInput>,
    tip: Option<Vec<u8>>,
    events: Vec<HostEvent>,
    started: bool,
}

impl DuelHost {
    /// A host for `spec` at tick zero with no seat; the match starts with the first join.
    ///
    /// # Errors
    ///
    /// [`DuelError::Server`] when the spec does not build a match.
    pub fn new(spec: MatchSpec) -> Result<Self, DuelError> {
        Self::with_seats(spec, 1)
    }

    /// A host that starts the match once `seats` different tokens have joined (a human
    /// duel waits for the second human; an empty seat is AI-filled only after the start).
    ///
    /// # Errors
    ///
    /// [`DuelError::Server`] when the spec does not build a match or `seats` is zero or
    /// more than the spec has fighters.
    pub fn with_seats(spec: MatchSpec, seats: u8) -> Result<Self, DuelError> {
        if seats == 0 || seats > spec.config.fighters {
            return Err(DuelError::Server(ServerError::BadInput));
        }
        let build = duel_build(&spec);
        Ok(Self {
            server: HeadlessServer::from_spec(spec)?,
            build,
            seats: Vec::new(),
            need: usize::from(seats),
            inputs: BTreeMap::new(),
            tip: None,
            events: Vec::new(),
            started: false,
        })
    }

    /// The build identity a joining peer must present.
    #[must_use]
    pub fn build(&self) -> BuildId {
        self.build
    }

    /// Next tick to author.
    #[must_use]
    pub fn tick_now(&self) -> u32 {
        self.server.tick_now()
    }

    /// True once the last tick is authored.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.server.is_finished()
    }

    /// True once any seat has joined: the match starts with the first one.
    #[must_use]
    pub fn started(&self) -> bool {
        self.started
    }

    /// Seats currently reachable.
    #[must_use]
    pub fn live_seats(&self) -> usize {
        self.seats.iter().filter(|seat| seat.live).count()
    }

    /// The authoritative hash of the last authored tick.
    #[must_use]
    pub fn last_hash(&self) -> Option<u64> {
        self.server.checkpoints().last().map(|cp| cp.hash)
    }

    /// The hash the finished match ended on.
    #[must_use]
    pub fn final_hash(&self) -> Option<u64> {
        self.is_finished().then(|| self.last_hash()).flatten()
    }

    /// Take the events since the last call.
    pub fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.events)
    }

    /// Read every transport once: joins and inputs. Frames from peers that
    /// hold no seat are ignored, malformed frames are dropped.
    pub fn pump(&mut self, transports: &mut [&mut dyn Transport]) {
        for (via, transport) in transports.iter_mut().enumerate() {
            for Inbound { from, frame } in transport.recv() {
                let link = SeatLink { via, peer: from };
                match frame {
                    InboundFrame::Stream {
                        kind: ControlKind::Lobby,
                        bytes,
                    } => self.on_lobby(link, &bytes, &mut **transport),
                    InboundFrame::Datagram(bytes) => self.on_input(link, &bytes),
                    InboundFrame::Stream { .. } => {}
                }
            }
        }
        self.flush_backlogs(transports);
    }

    /// Send each live seat the next few packets it is owed. A refused send (the transport's
    /// reliable queue is full) keeps the packet for the next round.
    fn flush_backlogs(&mut self, transports: &mut [&mut dyn Transport]) {
        for seat in self.seats.iter_mut().filter(|seat| seat.live) {
            let Some(transport) = transports.get_mut(seat.link.via) else {
                continue;
            };
            for _ in 0..BACKLOG_PER_ROUND {
                let Some(packet) = seat.backlog.front() else {
                    break;
                };
                if transport
                    .send_reliable(seat.link.peer, ControlKind::Lobby, packet)
                    .is_err()
                {
                    break;
                }
                seat.backlog.pop_front();
            }
        }
    }

    fn on_lobby(&mut self, link: SeatLink, bytes: &[u8], transport: &mut dyn Transport) {
        if u16::from_le_bytes([
            bytes.first().copied().unwrap_or(0),
            bytes.get(1).copied().unwrap_or(0),
        ]) != LOBBY_MAGIC
        {
            return;
        }
        let (token, build) = match Msg::decode(bytes) {
            Ok(Msg::JoinRequest { token, build }) => (token, build),
            // A seat's Ready is its ping: the host echoes it so the page can time the round trip.
            Ok(Msg::Ready { token, ready }) => {
                if self
                    .seats
                    .iter()
                    .any(|seat| seat.token == token && seat.link == link)
                {
                    // A lost echo is a lost ping sample.
                    let _ = send_lobby(transport, link.peer, Msg::Ready { token, ready });
                }
                return;
            }
            _ => return,
        };
        let refuse = |host: &mut Self, transport: &mut dyn Transport, reason| {
            host.events.push(HostEvent::Refused { link, reason });
            // A refusal that cannot be sent leaves the peer to time out.
            let _ = send_lobby(
                transport,
                link.peer,
                Msg::JoinRefuse {
                    token,
                    reason,
                    want: host.build,
                    got: build,
                },
            );
        };
        if build != self.build {
            refuse(self, transport, RefuseReason::VersionMismatch);
            return;
        }
        let rejoin = self.server.slot_of(token).is_some();
        let slot = match self.server.connect(token) {
            Ok(slot) => slot,
            Err(ServerError::SlotFull) => {
                refuse(self, transport, RefuseReason::SlotFull);
                return;
            }
            Err(_) => {
                refuse(self, transport, RefuseReason::BadInput);
                return;
            }
        };
        match self.seats.iter_mut().find(|seat| seat.token == token) {
            Some(seat) => {
                seat.link = link;
                seat.live = true;
                seat.quiet = 0;
            }
            None => self.seats.push(Seat {
                token,
                slot,
                link,
                live: true,
                quiet: 0,
                backlog: VecDeque::new(),
            }),
        }
        self.started = self.started || self.seats.len() >= self.need;
        self.events.push(HostEvent::Joined { slot, link, rejoin });
        let grant = Msg::JoinGrant {
            token,
            slot,
            fighters: DUEL_FIGHTERS,
            build: self.build,
        };
        if send_lobby(transport, link.peer, grant).is_err() {
            return;
        }
        // A seat that comes back mid-match is caught up over the reliable lane.
        let Some(snapshot) = self.server.snapshot() else {
            return;
        };
        if let Ok(packets) = resync_packets(&snapshot, self.server.checkpoints()) {
            if let Some(seat) = self.seats.iter_mut().find(|seat| seat.token == token) {
                seat.backlog = packets.into();
            }
        }
    }

    /// A seat's input datagram enters the server through its guard: malformed packets, other
    /// slots' inputs, far-future, stale, rewritten and flooded inputs are refused and counted
    /// per slot ([`DuelHost::admission`]); the rest wait for their tick.
    fn on_input(&mut self, link: SeatLink, bytes: &[u8]) {
        let Some(seat) = self
            .seats
            .iter()
            .find(|seat| seat.live && seat.link == link)
        else {
            return;
        };
        let now = self.server.tick_now();
        let Ok(admitted) = self.server.offer(seat.token, bytes, now) else {
            return;
        };
        for input in admitted {
            self.inputs
                .entry((input.tick, input.fighter.0))
                .or_insert(input);
        }
    }

    /// What the server's input guard did with each slot's traffic: packets, inputs admitted,
    /// and every refusal by kind.
    #[must_use]
    pub fn admission(&self) -> &[SlotReceipt] {
        self.server.admission()
    }

    /// A seat's transport reported it gone: AI-fill it from now on.
    pub fn depart(&mut self, link: SeatLink, why: &str) {
        let Some(seat) = self
            .seats
            .iter_mut()
            .find(|seat| seat.live && seat.link == link)
        else {
            return;
        };
        seat.live = false;
        let (token, slot) = (seat.token, seat.slot);
        self.server.disconnect(token);
        self.events.push(HostEvent::Dropped {
            slot,
            why: why.to_owned(),
        });
    }

    /// Whether a live seat still owes the input of the next tick.
    #[must_use]
    pub fn blocked(&self) -> bool {
        let now = self.server.tick_now();
        self.seats
            .iter()
            .any(|seat| seat.live && !self.inputs.contains_key(&(now, seat.slot)))
    }

    /// Author the next tick when every live seat has sent its input, or when
    /// `give_up` says patience ran out (the silent seats are AI-filled for that
    /// tick and dropped after [`STALL_LIMIT`] in a row). Before the first join
    /// and after the last tick nothing is authored. Returns the checkpoint.
    ///
    /// # Errors
    ///
    /// [`DuelError`] when the server or the codec refuses the tick.
    pub fn tick(
        &mut self,
        transports: &mut [&mut dyn Transport],
        give_up: bool,
    ) -> Result<Option<Checkpoint>, DuelError> {
        if !self.started || self.is_finished() {
            return Ok(None);
        }
        let tick = self.server.tick_now();
        if self.blocked() {
            if !give_up {
                return Ok(None);
            }
            self.note_silent_seats(tick);
        }
        for seat in &mut self.seats {
            if !seat.live {
                continue;
            }
            if let Some(input) = self.inputs.remove(&(tick, seat.slot)) {
                // A refused input (stale stamp) is a lost input: the seat is AI-filled that tick.
                if self.server.submit(seat.token, input).is_ok() {
                    seat.quiet = 0;
                }
            }
        }
        self.inputs.retain(|&(at, _), _| at > tick);
        let (checkpoint, delta) = self.server.tick()?;
        let window = redundancy_window(&delta.tape, checkpoint.tick, STATE_COVER);
        let bytes = encode_state(checkpoint, &window)?;
        self.send_all(transports, &bytes);
        self.tip = Some(bytes);
        if self.is_finished() {
            self.queue_history(&delta);
        }
        Ok(Some(checkpoint))
    }

    /// Count the seats that owe the input of `tick`, and drop the ones that
    /// have been silent [`STALL_LIMIT`] ticks in a row.
    fn note_silent_seats(&mut self, tick: u32) {
        for index in 0..self.seats.len() {
            let seat = &mut self.seats[index];
            if !seat.live || self.inputs.contains_key(&(tick, seat.slot)) {
                continue;
            }
            self.events.push(HostEvent::Stalled {
                slot: seat.slot,
                tick,
            });
            seat.quiet += 1;
            if seat.quiet >= STALL_LIMIT {
                let link = seat.link;
                self.depart(link, "no input within the host's patience");
            }
        }
    }

    /// Send the newest state to every live seat again (the closing drain).
    pub fn resend_tip(&self, transports: &mut [&mut dyn Transport]) {
        if let Some(bytes) = &self.tip {
            self.send_all(transports, bytes);
        }
    }

    fn send_all(&self, transports: &mut [&mut dyn Transport], bytes: &[u8]) {
        for seat in self.seats.iter().filter(|seat| seat.live) {
            if let Some(transport) = transports.get_mut(seat.link.via) {
                // A datagram that cannot be sent is loss; the cover repeats it.
                let _ = transport.send_unreliable(seat.link.peer, bytes);
            }
        }
    }

    /// The whole authoritative tape on the reliable lane, so a lossy datagram
    /// path still ends on the server's state.
    fn queue_history(&mut self, delta: &SnapshotDelta) {
        let Ok(packets) = resync_packets(delta, self.server.checkpoints()) else {
            return;
        };
        for seat in self.seats.iter_mut().filter(|seat| seat.live) {
            seat.backlog = packets.iter().cloned().collect();
        }
    }
}

/// What a finished duel looked like from the client.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DuelOutcome {
    /// Ticks played: the checkpoints the client holds.
    pub ticks: u32,
    /// The server's hash of the last tick, which the client's own sim equals.
    pub hash: u64,
    /// Peers the client talked to (the server).
    pub peers: usize,
    /// Slot the server granted.
    pub slot: u8,
    /// Ticks where the prediction differed from the server and was replayed.
    pub mispredicts: u64,
    /// True when the end needed the full-tape rebuild (datagrams were lost).
    pub rebuilt: bool,
    /// Deepest rollback of the match: ticks replayed after the server disagreed.
    pub rollback_max: u32,
    /// The last ping round trip, when one was measured.
    pub rtt_ms: Option<u32>,
}

/// Why the client could not finish.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DuelFailure(pub String);

impl fmt::Display for DuelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DuelFailure {}

enum Phase {
    Start,
    Joining,
    Playing(Box<Client>),
    Done(Result<DuelOutcome, DuelFailure>),
}

/// How often a seat pings the host, and how long it waits for the echo.
const PING_EVERY_MS: u64 = 1_000;
const PING_LOST_MS: u64 = 3_000;
/// How long a join may go unanswered before it is sent again (a lossy path).
const JOIN_RETRY_MS: u64 = 1_000;

/// How the session is going right now: what the page prints while the duel runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DuelStats {
    /// Next tick this client predicts.
    pub tick: u32,
    /// Newest authoritative tick held, once a state arrived.
    pub server_tick: Option<u32>,
    /// The last ping round trip (the join handshake counts as the first).
    pub rtt_ms: Option<u32>,
    /// Ticks replayed by the newest rollback (zero when the prediction held).
    pub rollback_last: u32,
    /// Deepest rollback so far.
    pub rollback_max: u32,
    /// Predicted ticks the server corrected.
    pub mispredicts: u64,
    /// Milliseconds since the newest state arrived (zero before the first, or unclocked).
    pub quiet_ms: u64,
    /// State packets that did not decode.
    pub rejected: u32,
}

/// The server's hash of each tick whose state reached this client.
#[derive(Default)]
struct Attested {
    hashes: BTreeMap<u32, u64>,
    /// Ticks `0..contiguous` all have a hash, so every input of them reached the client too.
    contiguous: u32,
    /// Ticks below this already agree with the client's own run.
    checked: u32,
}

impl Attested {
    fn note(&mut self, checkpoint: Checkpoint) {
        self.hashes.insert(checkpoint.tick, checkpoint.hash);
        while self.hashes.contains_key(&self.contiguous) {
            self.contiguous += 1;
        }
    }

    /// The first tick of `held` whose hash is not the server's.
    fn first_divergence(&self, held: &[Checkpoint]) -> Option<u32> {
        held.iter()
            .find(|cp| {
                self.hashes
                    .get(&cp.tick)
                    .is_some_and(|&server| server != cp.hash)
            })
            .map(|cp| cp.tick)
    }
}

/// The page's side of the duel.
pub struct DuelClient {
    spec: MatchSpec,
    ticks: u32,
    build: BuildId,
    token: u64,
    slot: u8,
    phase: Phase,
    pilot: Box<dyn Pilot>,
    known: BTreeMap<(u32, u8), BodyInput>,
    sent: Vec<BodyInput>,
    top: Option<Checkpoint>,
    attested: Attested,
    mispredicts: u64,
    rollback_last: u32,
    rollback_max: u32,
    rejected: u32,
    /// The caller's clock, set by [`DuelClient::poll_at`]; `None` for an unclocked client.
    now_ms: Option<u64>,
    last_state_ms: Option<u64>,
    join_sent_ms: Option<u64>,
    ping_sent_ms: Option<u64>,
    ping_next_ms: u64,
    rtt_ms: Option<u32>,
    /// Inputs known when the end last looked wrong, and the rounds since the tape grew.
    settled_at: usize,
    settle: u32,
}

impl DuelClient {
    /// A client for the scripted duel with a nonzero seat `token` (the same token
    /// reclaims the same seat).
    ///
    /// # Errors
    ///
    /// [`DuelError`] for a zero token or a spec that does not build.
    pub fn new(token: u64) -> Result<Self, DuelError> {
        Self::playing(token, DUEL_TICKS, Box::new(Script))
    }

    /// A client whose inputs come from `pilot`, in a match of `ticks` ticks (the server
    /// must serve the same length, or the join is refused).
    ///
    /// # Errors
    ///
    /// [`DuelError`] for a zero token or a length the arena does not allow.
    pub fn playing(token: u64, ticks: u32, pilot: Box<dyn Pilot>) -> Result<Self, DuelError> {
        if token == 0 {
            return Err(DuelError::Net(NetError::BadInput));
        }
        let mut client = Self::with_spec(token, duel_spec_for(ticks)?);
        client.pilot = pilot;
        Ok(client)
    }

    /// [`DuelClient::new`] on a spec of the caller's (a build mismatch proof).
    #[must_use]
    pub fn with_spec(token: u64, spec: MatchSpec) -> Self {
        Self {
            build: duel_build(&spec),
            ticks: spec.config.ticks,
            spec,
            token,
            slot: 0,
            phase: Phase::Start,
            pilot: Box::new(Script),
            known: BTreeMap::new(),
            sent: Vec::new(),
            top: None,
            attested: Attested::default(),
            mispredicts: 0,
            rollback_last: 0,
            rollback_max: 0,
            rejected: 0,
            now_ms: None,
            last_state_ms: None,
            join_sent_ms: None,
            ping_sent_ms: None,
            ping_next_ms: 0,
            rtt_ms: None,
            settled_at: 0,
            settle: 0,
        }
    }

    /// The verdict once the duel ended, either way.
    #[must_use]
    pub fn outcome(&self) -> Option<&Result<DuelOutcome, DuelFailure>> {
        match &self.phase {
            Phase::Done(result) => Some(result),
            _ => None,
        }
    }

    /// End the duel with a reason of the caller's (the transport closed, a deadline passed).
    pub fn fail(&mut self, reason: impl Into<String>) {
        if !matches!(self.phase, Phase::Done(_)) {
            self.phase = Phase::Done(Err(DuelFailure(reason.into())));
        }
    }

    /// Ticks the client has authority for so far.
    #[must_use]
    pub fn server_tick(&self) -> Option<u32> {
        match &self.phase {
            Phase::Playing(client) => client.server_tick(),
            _ => None,
        }
    }

    /// Length of the match this client plays, in ticks.
    #[must_use]
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// True once the server granted a seat.
    #[must_use]
    pub fn seated(&self) -> bool {
        matches!(self.phase, Phase::Playing(_))
    }

    /// How the session is going at the caller's last clock reading.
    #[must_use]
    pub fn stats(&self) -> DuelStats {
        let (tick, server_tick) = match &self.phase {
            Phase::Playing(client) => (client.tick_now(), client.server_tick()),
            _ => (0, None),
        };
        DuelStats {
            tick,
            server_tick,
            rtt_ms: self.rtt_ms,
            rollback_last: self.rollback_last,
            rollback_max: self.rollback_max,
            mispredicts: self.mispredicts,
            quiet_ms: match (self.now_ms, self.last_state_ms) {
                (Some(now), Some(last)) => now.saturating_sub(last),
                _ => 0,
            },
            rejected: self.rejected,
        }
    }

    /// Run one round without a clock: no ping, no join retry (the in-memory tests).
    pub fn poll<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) -> bool {
        self.round(transport, server)
    }

    /// Run one round at `now_ms` on the caller's clock: the same as [`DuelClient::poll`]
    /// plus the ping, the join retry and the quiet time of [`DuelStats`].
    pub fn poll_at<T: Transport + ?Sized>(
        &mut self,
        transport: &mut T,
        server: PeerId,
        now_ms: u64,
    ) -> bool {
        self.now_ms = Some(now_ms);
        self.round(transport, server)
    }

    /// One round: send the join, read the transport, apply the newest state,
    /// check it against the server's hashes, send the next inputs. Returns true
    /// once the duel ended.
    fn round<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) -> bool {
        self.join(transport, server);
        for inbound in transport.recv() {
            if inbound.from != server {
                continue;
            }
            let bytes = match inbound.frame {
                InboundFrame::Datagram(bytes) | InboundFrame::Stream { bytes, .. } => bytes,
            };
            self.on_bytes(&bytes);
        }
        self.apply();
        self.verify();
        self.ping(transport, server);
        self.send_inputs(transport, server);
        self.finish();
        matches!(self.phase, Phase::Done(_))
    }

    /// Send the join, and again when a clocked client has heard nothing for a second.
    fn join<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) {
        let overdue = matches!((self.now_ms, self.join_sent_ms), (Some(now), Some(sent))
            if now.saturating_sub(sent) >= JOIN_RETRY_MS);
        if !(matches!(self.phase, Phase::Start) || matches!(self.phase, Phase::Joining) && overdue)
        {
            return;
        }
        let join = Msg::JoinRequest {
            token: self.token,
            build: self.build,
        };
        match send_lobby(transport, server, join) {
            Ok(()) => {
                self.phase = Phase::Joining;
                self.join_sent_ms = self.now_ms;
            }
            Err(error) => self.fail(format!("the join could not be sent: {error}")),
        }
    }

    /// Send a Ready ping when one is due; the host echoes it and [`DuelClient::on_lobby`] times it.
    fn ping<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) {
        let (Phase::Playing(_), Some(now)) = (&self.phase, self.now_ms) else {
            return;
        };
        if self
            .ping_sent_ms
            .is_some_and(|sent| now.saturating_sub(sent) >= PING_LOST_MS)
        {
            self.ping_sent_ms = None;
        }
        if self.ping_sent_ms.is_some() || now < self.ping_next_ms {
            return;
        }
        let ping = Msg::Ready {
            token: self.token,
            ready: true,
        };
        if send_lobby(transport, server, ping).is_ok() {
            self.ping_sent_ms = Some(now);
            self.ping_next_ms = now + PING_EVERY_MS;
        }
    }

    fn take_rtt(&mut self, sent: Option<u64>) {
        if let (Some(now), Some(sent)) = (self.now_ms, sent) {
            self.rtt_ms = Some(u32::try_from(now.saturating_sub(sent)).unwrap_or(u32::MAX));
        }
    }

    fn on_bytes(&mut self, bytes: &[u8]) {
        let magic = match bytes {
            [a, b, ..] => u16::from_le_bytes([*a, *b]),
            _ => return,
        };
        if magic == LOBBY_MAGIC {
            self.on_lobby(bytes);
        } else if magic == WIRE_MAGIC {
            self.on_state(bytes);
        }
    }

    fn on_lobby(&mut self, bytes: &[u8]) {
        match Msg::decode(bytes) {
            Ok(Msg::JoinGrant { token, slot, .. })
                if token == self.token && matches!(self.phase, Phase::Joining) =>
            {
                match Client::from_spec(self.spec.clone(), BodyId(slot)) {
                    Ok(client) => {
                        self.slot = slot;
                        self.phase = Phase::Playing(Box::new(client));
                        // The handshake is the first round trip.
                        self.take_rtt(self.join_sent_ms);
                    }
                    Err(error) => self.fail(format!("the granted seat does not build: {error}")),
                }
            }
            Ok(Msg::JoinRefuse {
                token,
                reason,
                want,
                got,
            }) if token == self.token => {
                self.fail(format!(
                    "the server refused the join ({reason:?}): its build is v{} rules {:016x}, ours v{} rules {:016x}",
                    want.version, want.rules, got.version, got.rules
                ));
            }
            Ok(Msg::Ready { token, .. }) if token == self.token => {
                let sent = self.ping_sent_ms.take();
                self.take_rtt(sent);
            }
            _ => {}
        }
    }

    fn on_state(&mut self, bytes: &[u8]) {
        let Ok(packet) = decode_state(bytes) else {
            self.rejected += 1;
            return;
        };
        self.last_state_ms = self.now_ms;
        self.attested.note(packet.checkpoint);
        for input in packet.inputs {
            self.known.insert((input.tick, input.fighter.0), input);
        }
        if self.top.is_none_or(|top| packet.checkpoint.tick > top.tick) {
            self.top = Some(packet.checkpoint);
        }
    }

    /// Merge the newest state into the client and replay what it disagrees with.
    fn apply(&mut self) {
        let Phase::Playing(client) = &mut self.phase else {
            return;
        };
        let Some(checkpoint) = self.top else {
            return;
        };
        if client
            .server_tick()
            .is_some_and(|done| checkpoint.tick <= done)
        {
            return;
        }
        let tape: Vec<BodyInput> = self
            .known
            .range(..=(checkpoint.tick, u8::MAX))
            .map(|(_, input)| *input)
            .collect();
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: checkpoint.tick,
            hash: checkpoint.hash,
            tape,
        };
        match client.on_server_state(checkpoint, &delta) {
            Ok(mispredicts) => {
                self.mispredicts += u64::try_from(mispredicts.len()).unwrap_or(u64::MAX);
                // The replay reaches from the first corrected tick to the newest prediction.
                self.rollback_last = mispredicts
                    .first()
                    .map_or(0, |&first| client.tick_now().saturating_sub(first));
                self.rollback_max = self.rollback_max.max(self.rollback_last);
                client.resimulate();
            }
            Err(_) => self.rejected += 1,
        }
    }

    /// Check the client's own run against the hashes the server attested while the duel
    /// is on, so a desync is named when it happens and not at the last tick. A tape with a
    /// hole (late datagrams) looks like a divergence until the whole known tape is
    /// replayed, so only a divergence the replay keeps ends the duel.
    fn verify(&mut self) {
        let Phase::Playing(client) = &self.phase else {
            return;
        };
        let upto = client
            .server_tick()
            .map_or(0, |tick| tick + 1)
            .min(self.attested.contiguous);
        let from = self.attested.checked;
        let disagreed = (from..upto).any(|tick| {
            let held = usize::try_from(tick)
                .ok()
                .and_then(|index| client.checkpoints().get(index));
            held.map(|cp| cp.hash) != self.attested.hashes.get(&tick).copied()
        });
        self.attested.checked = from.max(upto);
        if !disagreed {
            return;
        }
        let tip = upto.checked_sub(1).and_then(|tick| {
            self.attested
                .hashes
                .get(&tick)
                .map(|&hash| Checkpoint { tick, hash })
        });
        let Some(replay) = tip.and_then(|tip| self.replay_to(tip)) else {
            return;
        };
        if let Some(first) = self.attested.first_divergence(&replay) {
            self.fail(self.desync_text(first, &replay));
        }
    }

    /// The desync message: the first tick the two runs part at and both hashes.
    fn desync_text(&self, first: u32, replay: &[Checkpoint]) -> String {
        let held = usize::try_from(first)
            .ok()
            .and_then(|index| replay.get(index));
        format!(
            "desync at tick {first}: the server says {:016x}, this client computed {}",
            self.attested.hashes.get(&first).copied().unwrap_or(0),
            held.map_or_else(|| "nothing".to_owned(), |cp| format!("{:016x}", cp.hash)),
        )
    }

    /// Predict and send every input up to the lead past the newest state.
    fn send_inputs<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) {
        let Phase::Playing(client) = &mut self.phase else {
            return;
        };
        let limit = client
            .server_tick()
            .map_or(LEAD_TICKS, |tick| tick.saturating_add(1 + LEAD_TICKS))
            .min(self.ticks - 1);
        let mut failure = None;
        while client.tick_now() <= limit {
            let input = self.pilot.input(client.tick_now(), self.slot);
            if let Err(error) = client.predict(input) {
                failure = Some(format!(
                    "the prediction refused tick {}: {error}",
                    input.tick
                ));
                break;
            }
            // A tick the server already authored needs no input (a returning seat predicts
            // its way back to the tip without flooding the server's guard with old ones).
            if client.server_tick().is_some_and(|done| input.tick <= done) {
                continue;
            }
            self.sent.push(input);
            let from = self.sent.len().saturating_sub(REDUNDANCY);
            if let Err(error) = send_input(transport, server, input.tick, &self.sent[from..]) {
                failure = Some(format!(
                    "the input of tick {} could not be sent: {error}",
                    input.tick
                ));
                break;
            }
        }
        if let Some(reason) = failure {
            self.fail(reason);
        }
    }

    /// End the duel when the last authoritative tick is in.
    fn finish(&mut self) {
        let Phase::Playing(client) = &self.phase else {
            return;
        };
        let end = self.ticks - 1;
        if client.server_tick() != Some(end) {
            return;
        }
        let Some(last) = self.top.filter(|cp| cp.tick == end) else {
            return;
        };
        let mut rebuilt = false;
        let mut diverged = None;
        let mut held = client
            .checkpoints()
            .last()
            .copied()
            .filter(|cp| cp.tick == end);
        if held.map(|cp| cp.hash) != Some(last.hash) {
            // Datagrams went missing: replay over everything the server ever attested,
            // the way a rejoin does. The closing history may still be arriving, so a
            // replay that still disagrees waits for the tape to stop growing before it
            // is called a desync, and replays again only when the tape grew.
            rebuilt = true;
            let grew = self.known.len() != self.settled_at;
            if !grew && self.settle < SETTLE_ROUNDS {
                self.settle += 1;
                return;
            }
            let replay = self.replay_to(last);
            diverged = replay
                .as_deref()
                .and_then(|cps| self.attested.first_divergence(cps));
            held = replay
                .and_then(|cps| cps.last().copied())
                .filter(|cp| cp.tick == end);
            if held.map(|cp| cp.hash) != Some(last.hash) && self.settle < SETTLE_ROUNDS {
                self.settled_at = self.known.len();
                self.settle = 0;
                return;
            }
        }
        let result = match held {
            Some(cp) if cp.hash == last.hash => Ok(DuelOutcome {
                ticks: self.ticks,
                hash: last.hash,
                peers: 1,
                slot: self.slot,
                mispredicts: self.mispredicts,
                rebuilt,
                rollback_max: self.rollback_max,
                rtt_ms: self.rtt_ms,
            }),
            other => Err(DuelFailure(format!(
                "desync at tick {}: the server says {:016x}, this client computed {}{}",
                last.tick,
                last.hash,
                other.map_or_else(|| "nothing".to_owned(), |cp| format!("{:016x}", cp.hash)),
                diverged.map_or_else(String::new, |tick| format!(
                    " (first divergent tick {tick})"
                )),
            ))),
        };
        self.phase = Phase::Done(result);
    }

    /// The checkpoints of a fresh client fed the whole known tape up to `last`.
    fn replay_to(&self, last: Checkpoint) -> Option<Vec<Checkpoint>> {
        let mut fresh = Client::from_spec(self.spec.clone(), BodyId(self.slot)).ok()?;
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: last.tick,
            hash: last.hash,
            tape: self
                .known
                .range(..=(last.tick, u8::MAX))
                .map(|(_, input)| *input)
                .collect(),
        };
        fresh.on_server_state(last, &delta).ok()?;
        Some(fresh.resimulate())
    }
}

/// The one line the page prints for an outcome. `transport` is the leg that won
/// the connect race.
#[must_use]
pub fn net_line(transport: &str, outcome: &Result<DuelOutcome, DuelFailure>) -> String {
    match outcome {
        Ok(done) => format!(
            "net: transport={transport} ticks={} hash={:016x} peers={}",
            done.ticks, done.hash, done.peers
        ),
        Err(failure) => format!("net: failed transport={transport} reason={failure}"),
    }
}

/// The line the page prints while the duel runs: the leg, the tick against the match length,
/// the ping, the rollback depth (newest and deepest) and how long the newest state is old.
#[must_use]
pub fn playing_line(transport: &str, stats: &DuelStats, ticks: u32) -> String {
    let rtt = stats
        .rtt_ms
        .map_or_else(|| "?".to_owned(), |ms| format!("{ms}ms"));
    match stats.server_tick {
        None => format!("net: seated transport={transport} rtt={rtt}, waiting for the match to start"),
        Some(tick) => format!(
            "net: playing transport={transport} tick={}/{ticks} rtt={rtt} rollback={} max={} quiet={}ms",
            tick + 1,
            stats.rollback_last,
            stats.rollback_max,
            stats.quiet_ms,
        ),
    }
}

#[cfg(test)]
mod tests;
