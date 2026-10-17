//! webduel: the scripted duel a page plays against the native dedicated server.
//!
//! Purpose: one real match, over the two browser legs. [`DuelHost`] authors a
//! [`MatchSpec`] duel (the window's rules v2 on the kit walls) with the
//! [`HeadlessServer`] and talks to any number of [`Transport`]s (the WebSocket
//! server, the WebTransport server, QUIC); [`DuelClient`] is the page side: it
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
//! as a state datagram that repeats the last [`STATE_COVER`] ticks of human
//! inputs. The last tick also rides the reliable lane as the whole tape, so a
//! lossy datagram path still ends on the same state. A seat that leaves is
//! AI-filled; the same token joining again reclaims its slot and is caught up
//! over the reliable lane.
//!
//! Units: fixed ticks at 60 Hz, bytes. Determinism: the script, the spec and
//! the sim are integer and fixed-point; no clock enters this module (the
//! caller owns patience and pacing), so the same spec and script give the same
//! hash on every machine, which is what the page and the native server compare.
//! Errors: [`DuelError`] for setup and server faults, [`DuelFailure`] for a
//! client that cannot finish, with the reason as text.

use std::collections::BTreeMap;
use std::fmt;

use engine_arena::{ArenaConfig, Checkpoint, MatchSpec};
use engine_input::fighter::{Buttons, FighterId, ATTACK, BLOCK};
use engine_input::FighterInput;

use crate::dedicated::{HeadlessServer, ServerError};
use crate::lobby::{BuildId, Msg, RefuseReason, LOBBY_MAGIC};
use crate::played::STATE_COVER;
use crate::quic::{send_input, send_lobby, ControlKind, Inbound, InboundFrame, PeerId, Transport};
use crate::session::{
    decode_input, decode_state, encode_state, redundancy_window, resync_packets, Client, NetError,
    Server, SnapshotDelta, REDUNDANCY, WIRE_MAGIC, WIRE_VERSION,
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
/// Inputs for ticks this far past the server cursor are refused (flood guard).
pub const MAX_AHEAD_TICKS: u32 = 64;
/// Ticks a seat may stay silent past the host's patience before it is dropped.
pub const STALL_LIMIT: u32 = 3;

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
    MatchSpec::kit_duel(ArenaConfig {
        seed: DUEL_SEED,
        ticks: DUEL_TICKS,
        fighters: DUEL_FIGHTERS,
        half_extent_m: DUEL_EXTENT_M,
        table_version: 1,
    })
    .map_err(|_| DuelError::Spec)
}

/// The build identity of `spec`: refused at the join when the peers differ.
#[must_use]
pub fn duel_build(spec: &MatchSpec) -> BuildId {
    BuildId::of_spec(DUEL_BUILD_VERSION, spec)
}

/// The scripted input of `slot` at `tick`: walk in, strafe, back off, with an
/// attack every 30 ticks and a short block. A pure function of its arguments.
#[must_use]
pub fn scripted_input(tick: u32, slot: u8) -> FighterInput {
    let phase = tick % 120;
    let side: i8 = if (tick / 120).is_multiple_of(2) { 60 } else { -60 };
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
    FighterInput {
        tick,
        // Slots above the participant cap never reach the wire; slot 0 is the fallback.
        fighter: FighterId::new(slot).unwrap_or(FighterId(0)),
        mx,
        mz,
        facing: turn,
        buttons,
    }
}

/// The hash the duel ends on when `slot` plays the script without a stall:
/// the plain [`Server`] over the same spec and tape, no transport involved.
///
/// # Errors
///
/// [`DuelError`] when the spec or a tick is refused.
pub fn reference_hash(slot: u8) -> Result<u64, DuelError> {
    let mut server = Server::from_spec(duel_spec()?)?;
    let mut hash = 0;
    for tick in 0..DUEL_TICKS {
        hash = server.tick(&[scripted_input(tick, slot)])?.0.hash;
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
}

/// The dedicated server's side of the duel over any number of transports.
pub struct DuelHost {
    server: HeadlessServer,
    build: BuildId,
    seats: Vec<Seat>,
    inputs: BTreeMap<(u32, u8), FighterInput>,
    tip: Option<Vec<u8>>,
    events: Vec<HostEvent>,
    started: bool,
}

impl DuelHost {
    /// A host for `spec` at tick zero with no seat.
    ///
    /// # Errors
    ///
    /// [`DuelError::Server`] when the spec does not build a match.
    pub fn new(spec: MatchSpec) -> Result<Self, DuelError> {
        let build = duel_build(&spec);
        Ok(Self {
            server: HeadlessServer::from_spec(spec)?,
            build,
            seats: Vec::new(),
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
    }

    fn on_lobby(&mut self, link: SeatLink, bytes: &[u8], transport: &mut dyn Transport) {
        if u16::from_le_bytes([bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0)])
            != LOBBY_MAGIC
        {
            return;
        }
        let Ok(Msg::JoinRequest { token, build }) = Msg::decode(bytes) else {
            return;
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
            }),
        }
        self.started = true;
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
        if let Some(snapshot) = self.server.snapshot() {
            if let Ok(packets) = resync_packets(&snapshot, self.server.checkpoints()) {
                for packet in packets {
                    if transport
                        .send_reliable(link.peer, ControlKind::Lobby, &packet)
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }

    fn on_input(&mut self, link: SeatLink, bytes: &[u8]) {
        let Some(seat) = self.seats.iter().find(|seat| seat.live && seat.link == link) else {
            return;
        };
        let Ok(packet) = decode_input(bytes) else {
            return;
        };
        let now = self.server.tick_now();
        for input in packet.inputs {
            if input.fighter.0 == seat.slot
                && input.tick >= now
                && input.tick <= now.saturating_add(MAX_AHEAD_TICKS)
            {
                self.inputs.entry((input.tick, seat.slot)).or_insert(input);
            }
        }
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
            self.send_history(transports, &delta);
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
    fn send_history(&self, transports: &mut [&mut dyn Transport], delta: &SnapshotDelta) {
        let Ok(packets) = resync_packets(delta, self.server.checkpoints()) else {
            return;
        };
        for seat in self.seats.iter().filter(|seat| seat.live) {
            let Some(transport) = transports.get_mut(seat.link.via) else {
                continue;
            };
            for packet in &packets {
                if transport
                    .send_reliable(seat.link.peer, ControlKind::Lobby, packet)
                    .is_err()
                {
                    break;
                }
            }
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

/// The page's side of the duel.
pub struct DuelClient {
    spec: MatchSpec,
    build: BuildId,
    token: u64,
    slot: u8,
    phase: Phase,
    known: BTreeMap<(u32, u8), FighterInput>,
    sent: Vec<FighterInput>,
    top: Option<Checkpoint>,
    mispredicts: u64,
    rejected: u32,
}

impl DuelClient {
    /// A client for the duel with a nonzero seat `token` (the same token
    /// reclaims the same seat).
    ///
    /// # Errors
    ///
    /// [`DuelError`] for a zero token or a spec that does not build.
    pub fn new(token: u64) -> Result<Self, DuelError> {
        if token == 0 {
            return Err(DuelError::Net(NetError::BadInput));
        }
        let spec = duel_spec()?;
        Ok(Self {
            build: duel_build(&spec),
            spec,
            token,
            slot: 0,
            phase: Phase::Start,
            known: BTreeMap::new(),
            sent: Vec::new(),
            top: None,
            mispredicts: 0,
            rejected: 0,
        })
    }

    /// [`DuelClient::new`] on a spec of the caller's (a build mismatch proof).
    #[must_use]
    pub fn with_spec(token: u64, spec: MatchSpec) -> Self {
        Self {
            build: duel_build(&spec),
            spec,
            token,
            slot: 0,
            phase: Phase::Start,
            known: BTreeMap::new(),
            sent: Vec::new(),
            top: None,
            mispredicts: 0,
            rejected: 0,
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

    /// Run one round: send the join, read the transport, apply the newest
    /// state, send the next inputs. Returns true once the duel ended.
    pub fn poll<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) -> bool {
        if matches!(self.phase, Phase::Start) {
            let join = Msg::JoinRequest {
                token: self.token,
                build: self.build,
            };
            match send_lobby(transport, server, join) {
                Ok(()) => self.phase = Phase::Joining,
                Err(error) => self.fail(format!("the join could not be sent: {error}")),
            }
        }
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
        self.send_inputs(transport, server);
        self.finish();
        matches!(self.phase, Phase::Done(_))
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
                match Client::from_spec(self.spec.clone(), FighterId(slot)) {
                    Ok(client) => {
                        self.slot = slot;
                        self.phase = Phase::Playing(Box::new(client));
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
            _ => {}
        }
    }

    fn on_state(&mut self, bytes: &[u8]) {
        let Ok(packet) = decode_state(bytes) else {
            self.rejected += 1;
            return;
        };
        for input in packet.inputs {
            self.known.insert((input.tick, input.fighter.0), input);
        }
        if self
            .top
            .is_none_or(|top| packet.checkpoint.tick > top.tick)
        {
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
        if client.server_tick().is_some_and(|done| checkpoint.tick <= done) {
            return;
        }
        let tape: Vec<FighterInput> = self
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
                client.resimulate();
            }
            Err(_) => self.rejected += 1,
        }
    }

    /// Predict and send every input up to the lead past the newest state.
    fn send_inputs<T: Transport + ?Sized>(&mut self, transport: &mut T, server: PeerId) {
        let Phase::Playing(client) = &mut self.phase else {
            return;
        };
        let limit = client
            .server_tick()
            .map_or(LEAD_TICKS, |tick| tick.saturating_add(1 + LEAD_TICKS))
            .min(DUEL_TICKS - 1);
        let mut failure = None;
        while client.tick_now() <= limit {
            let input = scripted_input(client.tick_now(), self.slot);
            if let Err(error) = client.predict(input) {
                failure = Some(format!("the prediction refused tick {}: {error}", input.tick));
                break;
            }
            self.sent.push(input);
            let from = self.sent.len().saturating_sub(REDUNDANCY);
            if let Err(error) = send_input(transport, server, input.tick, &self.sent[from..]) {
                failure = Some(format!("the input of tick {} could not be sent: {error}", input.tick));
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
        if client.server_tick() != Some(DUEL_TICKS - 1) {
            return;
        }
        let Some(last) = self.top.filter(|cp| cp.tick == DUEL_TICKS - 1) else {
            return;
        };
        let mut rebuilt = false;
        let mut held = client.checkpoints().last().copied().filter(|cp| cp.tick == DUEL_TICKS - 1);
        if held.map(|cp| cp.hash) != Some(last.hash) {
            // Datagrams went missing: replay once over everything the server
            // ever attested, the way a rejoin does.
            rebuilt = true;
            held = self.replay_known(last);
        }
        let result = match held {
            Some(cp) if cp.hash == last.hash => Ok(DuelOutcome {
                ticks: DUEL_TICKS,
                hash: last.hash,
                peers: 1,
                slot: self.slot,
                mispredicts: self.mispredicts,
                rebuilt,
            }),
            other => Err(DuelFailure(format!(
                "desync at tick {}: the server says {:016x}, this client computed {}",
                last.tick,
                last.hash,
                other.map_or_else(|| "nothing".to_owned(), |cp| format!("{:016x}", cp.hash)),
            ))),
        };
        self.phase = Phase::Done(result);
    }

    /// The checkpoint at the last tick of a fresh client fed the whole known tape.
    fn replay_known(&self, last: Checkpoint) -> Option<Checkpoint> {
        let mut fresh = Client::from_spec(self.spec.clone(), FighterId(self.slot)).ok()?;
        let delta = SnapshotDelta {
            version: WIRE_VERSION,
            tick: last.tick,
            hash: last.hash,
            tape: self.known.values().copied().collect(),
        };
        fresh.on_server_state(last, &delta).ok()?;
        fresh.resimulate().last().copied().filter(|cp| cp.tick == last.tick)
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

#[cfg(test)]
mod tests;
