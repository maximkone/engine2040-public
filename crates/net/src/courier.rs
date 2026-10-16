//! courier: the real-UDP link between the two machines of a split bout.
//!
//! Purpose: carry one machine's blobs to the other over a UDP socket, either
//! straight or through the self-hosted relay, in lockstep slots: each
//! [`Courier::exchange`] sends this machine's blob for the next slot and
//! returns the other machine's, resending lost pieces until both hold both.
//! It also registers at the relay, learns the public address from the
//! relay's STUN echo, runs the hole punch, times probe round trips, and
//! serves the relay hub for the relay binary. Owner: net lane.
//! Units: wall microseconds, bytes, 60 Hz punch ticks.
//! Determinism: none needed and none claimed: the courier moves bytes and
//! measures the real network; the wall clock paces resends and times probes
//! only, and never reaches the sim, a tape or a hash.
//! Errors: [`SplitError`]; decoders refuse hostile bytes, never panic.

use std::collections::{BTreeMap, VecDeque};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::OnceLock;
use std::time::Duration;

use engine_core::mix64;

use crate::played::BoutError;
use crate::relay::{
    answer_challenge, decode_binding_response, encode_binding_request, mint_challenge,
    verify_answer, Candidate, HolePunch, NatKind, RelayError, RelayHub, RelayMsg,
};

/// Split wire magic, `b"SP"` little-endian: the first two bytes of every datagram.
pub const SPLIT_MAGIC: u16 = 0x5053;
/// Split wire version; every other version refuses.
pub const SPLIT_VERSION: u8 = 1;
/// Probes per burst on a fresh road.
pub const PROBES: u32 = 300;
/// Wall microseconds between two probes of a burst.
pub const PROBE_GAP_US: u64 = 2_000;
/// Blob bytes per piece: a piece plus its header plus a relay forward header
/// stays under the relay's 1200-byte cap.
const CHUNK_BYTES: usize = 1024;
/// Most pieces in one blob (1 MiB).
const MAX_PIECES: usize = 1024;
const KIND_PIECE: u8 = 0;
const KIND_PROBE: u8 = 1;
const KIND_ECHO: u8 = 2;
const KIND_PUNCH: u8 = 3;
/// Wall microseconds of one 60 Hz punch tick.
const TICK_US: u64 = 16_667;
/// Silence that ends a session once both machines are talking.
const PATIENCE_US: u64 = 30_000_000;
/// Silence allowed before first contact: the other machine may start late.
const FIRST_PATIENCE_US: u64 = 600_000_000;
/// Resend interval until the first probe burst measures the road.
const START_RTO_US: u64 = 100_000;
/// Keepalive interval for the road not in use: the relay registration and
/// the punched direct mapping both stay warm.
const KEEPALIVE_US: u64 = 2_000_000;
/// Answers to punches that already heard this side: enough to survive a
/// lost confirmation, few enough that two sides never echo forever.
const ECHO_BUDGET: u32 = 8;

/// Why a split bout stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SplitError {
    /// A socket call failed; the step is named.
    Io(&'static str, ErrorKind),
    /// The bout refused.
    Bout(BoutError),
    /// The relay refused.
    Relay(RelayError),
    /// The other machine stayed quiet for the whole patience window at this slot.
    Silent(u32),
    /// The other machine sent bytes that do not decode; the part is named.
    BadBlob(&'static str),
    /// The other machine runs another build.
    Build,
    /// No road leads to the other machine yet.
    NoRoad,
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(step, kind) => write!(f, "split socket {step}: {kind}"),
            Self::Bout(err) => write!(f, "split {err}"),
            Self::Relay(err) => write!(f, "split relay: {err}"),
            Self::Silent(slot) => write!(f, "split: the other machine went quiet at slot {slot}"),
            Self::BadBlob(part) => write!(f, "split: bad blob ({part})"),
            Self::Build => write!(f, "split: the other machine runs another build"),
            Self::NoRoad => write!(f, "split: no road to the other machine"),
        }
    }
}

impl std::error::Error for SplitError {}

impl From<BoutError> for SplitError {
    fn from(err: BoutError) -> Self {
        Self::Bout(err)
    }
}

impl From<RelayError> for SplitError {
    fn from(err: RelayError) -> Self {
        Self::Relay(err)
    }
}

/// Monotonic wall microseconds since the first call. Paces resends and
/// times probe round trips; it never reaches the sim, a tape or a hash.
#[expect(
    clippy::disallowed_types,
    reason = "real-network resend pacing and probe timing, never hashed"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "real-network resend pacing and probe timing, never hashed"
)]
fn wall_us() -> u64 {
    static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
    let now = std::time::Instant::now(); // det-ok: real-network resend pacing and probe timing, never hashed.
    let epoch = *EPOCH.get_or_init(|| now);
    u64::try_from(now.duration_since(epoch).as_micros()).unwrap_or(u64::MAX)
}

/// Map a socket failure to the step that failed.
fn io(step: &'static str) -> impl Fn(std::io::Error) -> SplitError {
    move |err| SplitError::Io(step, err.kind())
}

/// Socket outcomes that mean "nothing to read now": timeouts, and the
/// Windows reset a send to a closed port leaves on the next receive.
fn quiet(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock
            | ErrorKind::TimedOut
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionRefused
            | ErrorKind::Interrupted
    )
}

/// Nearest-rank value at `pct` percent of an ascending list; zero when empty.
fn nearest_rank(sorted: &[u64], pct: u64) -> u64 {
    let total = u64::try_from(sorted.len()).unwrap_or(u64::MAX);
    if total == 0 {
        return 0;
    }
    let rank = (total * pct.clamp(1, 100)).div_ceil(100).max(1);
    let at = usize::try_from(rank - 1).unwrap_or(usize::MAX);
    sorted
        .get(at)
        .or_else(|| sorted.last())
        .copied()
        .unwrap_or(0)
}

/// The road one machine's bytes take to the other.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Road {
    /// Straight to the other machine's address.
    Direct,
    /// Through the self-hosted relay as forwards between two registered ids.
    Relay,
}

impl Road {
    /// Receipt label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Relay => "relay",
        }
    }
}

/// This machine's registration at the relay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RelayLeg {
    /// Relay address.
    pub addr: SocketAddr,
    /// Token the relay checks on every registration.
    pub token: u64,
    /// This machine's relay id.
    pub me: u64,
    /// The other machine's relay id.
    pub peer: u64,
}

/// Round trips of one probe burst on one road.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProbeStats {
    /// Road the probes took.
    pub road: Road,
    /// Probes sent.
    pub sent: u32,
    /// Round trips of the probes that came back, microseconds, ascending.
    pub rtt_us: Vec<u64>,
}

impl ProbeStats {
    /// Probes lost either way, in basis points of those sent.
    #[must_use]
    pub fn loss_bp(&self) -> u64 {
        let sent = u64::from(self.sent);
        let back = u64::try_from(self.rtt_us.len())
            .unwrap_or(u64::MAX)
            .min(sent);
        if sent == 0 {
            return 0;
        }
        (sent - back) * 10_000 / sent
    }

    /// Nearest-rank round trip at `pct` percent, microseconds.
    #[must_use]
    pub fn rtt_pct(&self, pct: u64) -> u64 {
        nearest_rank(&self.rtt_us, pct)
    }
}

/// Traffic counters of one courier.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct CourierStats {
    /// Datagrams offered to the socket.
    pub datagrams_sent: u64,
    /// Datagrams read off the socket.
    pub datagrams_recv: u64,
    /// Bytes the socket accepted.
    pub bytes_sent: u64,
    /// Bytes read off the socket.
    pub bytes_recv: u64,
    /// Blob pieces sent again after a timeout or a stale piece.
    pub pieces_resent: u64,
    /// Slot exchanges completed.
    pub exchanges: u64,
}

/// Wall cost of one bout's lockstep on the real network.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RunNet {
    /// Road the bout took.
    pub road: Road,
    /// Wall time from setup to the final blob.
    pub wall_us: u64,
    /// Wait of every slot exchange, microseconds, ascending.
    pub waits_us: Vec<u64>,
    /// Pieces resent during the bout.
    pub pieces_resent: u64,
}

impl RunNet {
    /// Nearest-rank exchange wait at `pct` percent, microseconds.
    #[must_use]
    pub fn wait_pct(&self, pct: u64) -> u64 {
        nearest_rank(&self.waits_us, pct)
    }
}

/// What the hole punch saw on one machine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PunchReport {
    /// A two-way direct exchange happened inside the bound.
    pub confirmed: bool,
    /// Failed check ticks before the decision (the bound on a fallback).
    pub decided_after_ticks: u32,
    /// Wall time the punch took.
    pub wall_us: u64,
    /// Punch probes sent.
    pub probes_sent: u32,
    /// Punch probes heard from the other machine.
    pub punches_heard: u32,
    /// Address the other machine's punches came from.
    pub heard_from: Option<SocketAddr>,
}

/// Blob of one slot, put back together from its pieces.
struct Assembly {
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
}

impl Assembly {
    fn new(count: usize) -> Self {
        Self {
            parts: vec![None; count],
            have: 0,
        }
    }

    fn put(&mut self, index: usize, count: usize, chunk: &[u8]) {
        if count != self.parts.len() {
            return;
        }
        if let Some(part @ None) = self.parts.get_mut(index) {
            *part = Some(chunk.to_vec());
            self.have += 1;
        }
    }

    fn done(&self) -> bool {
        self.have == self.parts.len()
    }

    fn join(self) -> Vec<u8> {
        self.parts.into_iter().flatten().flatten().collect()
    }
}

/// What the punch has heard so far.
#[derive(Clone, Copy, Default)]
struct Heard {
    from: Option<SocketAddr>,
    confirmed: bool,
    punches: u32,
    echoes: u32,
}

/// Header of every split datagram.
fn head(kind: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(CHUNK_BYTES + 16);
    out.extend_from_slice(&SPLIT_MAGIC.to_le_bytes());
    out.push(SPLIT_VERSION);
    out.push(kind);
    out
}

/// Kind and body of a split datagram; `None` for anything else.
fn split_head(bytes: &[u8]) -> Option<(u8, &[u8])> {
    match bytes {
        [lo, hi, version, kind, body @ ..]
            if u16::from_le_bytes([*lo, *hi]) == SPLIT_MAGIC && *version == SPLIT_VERSION =>
        {
            Some((*kind, body))
        }
        _ => None,
    }
}

/// A punch probe: sender id, and whether the sender has heard us.
fn punch_msg(from: u64, heard: bool) -> Vec<u8> {
    let mut out = head(KIND_PUNCH);
    out.extend_from_slice(&from.to_le_bytes());
    out.push(u8::from(heard));
    out
}

fn decode_punch(bytes: &[u8]) -> Option<(u64, bool)> {
    let (KIND_PUNCH, body) = split_head(bytes)? else {
        return None;
    };
    let mut r = Reader::new(body);
    let (from, heard) = (r.u64().ok()?, r.flag().ok()?);
    r.end().ok()?;
    Some((from, heard))
}

/// Cut `blob` into the pieces of `slot`; an empty blob is one empty piece.
fn pieces(slot: u32, blob: &[u8]) -> Result<Vec<Vec<u8>>, SplitError> {
    let chunks: Vec<&[u8]> = if blob.is_empty() {
        vec![blob]
    } else {
        blob.chunks(CHUNK_BYTES).collect()
    };
    let count = u16::try_from(chunks.len())
        .ok()
        .filter(|count| usize::from(*count) <= MAX_PIECES)
        .ok_or(SplitError::BadBlob("too large"))?;
    Ok(chunks
        .iter()
        .zip(0u16..)
        .map(|(chunk, index)| {
            let mut piece = head(KIND_PIECE);
            piece.extend_from_slice(&slot.to_le_bytes());
            piece.extend_from_slice(&index.to_le_bytes());
            piece.extend_from_slice(&count.to_le_bytes());
            piece.extend_from_slice(chunk);
            piece
        })
        .collect())
}

/// One machine's end of the split link: a UDP socket, the road to the other
/// machine and the slot exchange that keeps both in lockstep. Each
/// [`Self::exchange`] sends this machine's blob for the next slot and
/// returns the other machine's; lost pieces are resent until both machines
/// hold both blobs, so the two always agree on the slot count.
pub struct Courier {
    sock: UdpSocket,
    buf: Vec<u8>,
    relay: Option<RelayLeg>,
    direct: Option<SocketAddr>,
    listen: bool,
    road: Road,
    slot: u32,
    sent: VecDeque<(u32, Vec<Vec<u8>>)>,
    inbox: BTreeMap<u32, Assembly>,
    rto_us: u64,
    old_resend_at: u64,
    keepalive_at: u64,
    echoes: Option<BTreeMap<u32, u64>>,
    probe_count: u32,
    probes: Vec<ProbeStats>,
    registered: bool,
    denied: Option<u8>,
    stun_tx: [u8; 12],
    srflx: Option<SocketAddr>,
    heard: Heard,
    stats: CourierStats,
    run_start: u64,
    run_resent: u64,
    waits_us: Vec<u64>,
    lose_every: u64,
}

impl Courier {
    fn open(bind: SocketAddr) -> Result<Self, SplitError> {
        let sock = UdpSocket::bind(bind).map_err(io("bind"))?;
        Ok(Self {
            sock,
            buf: vec![0; 65_536],
            relay: None,
            direct: None,
            listen: false,
            road: Road::Direct,
            slot: 0,
            sent: VecDeque::new(),
            inbox: BTreeMap::new(),
            rto_us: START_RTO_US,
            old_resend_at: 0,
            keepalive_at: 0,
            echoes: None,
            probe_count: PROBES,
            probes: Vec::new(),
            registered: false,
            denied: None,
            stun_tx: [0; 12],
            srflx: None,
            heard: Heard::default(),
            stats: CourierStats::default(),
            run_start: 0,
            run_resent: 0,
            waits_us: Vec::new(),
            lose_every: 0,
        })
    }

    /// Machine A on a LAN: bind and take the first machine that says hello.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the bind fails.
    pub fn listen(bind: SocketAddr) -> Result<Self, SplitError> {
        let mut courier = Self::open(bind)?;
        courier.listen = true;
        Ok(courier)
    }

    /// Machine B on a LAN: talk straight to `server`.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the bind fails.
    pub fn connect(bind: SocketAddr, server: SocketAddr) -> Result<Self, SplitError> {
        let mut courier = Self::open(bind)?;
        courier.direct = Some(server);
        Ok(courier)
    }

    /// Either machine across networks: every byte rides the relay until a
    /// punch opens a direct road.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the bind fails.
    pub fn via_relay(bind: SocketAddr, leg: RelayLeg) -> Result<Self, SplitError> {
        let mut courier = Self::open(bind)?;
        courier.relay = Some(leg);
        courier.road = Road::Relay;
        let (a, b) = (mix64(leg.me).to_le_bytes(), mix64(!leg.me).to_le_bytes());
        courier.stun_tx[..8].copy_from_slice(&a);
        courier.stun_tx[8..].copy_from_slice(&b[..4]);
        Ok(courier)
    }

    /// Address this machine receives on.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the socket cannot say.
    pub fn local_addr(&self) -> Result<SocketAddr, SplitError> {
        self.sock.local_addr().map_err(io("local address"))
    }

    /// The other machine's direct address, once known.
    #[must_use]
    pub fn peer(&self) -> Option<SocketAddr> {
        self.direct
    }

    /// The road in use.
    #[must_use]
    pub fn road(&self) -> Road {
        self.road
    }

    /// Traffic counters so far.
    #[must_use]
    pub fn stats(&self) -> CourierStats {
        self.stats
    }

    /// Every probe burst so far, in order.
    #[must_use]
    pub fn probes(&self) -> &[ProbeStats] {
        &self.probes
    }

    /// Probes per burst on a fresh road; both machines must agree.
    pub fn set_probes(&mut self, count: u32) {
        self.probe_count = count;
    }

    /// Drop every `nth` outgoing datagram on purpose (zero: never), to prove
    /// the resend path on a clean network.
    pub fn lose_every(&mut self, nth: u64) {
        self.lose_every = nth;
    }

    /// Send on `road` from now on.
    ///
    /// # Errors
    ///
    /// [`SplitError::NoRoad`] when that road is not open.
    pub fn set_road(&mut self, road: Road) -> Result<(), SplitError> {
        let open = match road {
            Road::Relay => self.relay.is_some(),
            Road::Direct => self.direct.is_some(),
        };
        if !open {
            return Err(SplitError::NoRoad);
        }
        self.road = road;
        Ok(())
    }

    fn send_raw(&mut self, dest: SocketAddr, bytes: &[u8]) {
        self.stats.datagrams_sent += 1;
        if self.lose_every > 0 && self.stats.datagrams_sent.is_multiple_of(self.lose_every) {
            return;
        }
        // A failed send is loss: the slot exchange resends it.
        if self.sock.send_to(bytes, dest).is_ok() {
            self.stats.bytes_sent += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        }
    }

    fn send_payload(&mut self, bytes: &[u8]) {
        match (self.road, self.relay, self.direct) {
            (Road::Relay, Some(leg), _) => {
                let forward = RelayMsg::Forward {
                    from: leg.me,
                    to: leg.peer,
                    payload: bytes.to_vec(),
                };
                if let Ok(body) = forward.encode() {
                    self.send_raw(leg.addr, &body);
                }
            }
            (Road::Direct, _, Some(addr)) => self.send_raw(addr, bytes),
            // No road yet: the listener has not heard its peer.
            _ => {}
        }
    }

    /// Wait up to `wait_us` for one datagram and handle it. True when it
    /// came from the other machine.
    fn pump(&mut self, wait_us: u64) -> Result<bool, SplitError> {
        let wait = Duration::from_micros(wait_us.max(1_000));
        self.sock
            .set_read_timeout(Some(wait))
            .map_err(io("timeout"))?;
        let (len, src) = match self.sock.recv_from(&mut self.buf) {
            Ok(got) => got,
            Err(err) if quiet(err.kind()) => return Ok(false),
            Err(err) => return Err(SplitError::Io("recv", err.kind())),
        };
        self.stats.datagrams_recv += 1;
        self.stats.bytes_recv += u64::try_from(len).unwrap_or(u64::MAX);
        let bytes = self.buf[..len].to_vec();
        Ok(self.on_raw(src, &bytes))
    }

    fn pump_until(&mut self, until_us: u64) -> Result<(), SplitError> {
        loop {
            let now = wall_us();
            if now >= until_us {
                return Ok(());
            }
            self.pump(until_us - now)?;
        }
    }

    fn on_raw(&mut self, src: SocketAddr, bytes: &[u8]) -> bool {
        if let Some(leg) = self.relay.filter(|leg| leg.addr == src) {
            return self.on_relay(leg, bytes);
        }
        if let Some((from, heard)) = decode_punch(bytes) {
            return self.on_punch(src, from, heard);
        }
        match self.direct {
            Some(addr) if addr == src => {
                self.on_payload(bytes);
                true
            }
            None if self.listen && opens_session(bytes) => {
                self.direct = Some(src);
                self.on_payload(bytes);
                // The opening blob went nowhere before the road was known.
                self.send_slot(self.slot);
                true
            }
            _ => false,
        }
    }

    fn on_relay(&mut self, leg: RelayLeg, bytes: &[u8]) -> bool {
        match RelayMsg::decode(bytes) {
            Ok(RelayMsg::Forward { from, to, payload }) if (from, to) == (leg.peer, leg.me) => {
                self.on_payload(&payload);
                true
            }
            Ok(RelayMsg::Registered { client_id }) if client_id == leg.me => {
                self.registered = true;
                false
            }
            Ok(RelayMsg::Denied { client_id, code }) if client_id == leg.me => {
                self.denied = Some(code);
                false
            }
            Ok(_) => false,
            Err(_) => {
                if let Ok(addr) = decode_binding_response(bytes, self.stun_tx) {
                    self.srflx = Some(addr);
                }
                false
            }
        }
    }

    /// A punch from the other machine: note where it came from and answer
    /// it, so the other side confirms even after this side has decided.
    fn on_punch(&mut self, src: SocketAddr, from: u64, heard: bool) -> bool {
        let Some(leg) = self.relay.filter(|leg| leg.peer == from) else {
            return false;
        };
        self.heard.punches += 1;
        self.heard.from.get_or_insert(src);
        self.heard.confirmed |= heard;
        // A punch that has not heard us always gets an answer; one that has
        // gets a few, never an endless echo between the two sides.
        if !heard || self.heard.echoes < ECHO_BUDGET {
            self.heard.echoes += u32::from(heard);
            self.send_raw(src, &punch_msg(leg.me, true));
        }
        true
    }

    fn on_payload(&mut self, bytes: &[u8]) {
        let Some((kind, body)) = split_head(bytes) else {
            return;
        };
        match kind {
            KIND_PIECE => self.on_piece(body),
            KIND_PROBE => {
                let mut echo = head(KIND_ECHO);
                echo.extend_from_slice(body);
                self.send_payload(&echo);
            }
            KIND_ECHO => {
                let mut r = Reader::new(body);
                let (Ok(seq), Ok(sent)) = (r.u32(), r.u64()) else {
                    return;
                };
                let now = wall_us();
                if let Some(echoes) = self.echoes.as_mut() {
                    echoes
                        .entry(seq)
                        .or_insert_with(|| now.saturating_sub(sent));
                }
            }
            _ => {}
        }
    }

    fn on_piece(&mut self, body: &[u8]) {
        let mut r = Reader::new(body);
        let (Ok(slot), Ok(index), Ok(count)) = (r.u32(), r.u16(), r.u16()) else {
            return;
        };
        let (index, count) = (usize::from(index), usize::from(count));
        if count == 0 || count > MAX_PIECES || index >= count {
            return;
        }
        if slot < self.slot {
            // The other machine still waits on an older slot: our blob for
            // it was lost. Answer at most twice per resend interval.
            let now = wall_us();
            if now >= self.old_resend_at {
                self.old_resend_at = now + self.rto_us / 2;
                self.stats.pieces_resent += self.send_slot(slot);
            }
            return;
        }
        if slot > self.slot.saturating_add(1) {
            return;
        }
        self.inbox
            .entry(slot)
            .or_insert_with(|| Assembly::new(count))
            .put(index, count, r.rest());
    }

    fn send_slot(&mut self, slot: u32) -> u64 {
        let Some(pieces) = self
            .sent
            .iter()
            .find(|(sent, _)| *sent == slot)
            .map(|(_, pieces)| pieces.clone())
        else {
            return 0;
        };
        for piece in &pieces {
            self.send_payload(piece);
        }
        u64::try_from(pieces.len()).unwrap_or(u64::MAX)
    }

    fn take(&mut self, slot: u32) -> Option<Vec<u8>> {
        if !self.inbox.get(&slot).is_some_and(Assembly::done) {
            return None;
        }
        let blob = self.inbox.remove(&slot).map(Assembly::join);
        self.inbox.retain(|kept, _| *kept > slot);
        blob
    }

    /// Keep the road not in use warm: re-register at the relay while the
    /// direct road carries the game, punch the direct mapping while the
    /// relay does.
    fn keepalive(&mut self, now: u64) {
        let Some(leg) = self.relay else {
            return;
        };
        if now < self.keepalive_at {
            return;
        }
        self.keepalive_at = now + KEEPALIVE_US;
        match (self.road, self.heard.from) {
            (Road::Direct, _) => {
                let register = RelayMsg::Register {
                    client_id: leg.me,
                    token: leg.token,
                };
                if let Ok(bytes) = register.encode() {
                    self.send_raw(leg.addr, &bytes);
                }
            }
            (Road::Relay, Some(addr)) if self.heard.confirmed => {
                self.send_raw(addr, &punch_msg(leg.me, true));
            }
            (Road::Relay, _) => {}
        }
    }

    /// Send this machine's `blob` for the next slot and return the other
    /// machine's blob for the same slot.
    ///
    /// # Errors
    ///
    /// [`SplitError::Silent`] when the other machine stays quiet past the
    /// patience window; [`SplitError::Io`] when the socket fails.
    pub fn exchange(&mut self, blob: &[u8]) -> Result<Vec<u8>, SplitError> {
        let slot = self.slot;
        self.sent.push_back((slot, pieces(slot, blob)?));
        while self.sent.len() > 2 {
            self.sent.pop_front();
        }
        self.send_slot(slot);
        let patience = if slot == 0 {
            FIRST_PATIENCE_US
        } else {
            PATIENCE_US
        };
        let start = wall_us();
        let (mut sent_at, mut heard_at) = (start, start);
        loop {
            if let Some(theirs) = self.take(slot) {
                self.slot = slot + 1;
                self.stats.exchanges += 1;
                self.waits_us.push(wall_us().saturating_sub(start));
                return Ok(theirs);
            }
            let now = wall_us();
            if now.saturating_sub(heard_at) > patience {
                return Err(SplitError::Silent(slot));
            }
            if now.saturating_sub(sent_at) >= self.rto_us {
                self.stats.pieces_resent += self.send_slot(slot);
                sent_at = now;
            }
            self.keepalive(now);
            let wait = self.rto_us.saturating_sub(now.saturating_sub(sent_at));
            if self.pump(wait)? {
                heard_at = wall_us();
            }
        }
    }

    /// Time `count` probe round trips on the road in use, `gap_us` apart,
    /// then size the resend interval from the measured p95.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the socket fails.
    pub fn probe(&mut self, count: u32, gap_us: u64) -> Result<ProbeStats, SplitError> {
        self.echoes = Some(BTreeMap::new());
        let start = wall_us();
        for seq in 0..count {
            let mut msg = head(KIND_PROBE);
            msg.extend_from_slice(&seq.to_le_bytes());
            msg.extend_from_slice(&wall_us().to_le_bytes());
            self.send_payload(&msg);
            self.pump_until(start + gap_us * u64::from(seq + 1))?;
        }
        self.pump_until(wall_us() + (self.rto_us * 4).max(300_000))?;
        let mut rtt_us: Vec<u64> = self
            .echoes
            .take()
            .unwrap_or_default()
            .into_values()
            .collect();
        rtt_us.sort_unstable();
        let stats = ProbeStats {
            road: self.road,
            sent: count,
            rtt_us,
        };
        if !stats.rtt_us.is_empty() {
            self.rto_us = (stats.rtt_pct(95) * 2 + 2_000).clamp(4_000, 1_000_000);
        }
        self.probes.push(stats.clone());
        Ok(stats)
    }

    /// Keep answering after the last exchange until the other machine goes
    /// quiet, so its last exchange lands too.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the socket fails.
    pub fn linger(&mut self) -> Result<(), SplitError> {
        let quiet_us = (self.rto_us * 4).max(300_000);
        let cap = wall_us() + 5_000_000;
        let mut heard = wall_us();
        loop {
            let now = wall_us();
            if now >= cap || now.saturating_sub(heard) >= quiet_us {
                return Ok(());
            }
            if self.pump(quiet_us)? {
                heard = wall_us();
            }
        }
    }

    /// Trust `addr` as the other machine's direct address: the punch heard
    /// it there and both machines confirmed a two-way exchange.
    pub(crate) fn open_direct(&mut self, addr: SocketAddr) {
        self.direct = Some(addr);
    }

    /// Switch to `road`, probe it when it is new, and start timing a bout.
    /// Both machines enter the same roads in the same order, so their probe
    /// bursts line up.
    pub(crate) fn enter(&mut self, road: Road) -> Result<(), SplitError> {
        self.set_road(road)?;
        if self.probes.last().map(|probe| probe.road) != Some(road) {
            self.probe(self.probe_count, PROBE_GAP_US)?;
        }
        self.run_start = wall_us();
        self.run_resent = self.stats.pieces_resent;
        self.waits_us.clear();
        Ok(())
    }

    /// Stop timing a bout and hand back its wall cost.
    pub(crate) fn finish_run(&mut self) -> RunNet {
        let mut waits_us = std::mem::take(&mut self.waits_us);
        waits_us.sort_unstable();
        RunNet {
            road: self.road,
            wall_us: wall_us().saturating_sub(self.run_start),
            waits_us,
            pieces_resent: self.stats.pieces_resent - self.run_resent,
        }
    }

    /// Register at the relay and learn this machine's public address from
    /// its STUN echo; `None` when no echo came back.
    ///
    /// # Errors
    ///
    /// [`SplitError::NoRoad`] without a relay, [`SplitError::Relay`] when the
    /// relay denies, [`SplitError::Silent`] when it never answers.
    pub fn register(&mut self) -> Result<Option<SocketAddr>, SplitError> {
        let leg = self.relay.ok_or(SplitError::NoRoad)?;
        let register = RelayMsg::Register {
            client_id: leg.me,
            token: leg.token,
        }
        .encode()?;
        let stun = encode_binding_request(self.stun_tx);
        let start = wall_us();
        let mut sent_at: Option<u64> = None;
        loop {
            let now = wall_us();
            if let Some(code) = self.denied {
                return Err(SplitError::Relay(match code {
                    0 => RelayError::Unauthorized,
                    1 => RelayError::SlotFull,
                    _ => RelayError::BadInput,
                }));
            }
            let waited = now.saturating_sub(start);
            if self.registered && (self.srflx.is_some() || waited > 3_000_000) {
                return Ok(self.srflx);
            }
            if waited > PATIENCE_US {
                return Err(SplitError::Silent(0));
            }
            if sent_at.is_none_or(|at| now.saturating_sub(at) >= 250_000) {
                self.send_raw(leg.addr, &register);
                if self.srflx.is_none() {
                    self.send_raw(leg.addr, &stun);
                }
                sent_at = Some(now);
            }
            self.pump(50_000)?;
        }
    }

    /// This machine's own address as the other machine could reach it on a
    /// shared network: the host candidate.
    ///
    /// # Errors
    ///
    /// [`SplitError::Io`] when the socket cannot say.
    pub fn host_candidate(&self) -> Result<SocketAddr, SplitError> {
        let bound = self.local_addr()?;
        if !bound.ip().is_unspecified() {
            return Ok(bound);
        }
        let Some(leg) = self.relay else {
            return Ok(bound);
        };
        // A connected socket names the interface that routes to the relay.
        let any = match leg.addr {
            SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        };
        let route = UdpSocket::bind(SocketAddr::new(any, 0)).map_err(io("route bind"))?;
        route.connect(leg.addr).map_err(io("route"))?;
        let ip = route.local_addr().map_err(io("route address"))?.ip();
        Ok(SocketAddr::new(ip, bound.port()))
    }

    /// Punch toward the other machine's candidates: one probe per candidate
    /// per 60 Hz tick until a two-way exchange confirms a direct road, or
    /// the punch bound passes and the relay stays the road.
    ///
    /// # Errors
    ///
    /// [`SplitError::NoRoad`] without a relay; [`SplitError::Io`] when the
    /// socket fails.
    pub fn punch(&mut self, theirs: &[Candidate]) -> Result<PunchReport, SplitError> {
        let leg = self.relay.ok_or(SplitError::NoRoad)?;
        // Challenge round before the punch budget starts: the relay token
        // must answer the leg-bound challenge, deny by default. The peer half
        // of the exchange rides the candidate trade; this local gate proves
        // the order and refuses zero tokens before the first probe.
        let challenge = mint_challenge(leg.me ^ leg.peer, leg.me);
        if !verify_answer(challenge, leg.token, answer_challenge(challenge, leg.token)) {
            return Err(SplitError::Relay(RelayError::Unauthorized));
        }
        let mut machine = HolePunch::new(NatKind::Cone);
        machine.on_gathered();
        let start = wall_us();
        let (mut ticks, mut sent) = (0u64, 0u32);
        while !machine.is_direct() && !machine.needs_relay() {
            let heard = self.heard.from.is_some();
            for candidate in theirs {
                self.send_raw(candidate.addr, &punch_msg(leg.me, heard));
                sent += 1;
            }
            ticks += 1;
            self.pump_until(start + ticks * TICK_US)?;
            machine.tick(self.heard.confirmed);
        }
        if let (true, Some(addr)) = (machine.is_direct(), self.heard.from) {
            for _ in 0..3 {
                self.send_raw(addr, &punch_msg(leg.me, true));
            }
        }
        Ok(PunchReport {
            confirmed: machine.is_direct(),
            decided_after_ticks: machine.attempts(),
            wall_us: wall_us().saturating_sub(start),
            probes_sent: sent,
            punches_heard: self.heard.punches,
            heard_from: self.heard.from,
        })
    }
}

/// True for the first piece a joining machine sends: the session opener.
fn opens_session(bytes: &[u8]) -> bool {
    let Some((KIND_PIECE, body)) = split_head(bytes) else {
        return false;
    };
    body.get(..4) == Some(&0u32.to_le_bytes()[..])
}

/// Bounded little-endian reader over one blob; every read checks length.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], SplitError> {
        let end = self
            .at
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(SplitError::BadBlob("short"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SplitError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, SplitError> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, SplitError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, SplitError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, SplitError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(crate) fn count(&mut self) -> Result<usize, SplitError> {
        usize::try_from(self.u32()?).map_err(|_| SplitError::BadBlob("count"))
    }

    pub(crate) fn flag(&mut self) -> Result<bool, SplitError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SplitError::BadBlob("flag")),
        }
    }

    pub(crate) fn words<const N: usize>(&mut self) -> Result<[u32; N], SplitError> {
        let mut out = [0u32; N];
        for word in &mut out {
            *word = self.u32()?;
        }
        Ok(out)
    }

    fn rest(&mut self) -> &'a [u8] {
        let out = &self.bytes[self.at..];
        self.at = self.bytes.len();
        out
    }

    pub(crate) fn end(&self) -> Result<(), SplitError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(SplitError::BadBlob("trailing"))
        }
    }
}

/// Serve the relay hub on `sock`: answer every datagram, skip socket
/// hiccups, and ask `keep_going` after each datagram or quiet second.
///
/// # Errors
///
/// [`SplitError::Io`] when the socket fails for good.
pub fn serve_relay(
    sock: &UdpSocket,
    hub: &mut RelayHub,
    mut keep_going: impl FnMut(&RelayHub) -> bool,
) -> Result<(), SplitError> {
    sock.set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(io("timeout"))?;
    let mut buf = vec![0u8; 65_536];
    while keep_going(hub) {
        let (len, src) = match sock.recv_from(&mut buf) {
            Ok(got) => got,
            Err(err) if quiet(err.kind()) => continue,
            Err(err) => return Err(SplitError::Io("relay recv", err.kind())),
        };
        for (dest, bytes) in hub.handle(src, &buf[..len]) {
            // A failed send is loss; the clients resend.
            let _ = sock.send_to(&bytes, dest);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn local() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 0))
    }

    #[test]
    fn courier_exchange_resends_through_heavy_loss() {
        // Loss periods coprime with every piece count (1 to 4), so a resend
        // never loses the same piece twice in a row.
        let mut a = Courier::listen(local()).expect("listen");
        let server = a.local_addr().expect("address");
        a.lose_every(7);
        a.rto_us = 10_000;
        let blob = |slot: u32| -> Vec<u8> {
            let len = usize::try_from(slot % 5).expect("small") * 900;
            (0..len)
                .map(|at| {
                    u8::try_from((at + 7 * usize::try_from(slot).expect("small")) % 251)
                        .expect("byte")
                })
                .collect()
        };
        let peer = thread::spawn(move || {
            let mut b = Courier::connect(local(), server).expect("connect");
            b.lose_every(5);
            b.rto_us = 10_000;
            let mut got = Vec::new();
            for slot in 0..40 {
                got.push(b.exchange(&blob(slot + 1000)).expect("exchange"));
            }
            b.linger().expect("linger");
            (got, b.stats())
        });
        for slot in 0..40 {
            assert_eq!(
                a.exchange(&blob(slot)).expect("exchange"),
                blob(slot + 1000)
            );
        }
        a.linger().expect("linger");
        let (got, b_stats) = peer.join().expect("peer");
        for (slot, bytes) in (0..40).zip(got) {
            assert_eq!(bytes, blob(slot), "slot {slot} arrives whole");
        }
        assert!(
            a.stats().pieces_resent > 0 && b_stats.pieces_resent > 0,
            "loss forced resends"
        );
        assert_eq!((a.stats().exchanges, b_stats.exchanges), (40, 40));
    }

    #[test]
    fn courier_answers_punches_without_an_endless_echo() {
        let relay = SocketAddr::from(([127, 0, 0, 1], 9));
        let leg = RelayLeg {
            addr: relay,
            token: 1,
            me: 10,
            peer: 11,
        };
        let mut side = Courier::via_relay(local(), leg).expect("bind");
        let them = SocketAddr::from(([127, 0, 0, 1], 10));
        assert!(
            !side.on_punch(them, 99, false),
            "a stranger's punch is ignored"
        );
        assert_eq!(side.stats().datagrams_sent, 0);
        for _ in 0..3 {
            assert!(side.on_punch(them, 11, false));
        }
        assert_eq!(
            side.stats().datagrams_sent,
            3,
            "every unheard punch is answered"
        );
        assert!(!side.heard.confirmed);
        for _ in 0..50 {
            side.on_punch(them, 11, true);
        }
        assert!(side.heard.confirmed);
        assert_eq!(
            side.stats().datagrams_sent,
            3 + u64::from(ECHO_BUDGET),
            "heard punches get a bounded echo"
        );
        assert_eq!(side.heard.from, Some(them));
    }

    #[test]
    fn courier_pieces_cut_blobs_and_frames_refuse_junk() {
        let blob: Vec<u8> = (0..2500u32)
            .map(|at| u8::try_from(at % 256).expect("byte"))
            .collect();
        let cut = pieces(9, &blob).expect("pieces");
        assert_eq!(cut.len(), 3, "1024-byte chunks");
        assert!(cut.iter().all(|piece| piece.len() <= CHUNK_BYTES + 12));
        let mut whole = Assembly::new(3);
        for piece in cut.iter().rev() {
            let (KIND_PIECE, body) = split_head(piece).expect("split datagram") else {
                panic!("a piece");
            };
            let mut r = Reader::new(body);
            let (slot, index, count) = (
                r.u32().expect("slot"),
                r.u16().expect("index"),
                r.u16().expect("count"),
            );
            assert_eq!(slot, 9);
            whole.put(usize::from(index), usize::from(count), r.rest());
        }
        assert!(whole.done());
        assert_eq!(whole.join(), blob, "pieces reassemble in any order");
        assert_eq!(
            pieces(0, &[]).expect("empty").len(),
            1,
            "an empty blob is one piece"
        );
        let huge = vec![0u8; CHUNK_BYTES * (MAX_PIECES + 1)];
        assert_eq!(pieces(0, &huge), Err(SplitError::BadBlob("too large")));
        assert_eq!(decode_punch(&punch_msg(7, true)), Some((7, true)));
        assert_eq!(
            decode_punch(&punch_msg(7, true)[..12]),
            None,
            "a short punch is junk"
        );
        assert_eq!(
            split_head(b"RL\x01\x02"),
            None,
            "relay bytes are not split bytes"
        );
        assert!(opens_session(&pieces(0, b"hi").expect("piece")[0]));
        assert!(!opens_session(&pieces(1, b"hi").expect("piece")[0]));
    }

    #[test]
    fn courier_punch_refuses_zero_token_before_probing() {
        let relay = SocketAddr::from(([127, 0, 0, 1], 9));
        let leg = RelayLeg {
            addr: relay,
            token: 0,
            me: 10,
            peer: 11,
        };
        let mut side = Courier::via_relay(local(), leg).expect("bind");
        assert_eq!(
            side.punch(&[]),
            Err(SplitError::Relay(RelayError::Unauthorized)),
            "the challenge gate refuses before the punch budget starts"
        );
        assert_eq!(side.stats().datagrams_sent, 0, "no probe leaves first");
    }
}
