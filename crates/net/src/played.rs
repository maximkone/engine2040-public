//! played: the two-machine played bout over a [`Wire`](crate::link::Wire).
//!
//! Purpose: one real match, played. A lobby forms 2 humans plus AI fill, the
//! dedicated [`HeadlessServer`] authors the arena, two predicting [`Client`]s
//! send human-shaped input tapes, and a scripted drop plus rejoin runs
//! mid-fight (supervised disconnect, AI takeover, reclaim, catch-up stream).
//! The bout passes when every client holds the server's checkpoints in full
//! and the recorded tape replays to the same tip.
//! Owner: net lane. Units: ticks at 60 Hz, bytes, whole percent.
//! Determinism: the wire draws are seeded, inputs come from the shared
//! [`crate::scale`] human script, every map is a `BTreeMap`; the same config
//! and wire give the same [`BoutReport::digest`](crate::played::BoutReport::digest) on every run. No clock.
//! Errors: [`BoutError`](crate::played::BoutError) for setup, the report carries the verdict.
//! Budget: proof tool, not per-tick sim code; the arena prefix reruns per
//! tick as in [`crate::session`].

use std::collections::BTreeMap;

use engine_arena::{run, run_events, ArenaConfig, Checkpoint, LiveMatch};
use engine_core::fixed::Fixed;
use engine_core::{mix64, Fnv64};
use engine_input::fighter::{Buttons, ATTACK, BLOCK};

use crate::ack::input::{DeliveryStats, InputDelivery};
use crate::dedicated::{HeadlessServer, ServerError};
use crate::delay::{AdvantageWindow, ClockPacer};
use crate::guard::SlotReceipt;
use crate::heal::NetHealError;
use crate::lagcomp::LagTape;
use crate::link::{wire_len, Frame, LinkProfile, LinkReport, LinkStats, LoopbackWire, Peer, Wire};
use crate::lobby::{BuildId, Lobby, LobbyError, Msg, LOBBY_MAGIC};
use crate::relay::RelayError;
use crate::scale::{human_dwell, human_wish, RESIM_CAP_TICKS};
use crate::session::{
    decode_state, encode_input, encode_state, redundancy_window, resync_packets, tick_slice,
    BodyId, BodyInput, BodyTape, Client, NetError, SnapshotDelta, MAX_INPUTS_PER_PACKET,
    REDUNDANCY, WIRE_MAGIC, WIRE_VERSION,
};

/// Human seats in a bout: the first two lobby slots, the rest are AI fill.
pub const SEATS: u8 = 2;
/// [`SEATS`] as an array length.
pub const SEAT_COUNT: usize = 2;
/// Seat tokens, nonzero and sticky: the same token reclaims the same slot.
const TOKENS: [u64; SEAT_COUNT] = [0xA11CE, 0x0B0B];
/// Build identity both ends present at the lobby and the rejoin.
pub const BOUT_BUILD: BuildId = BuildId {
    version: 1,
    rules: 0x4D50_3620_504C_4159,
};
/// Fighters in the canonical bout: 2 humans plus 6 AI.
pub const BOUT_FIGHTERS: u8 = 8;
/// Canonical bout length in ticks (10 s at 60 Hz).
pub const BOUT_TICKS: u32 = 600;
/// Arena half-extent in meters: small enough that the fight stays in contact.
pub const BOUT_EXTENT_M: f32 = 9.0;
/// Authored ticks without a human input before the server drops the seat.
pub const STALL_TICKS: u32 = 30;
/// Steps after a rejoin that widen the input window to cover the backlog.
pub const REJOIN_BURST_STEPS: u32 = 12;
/// Steps the server keeps re-sending the tip after the last authored tick.
pub const DRAIN_STEPS: u32 = 48;
/// Steps a client waits after a resync request before it may ask again.
pub const RESYNC_WAIT_STEPS: u32 = 60;
/// Ticks of authoritative inputs repeated in every state packet.
pub const STATE_COVER: usize = 8;
/// Rollback histogram buckets: one per tick, the last holds everything deeper.
pub const ROLLBACK_BUCKETS: usize = 33;
/// Ticks each [`BoutReport::sync_test`] window rewinds and replays (GGRS
/// SyncTest style; the GGRS default is unverified).
pub const SYNC_CHECK_DISTANCE: u32 = 7;

/// Bout setup and verdict failures.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoutError {
    /// Config outside the bout shape; the culprit field is named.
    BadConfig(&'static str),
    /// The lobby refused to form the fight.
    Lobby(LobbyError),
    /// The dedicated server refused.
    Server(ServerError),
    /// A session codec or client call refused.
    Net(NetError),
    /// The supervised reconnect path refused.
    Heal(NetHealError),
    /// The relay or an invite refused.
    Relay(RelayError),
}

impl std::fmt::Display for BoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadConfig(field) => write!(f, "bout config: bad {field}"),
            Self::Lobby(err) => write!(f, "bout lobby: {err}"),
            Self::Server(err) => write!(f, "bout server: {err}"),
            Self::Net(err) => write!(f, "bout session: {err}"),
            Self::Heal(err) => write!(f, "bout heal: {err}"),
            Self::Relay(err) => write!(f, "bout relay: {err}"),
        }
    }
}

impl std::error::Error for BoutError {}

impl From<LobbyError> for BoutError {
    fn from(err: LobbyError) -> Self {
        Self::Lobby(err)
    }
}

impl From<ServerError> for BoutError {
    fn from(err: ServerError) -> Self {
        Self::Server(err)
    }
}

impl From<NetError> for BoutError {
    fn from(err: NetError) -> Self {
        Self::Net(err)
    }
}

impl From<RelayError> for BoutError {
    fn from(err: RelayError) -> Self {
        Self::Relay(err)
    }
}

impl From<NetHealError> for BoutError {
    fn from(err: NetHealError) -> Self {
        Self::Heal(err)
    }
}

/// The scripted drop: one seat loses its link at `at` and rejoins at `back`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DropScript {
    /// Seat that drops, below [`SEATS`].
    pub seat: u8,
    /// First tick of the outage.
    pub at: u32,
    /// Tick the link is back and the client asks to rejoin.
    pub back: u32,
}

/// One played bout: match shape, server hold and the scripted drop.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BoutConfig {
    /// Lobby seed; the arena expands it.
    pub seed: u64,
    /// Fighters, humans plus AI fill.
    pub fighters: u8,
    /// Bout length in ticks.
    pub ticks: u32,
    /// Arena half-extent in meters.
    pub half_extent_m: f32,
    /// Ticks the server holds a tick open for late inputs at least; the
    /// server's lag window widens it for a slower seat.
    pub hold: u32,
    /// The mid-fight drop and rejoin.
    pub drop: DropScript,
    /// Seat tokens (the machine identities), one per seat. The bout never
    /// reads a token into the sim: the AI fill and the tape key on seats.
    pub tokens: [u64; SEAT_COUNT],
}

impl BoutConfig {
    /// Canonical bout over a link of this `worst` one-way delay in ticks:
    /// the server holds each tick open one tick past the slowest input.
    #[must_use]
    pub fn canonical(seed: u64, worst_delay: u32) -> Self {
        Self {
            seed,
            fighters: BOUT_FIGHTERS,
            ticks: BOUT_TICKS,
            half_extent_m: BOUT_EXTENT_M,
            hold: worst_delay + 1,
            drop: DropScript {
                seat: 1,
                at: 60,
                back: 180,
            },
            tokens: TOKENS,
        }
    }

    /// Canonical bout sized for one link profile.
    #[must_use]
    pub fn for_profile(seed: u64, profile: LinkProfile) -> Self {
        Self::canonical(seed, profile.worst_delay())
    }

    /// Range-check the bout shape.
    pub fn validate(&self) -> Result<(), BoutError> {
        self.arena()
            .validate()
            .map_err(|_| BoutError::BadConfig("arena"))?;
        if self.fighters <= SEATS {
            return Err(BoutError::BadConfig("fighters"));
        }
        if self.hold == 0 {
            return Err(BoutError::BadConfig("hold"));
        }
        let drop = self.drop;
        if drop.seat >= SEATS || drop.at == 0 || drop.at >= drop.back {
            return Err(BoutError::BadConfig("drop"));
        }
        Ok(())
    }

    /// The arena config the bout plays on.
    #[must_use]
    pub fn arena(&self) -> ArenaConfig {
        ArenaConfig {
            seed: self.seed,
            ticks: self.ticks,
            fighters: self.fighters,
            half_extent_m: self.half_extent_m,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }
}

/// Human-shaped input tape for one seat: reaction dwell wishes from the
/// shared [`crate::scale`] script, attack pulses on even epochs, block
/// pulses on some odd ones, facing along the held wish.
struct Script {
    seed: u64,
    seat: u8,
    slot: BodyId,
    epoch: u32,
    dwell_left: u32,
    fresh: bool,
    wish: (i32, i32),
}

impl Script {
    fn new(seed: u64, seat: u8, slot: BodyId) -> Self {
        Self {
            seed,
            seat,
            slot,
            epoch: 0,
            dwell_left: human_dwell(seed, seat, 0),
            fresh: true,
            wish: human_wish(seed, seat, 0),
        }
    }

    fn next(&mut self, tick: u32) -> BodyInput {
        if self.dwell_left == 0 {
            self.epoch += 1;
            self.dwell_left = human_dwell(self.seed, self.seat, self.epoch);
            self.wish = human_wish(self.seed, self.seat, self.epoch);
            self.fresh = true;
        }
        self.dwell_left -= 1;
        let pulse = std::mem::take(&mut self.fresh);
        let axis = |raw: i32| i8::try_from((raw * 100 / 1024).clamp(-100, 100)).unwrap_or(0);
        let heading = Fixed::atan2(
            Fixed::from_int(i64::from(self.wish.1)),
            Fixed::from_int(i64::from(self.wish.0)),
        );
        let facing = (heading * Fixed::from_int(18_000) / Fixed::PI)
            .to_int()
            .clamp(-18_000, 18_000);
        let buttons = match (pulse, self.epoch % 2, self.epoch % 7) {
            (true, 0, _) => Buttons(ATTACK),
            (true, _, 3) => Buttons(BLOCK),
            _ => Buttons::NONE,
        };
        BodyInput {
            tick,
            fighter: self.slot,
            mx: axis(self.wish.0),
            mz: axis(self.wish.1),
            facing: i16::try_from(facing).unwrap_or(0),
            buttons,
        }
    }
}

/// Dedicated-server side of the bout: owns the authority, the input inbox,
/// the stall watch and the rejoin path.
struct Host {
    server: HeadlessServer,
    slots: [u8; SEAT_COUNT],
    tokens: [u64; SEAT_COUNT],
    inbox: BTreeMap<(u32, u8), BodyInput>,
    miss: [u32; SEAT_COUNT],
    ticks: u32,
    ledger: HostLedger,
}

/// What the server saw of the drop and the rejoin.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct HostLedger {
    /// Authored tick at which the stall watch dropped the seat.
    pub stalled_at: Option<u32>,
    /// Earliest server tick the supervisor allowed a reclaim.
    pub not_before: u64,
    /// Bout step at which the seat reclaimed its slot.
    pub reclaimed_at: Option<u32>,
    /// Ticks authored while the dropped seat's slot was AI-held.
    pub ai_cover_ticks: u32,
    /// AI-filled slots when both humans play (the lobby's AI fill).
    pub ai_base: usize,
    /// Most AI-filled slots on any tick (the 6 AI plus the dropped human).
    pub ai_peak: usize,
    /// Catch-up packets sent on the rejoin stream.
    pub catchup_packets: u32,
    /// Catch-up bytes sent on the rejoin stream.
    pub catchup_bytes: u64,
    /// Supervisor heals applied at tick boundaries.
    pub heals_applied: u32,
    /// Join requests the server refused.
    pub refused: u32,
}

impl Host {
    fn new(
        arena: ArenaConfig,
        hold: u32,
        slots: [u8; SEAT_COUNT],
        tokens: [u64; SEAT_COUNT],
    ) -> Result<Self, BoutError> {
        let mut server = HeadlessServer::new(arena)?;
        server.set_hold_floor(hold);
        for (seat, token) in tokens.iter().enumerate() {
            let slot = server.connect(*token)?;
            if slot != slots[seat] {
                return Err(BoutError::BadConfig("slots"));
            }
        }
        Ok(Self {
            server,
            slots,
            tokens,
            inbox: BTreeMap::new(),
            miss: [0; SEAT_COUNT],
            ticks: arena.ticks,
            ledger: HostLedger {
                ai_base: usize::from(arena.fighters.saturating_sub(SEATS)),
                ..HostLedger::default()
            },
        })
    }

    fn on_frame<W: Wire>(&mut self, seat: u8, frame: Frame, wire: &mut W, step: u32) {
        match frame {
            Frame::Datagram(bytes) => self.stage_inputs(seat, &bytes, step),
            Frame::Stream(bytes) => self.on_control(seat, &bytes, wire, step),
        }
    }

    /// Stage a client input window: the server's guard screens it (foreign
    /// slots, stale, late, far-future and rewritten ticks are refused and
    /// counted per seat, repeats from the redundancy cover collapse) and the
    /// admitted inputs wait here for their tick.
    fn stage_inputs(&mut self, seat: u8, bytes: &[u8], step: u32) {
        let Some(token) = self.tokens.get(usize::from(seat)).copied() else {
            return;
        };
        let Ok(admitted) = self.server.offer(token, bytes, step) else {
            return;
        };
        for input in admitted {
            self.inbox.entry((input.tick, seat)).or_insert(input);
        }
    }

    /// Rejoin: reclaim the seat's sticky slot, grant it, then stream the
    /// authoritative history the client missed.
    fn on_control<W: Wire>(&mut self, seat: u8, bytes: &[u8], wire: &mut W, step: u32) {
        if bytes.len() < 2 || u16::from_le_bytes([bytes[0], bytes[1]]) != LOBBY_MAGIC {
            return;
        }
        let Ok(Msg::JoinRequest { token, build }) = Msg::decode(bytes) else {
            return;
        };
        let to = Peer::Client(seat);
        let known = self.tokens.get(usize::from(seat)).copied() == Some(token);
        if !known || build != BOUT_BUILD {
            self.ledger.refused += 1;
            let refuse = Msg::JoinRefuse {
                token,
                reason: crate::lobby::RefuseReason::BadInput,
                want: BOUT_BUILD,
                got: build,
            };
            if let Ok(bytes) = refuse.encode() {
                wire.stream(Peer::Server, to, &bytes);
            }
            return;
        }
        if !self.server.is_connected(token) && self.server.reclaim_slot(token).is_err() {
            self.ledger.refused += 1;
            return;
        }
        let Ok(snapshot) = self.server.resync_via_supervisor(token) else {
            self.ledger.refused += 1;
            return;
        };
        let Ok(packets) = resync_packets(&snapshot, self.server.checkpoints()) else {
            self.ledger.refused += 1;
            return;
        };
        let grant = Msg::JoinGrant {
            token,
            slot: self.slots[usize::from(seat)],
            fighters: self.server.config().fighters,
            build: BOUT_BUILD,
        };
        if let Ok(bytes) = grant.encode() {
            wire.stream(Peer::Server, to, &bytes);
        }
        for bytes in &packets {
            wire.stream(Peer::Server, to, bytes);
            self.ledger.catchup_packets += 1;
            self.ledger.catchup_bytes += wire_len(bytes);
        }
        self.miss[usize::from(seat)] = 0;
        self.ledger.reclaimed_at.get_or_insert(step);
    }

    /// Author every tick whose hold has run out, then broadcast its window.
    /// The hold is the server's lag window: the bout's floor, widened to the
    /// slowest connected seat's measured lag so its inputs land on time.
    fn author<W: Wire>(&mut self, wire: &mut W, step: u32) -> Result<(), BoutError> {
        while self.server.tick_now() < self.ticks
            && step >= self.server.tick_now() + self.server.hold_ticks()
        {
            let tick = self.server.tick_now();
            for seat in 0..SEATS {
                let token = self.tokens[usize::from(seat)];
                let input = self.inbox.remove(&(tick, seat));
                if !self.server.is_connected(token) {
                    self.ledger.ai_cover_ticks += 1;
                    continue;
                }
                match input {
                    Some(input) => {
                        self.server.submit(token, input)?;
                        self.miss[usize::from(seat)] = 0;
                    }
                    None => self.miss[usize::from(seat)] += 1,
                }
            }
            let (checkpoint, delta) = self.server.tick()?;
            let filled = self.server.ai_filled_last_tick();
            self.ledger.ai_peak = self.ledger.ai_peak.max(filled);
            self.watch_stalls(tick)?;
            self.ledger.heals_applied +=
                u32::try_from(self.server.apply_heal_boundary(u64::from(tick)).len())
                    .unwrap_or(u32::MAX);
            self.inbox.retain(|(staged, _), _| *staged > tick);
            self.broadcast(wire, checkpoint, &delta.tape);
        }
        Ok(())
    }

    /// Drop any seat that went quiet for [`STALL_TICKS`] authored ticks: the
    /// supervised disconnect hands its slot to the AI until it reclaims.
    fn watch_stalls(&mut self, tick: u32) -> Result<(), BoutError> {
        for seat in 0..SEATS {
            let token = self.tokens[usize::from(seat)];
            if self.miss[usize::from(seat)] < STALL_TICKS || !self.server.is_connected(token) {
                continue;
            }
            let plan = self.server.disconnect_via_supervisor(token)?;
            self.ledger.stalled_at.get_or_insert(tick);
            self.ledger.not_before = plan.not_before_tick;
        }
        Ok(())
    }

    fn broadcast<W: Wire>(&self, wire: &mut W, checkpoint: Checkpoint, tape: &[BodyInput]) {
        let window = redundancy_window(tape, checkpoint.tick, STATE_COVER);
        let Ok(bytes) = encode_state(checkpoint, &window) else {
            return;
        };
        for seat in 0..SEATS {
            if self.server.is_connected(self.tokens[usize::from(seat)]) {
                wire.datagram(Peer::Server, Peer::Client(seat), &bytes);
            }
        }
    }

    /// After the last tick the tip window keeps going out so a lost final
    /// datagram cannot strand a client one tick short.
    fn repeat_tip<W: Wire>(&self, wire: &mut W) {
        if self.server.tick_now() < self.ticks {
            return;
        }
        if let Some(snapshot) = self.server.snapshot() {
            let tip = Checkpoint {
                tick: snapshot.tick,
                hash: snapshot.hash,
            };
            self.broadcast(wire, tip, &snapshot.tape);
        }
    }
}

/// Client-side counters of one seat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeatStats {
    /// Server states applied (one batch per step at most).
    pub applied: u32,
    /// Server states the client refused.
    pub rejected: u32,
    /// Rollback depth per live state applied, in ticks the client ran ahead of
    /// its earliest wrong tick, one bucket per tick.
    pub rollback: [u32; ROLLBACK_BUCKETS],
    /// Deepest rollback taken.
    pub rollback_max: u32,
    /// Applied states deeper than [`RESIM_CAP_TICKS`].
    pub over_cap: u32,
    /// Adaptive input delay in ticks this seat held its predictions back
    /// (MP24: round-trip worst minus the 8-tick window, pure function of
    /// `hold`; LAN 0, WAN 11). Logged, never hashed:
    /// [`BoutReport::digest`] skips it so LAN digests stay byte-identical.
    pub input_delay: u32,
    /// Catch-up (stream) batches applied after a rejoin.
    pub catchups: u32,
    /// Deepest catch-up resimulation, in ticks: the resync, not a rollback.
    pub resync_depth: u32,
    /// Bout step of the last catch-up batch applied.
    pub last_catchup_step: Option<u32>,
    /// Resync requests the client raised itself on a gap in the live windows.
    pub gap_resyncs: u32,
    /// Whether the rejoin grant arrived.
    pub granted: bool,
    /// Packet delivery measured from acks: how many of this seat's input
    /// ticks the server counted, and the loss of the live state datagrams.
    /// Logged, never hashed, like `input_delay`.
    pub delivery: DeliveryStats,
    /// Largest clock slew the pacer ran with, in permille of the tick rate
    /// (MP26: the cap is 5 percent, 50 permille). Logged, never hashed, like
    /// `input_delay`.
    pub max_slew_permille: i32,
    /// Frames the paced clock ran off the nominal one-tick rate: held frames
    /// and run-up ticks both count. Logged, never hashed.
    pub slewed_frames: u32,
    /// Bout ticks the client never predicted: the frame-skip guard. Zero by
    /// construction, because the pacer never runs past the nominal path.
    /// Logged, never hashed.
    pub skipped_frames: u32,
}

impl SeatStats {
    const fn new() -> Self {
        Self {
            applied: 0,
            rejected: 0,
            rollback: [0; ROLLBACK_BUCKETS],
            rollback_max: 0,
            over_cap: 0,
            input_delay: 0,
            catchups: 0,
            resync_depth: 0,
            last_catchup_step: None,
            gap_resyncs: 0,
            granted: false,
            delivery: DeliveryStats::EMPTY,
            max_slew_permille: 0,
            slewed_frames: 0,
            skipped_frames: 0,
        }
    }

    /// Nearest-rank rollback depth percentile in ticks over applied states.
    #[must_use]
    pub fn rollback_pct(&self, pct: u32) -> u32 {
        let total: u64 = self.rollback.iter().map(|count| u64::from(*count)).sum();
        if total == 0 {
            return 0;
        }
        let rank = (total * u64::from(pct.clamp(1, 100))).div_ceil(100).max(1);
        let mut seen = 0u64;
        for (depth, count) in self.rollback.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                return u32::try_from(depth).unwrap_or(u32::MAX);
            }
        }
        0
    }
}

/// Contiguous input knowledge of one client: the newest tick up to which it
/// holds every authoritative input. A live state window covers
/// [`STATE_COVER`] ticks, so it extends the knowledge only when it overlaps
/// or touches it; history streamed from tick zero extends it unconditionally.
/// A checkpoint past a gap is never applied: the session would mark the tick
/// done with inputs missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Coverage {
    through: i64,
}

impl Coverage {
    const fn new() -> Self {
        Self { through: -1 }
    }

    /// Fold one packet's checkpoint tick; `false` means a gap sits before it.
    fn extend(&mut self, tick: u32, history: bool) -> bool {
        let cover = i64::try_from(STATE_COVER).unwrap_or(0);
        let start = (i64::from(tick) - (cover - 1)).max(0);
        if history || start <= self.through + 1 {
            self.through = self.through.max(i64::from(tick));
            true
        } else {
            false
        }
    }
}

/// One human seat: a predicting client plus its input tape. The seat paces
/// its prediction loop against the peer: the 30-frame advantage window
/// (MP26, ggrs `time_sync.rs`) and the clock pacer (lightyear `sync.rs`).
struct Seat {
    seat: u8,
    token: u64,
    client: Client,
    delivery: InputDelivery,
    script: Script,
    predicted: Vec<BodyInput>,
    known: BTreeMap<(u32, u8), BodyInput>,
    top: Option<Checkpoint>,
    via_stream: bool,
    coverage: Coverage,
    gap: bool,
    resync_after: u32,
    burst_left: u32,
    rejoin_at: Option<u32>,
    delay: u32,
    ticks: u32,
    /// The peer seat's slot: the frontier the advantage window compares.
    peer_slot: u8,
    /// This seat's own slot (the frontier sample of the local side).
    slot: u8,
    /// Newest tick the authority took this seat's input for.
    local_frontier: u32,
    /// Newest tick the authority took the peer seat's input for.
    peer_frontier: u32,
    /// The 30-frame window of the two seats' confirmed frontiers.
    window: AdvantageWindow,
    /// The clock pacer the window drives.
    pacer: ClockPacer,
    stats: SeatStats,
}

impl Seat {
    fn new(
        arena: ArenaConfig,
        seat: u8,
        slot: u8,
        peer_slot: u8,
        token: u64,
        rejoin_at: Option<u32>,
    ) -> Result<Self, BoutError> {
        let id = BodyId::new(slot).ok_or(BoutError::BadConfig("slot"))?;
        Ok(Self {
            seat,
            token,
            client: Client::new(arena, id)?,
            delivery: InputDelivery::new(id),
            script: Script::new(arena.seed, seat, id),
            predicted: Vec::new(),
            known: BTreeMap::new(),
            top: None,
            via_stream: false,
            coverage: Coverage::new(),
            gap: false,
            resync_after: 0,
            burst_left: 0,
            rejoin_at,
            delay: 0,
            ticks: arena.ticks,
            slot,
            peer_slot,
            local_frontier: 0,
            peer_frontier: 0,
            window: AdvantageWindow::new(),
            pacer: ClockPacer::new(),
            stats: SeatStats::new(),
        })
    }

    /// Predict locally plus send the redundancy window; the seat that was
    /// dropped asks to rejoin when its link is back.
    ///
    /// MP24 adaptive input delay: the send side authors tick `step` at step
    /// `step` (unchanged wire timing, so the server tape never moves), while
    /// the predict side replays tick `step - delay` at step `step`. The
    /// client runs `delay` ticks behind real time, so the gap to the
    /// authoritative truth shrinks by `delay` and every live resimulation
    /// lands inside [`RESIM_CAP_TICKS`]. `delay` is a pure function of
    /// `hold` (see `scale::adaptive_input_delay_for_hold`); LAN stays 0 and
    /// replays byte-identical.
    fn play<W: Wire>(&mut self, step: u32, wire: &mut W) -> Result<(), BoutError> {
        if step < self.ticks {
            let input = self.script.next(step);
            self.predicted.push(input);
            // Steps rise by one, so the record cannot be refused; were it
            // refused the counters would only miss a tick, never the bout.
            let _ = self.delivery.on_input_sent(step);
            if self.rejoin_at == Some(step) {
                self.ask_to_join(step, wire)?;
                self.burst_left = REJOIN_BURST_STEPS;
            }
            let cover = if self.burst_left > 0 {
                self.burst_left -= 1;
                MAX_INPUTS_PER_PACKET
            } else {
                REDUNDANCY
            };
            let window = redundancy_window(&self.predicted, step, cover);
            wire.datagram(
                Peer::Client(self.seat),
                Peer::Server,
                &encode_input(step, &window)?,
            );
        } else if self.rejoin_at == Some(step) {
            self.ask_to_join(step, wire)?;
            self.burst_left = REJOIN_BURST_STEPS;
        }
        if step >= self.delay && self.client.tick_now() < self.ticks {
            // The paced clock (MP26): the prediction rate is trimmed by the
            // clock slew the 30-frame advantage window asks for, and the
            // pacer holds the client's tip inside the band its input delay
            // gave it. The send side is untouched, so the server tape and
            // the golden never move.
            let tip = self.client.tick_now();
            let nominal = step.saturating_sub(self.delay);
            let error = i32::try_from(tip.saturating_sub(nominal)).unwrap_or(i32::MAX);
            let available = u32::try_from(
                self.predicted
                    .len()
                    .saturating_sub(usize::try_from(tip).unwrap_or(usize::MAX)),
            )
            .unwrap_or(0);
            let slew =
                crate::delay::ClockSlew::for_advantage(self.window.average_frame_advantage());
            let pace = self.pacer.pace_within(&slew, error, available);
            for _ in 0..pace {
                let tick = self.client.tick_now();
                let Some(input) = self
                    .predicted
                    .get(usize::try_from(tick).unwrap_or(usize::MAX))
                    .copied()
                else {
                    // The seat holds no authored input for the next tick, so
                    // the frame is held rather than skipped: the tick stays
                    // covered by the authority's AI-fill the same way a
                    // lagging prediction does, and the stream never jumps
                    // over a tick.
                    break;
                };
                self.client.predict(input)?;
            }
        }
        Ok(())
    }

    /// Ask the server for a (re)join: it grants the slot and streams the
    /// authoritative history this client is missing.
    fn ask_to_join<W: Wire>(&mut self, step: u32, wire: &mut W) -> Result<(), BoutError> {
        let request = Msg::JoinRequest {
            token: self.token,
            build: BOUT_BUILD,
        };
        wire.stream(
            Peer::Client(self.seat),
            Peer::Server,
            &request.encode().map_err(BoutError::Lobby)?,
        );
        self.resync_after = step + RESYNC_WAIT_STEPS;
        Ok(())
    }

    /// A live window that skipped ticks raises one resync request, then
    /// waits [`RESYNC_WAIT_STEPS`] before it may ask again.
    fn heal_gap<W: Wire>(&mut self, step: u32, wire: &mut W) -> Result<(), BoutError> {
        if std::mem::take(&mut self.gap) && step >= self.resync_after {
            self.stats.gap_resyncs += 1;
            self.ask_to_join(step, wire)?;
        }
        Ok(())
    }

    fn on_frame(&mut self, frame: Frame) {
        let (Frame::Datagram(bytes) | Frame::Stream(bytes)) = &frame;
        let is_stream = matches!(frame, Frame::Stream(_));
        let magic = bytes
            .get(..2)
            .map(|head| u16::from_le_bytes([head[0], head[1]]));
        if magic == Some(LOBBY_MAGIC) {
            if let Ok(Msg::JoinGrant { token, .. }) = Msg::decode(bytes) {
                self.stats.granted |= token == self.token;
            }
            return;
        }
        if magic != Some(WIRE_MAGIC) {
            return;
        }
        let Ok(packet) = decode_state(bytes) else {
            return;
        };
        if !is_stream {
            // A live window names the inputs the server took for its last
            // STATE_COVER ticks, the ack of this seat's input stream. A
            // checkpoint from the future is refused and changes nothing.
            let _ = self
                .delivery
                .on_state(packet.checkpoint.tick, &packet.inputs, STATE_COVER);
        }
        for input in packet.inputs {
            let fighter = input.fighter.get();
            // The confirmed frontier of each seat (MP26): the newest tick
            // the authority took that seat's input for. Both seats read the
            // same broadcast, so the two windows are exact mirrors.
            if fighter == self.slot {
                self.local_frontier = self.local_frontier.max(input.tick);
            } else if fighter == self.peer_slot {
                self.peer_frontier = self.peer_frontier.max(input.tick);
            }
            self.known.insert((input.tick, fighter), input);
        }
        if !self.coverage.extend(packet.checkpoint.tick, is_stream) {
            self.gap = true;
            return;
        }
        if self.top.is_none_or(|top| packet.checkpoint.tick > top.tick) {
            self.top = Some(packet.checkpoint);
        }
        self.via_stream |= is_stream;
    }

    /// Apply the newest checkpoint heard this step: name the mispredicts,
    /// record how far the client ran ahead of the earliest one, resimulate.
    fn apply(&mut self, step: u32) {
        // The 30-frame advantage window (MP26): one sample per frame, taken
        // from the two seats' confirmed frontiers. Both seats read the same
        // broadcast, so the two windows are exact mirrors and their slews are
        // equal and opposite.
        self.window.advance(
            i32::try_from(self.local_frontier).unwrap_or(i32::MAX),
            i32::try_from(self.peer_frontier).unwrap_or(i32::MAX),
        );
        let Some(checkpoint) = self.top.take() else {
            return;
        };
        let catchup = std::mem::take(&mut self.via_stream);
        if self
            .client
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
        match self.client.on_server_state(checkpoint, &delta) {
            Ok(mispredicts) => {
                let depth = mispredicts
                    .first()
                    .map_or(0, |first| self.client.tick_now().saturating_sub(*first));
                if catchup {
                    // Streamed history is a resync, not a rollback: counted
                    // apart so the rollback window is sized by live play.
                    self.stats.catchups += 1;
                    self.stats.last_catchup_step = Some(step);
                    self.stats.resync_depth = self.stats.resync_depth.max(depth);
                } else {
                    let bucket = usize::try_from(depth)
                        .unwrap_or(usize::MAX)
                        .min(ROLLBACK_BUCKETS - 1);
                    self.stats.rollback[bucket] += 1;
                    self.stats.rollback_max = self.stats.rollback_max.max(depth);
                    self.stats.over_cap += u32::from(depth > RESIM_CAP_TICKS);
                }
                self.stats.applied += 1;
                self.client.resimulate();
            }
            Err(_) => self.stats.rejected += 1,
        }
    }
}

/// What one human seat ended with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeatReport {
    /// Seat token.
    pub token: u64,
    /// Arena slot the seat played.
    pub slot: u8,
    /// Newest checkpoint the client holds.
    pub tip: Checkpoint,
    /// Client checkpoints equal the server's tick for tick, full length.
    pub equal: bool,
    /// First tick where client and server disagree; `None` when equal.
    pub first_divergence: Option<u32>,
    /// Ticks the server truth corrected over the whole bout.
    pub mispredict_ticks: u64,
    /// Rollback counters.
    pub stats: SeatStats,
}

/// Combat events on the authoritative tape, split around the drop.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct FightLedger {
    /// Events before the outage began.
    pub before_drop: u32,
    /// Events while the seat was away and the AI held its slot.
    pub during_drop: u32,
    /// Events after the seat reclaimed its slot.
    pub after_rejoin: u32,
    /// Events in each second of the bout (60 ticks), in order.
    pub per_second: Vec<u32>,
}

/// What [`BoutReport::sync_test`] found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyncTestReport {
    /// Ticks rewound and replayed per window.
    pub distance: u32,
    /// Replayed checkpoints compared with the server's.
    pub checked: u32,
    /// Compared checkpoints that disagreed.
    pub mismatches: u32,
    /// Lowest tick that disagreed; `None` when every checkpoint matched.
    pub first_mismatch: Option<u32>,
}

/// The verdict and every posted number of one bout.
#[derive(Clone, PartialEq, Debug)]
pub struct BoutReport {
    /// The bout that ran.
    pub config: BoutConfig,
    /// Human slots the lobby formed.
    pub humans: usize,
    /// AI-filled slots the lobby formed.
    pub ai: usize,
    /// Server tip: the authoritative checkpoint of the last tick.
    pub server_tip: Checkpoint,
    /// Authoritative human inputs on the tape (the dedicated-server tape).
    pub tape_len: usize,
    /// Hash of that tape's canonical bytes.
    pub tape_hash: u64,
    /// The authoritative human inputs, `(tick, fighter)`-sorted.
    pub tape: Vec<BodyInput>,
    /// The tape replays to the server's checkpoints, tick for tick.
    pub replay_equal: bool,
    /// Both seats.
    pub seats: [SeatReport; SEAT_COUNT],
    /// Server-side drop and rejoin ledger.
    pub host: HostLedger,
    /// The server guard's admission receipt of each seat's slot, seat order.
    pub admission: [SlotReceipt; SEAT_COUNT],
    /// Lag receipt of every attack press the seats sent, in arrival order.
    pub hits: LagTape,
    /// Ticks the server held each tick open at the end (its lag window).
    pub hold_ticks: u32,
    /// Combat around the drop.
    pub fight: FightLedger,
    /// Every directed link, measured.
    pub links: Vec<LinkReport>,
    /// Server checkpoint of every tick, in order.
    pub checkpoints: Vec<Checkpoint>,
}

impl BoutReport {
    /// True when blows landed both before the drop and after the rejoin:
    /// the fight was live at both ends of the outage.
    #[must_use]
    pub fn mid_fight(&self) -> bool {
        self.fight.before_drop > 0 && self.fight.after_rejoin > 0
    }

    /// The acceptance verdict: the full match played mid-fight, both clients
    /// hold the server's checkpoints in full, the tape replays to the same
    /// tip, and the dropped seat was stalled out, reclaimed and granted.
    #[must_use]
    pub fn converged(&self) -> bool {
        let dropped = &self.seats[usize::from(self.config.drop.seat)];
        self.mid_fight()
            && self.checkpoints.len() == usize::try_from(self.config.ticks).unwrap_or(usize::MAX)
            && self.replay_equal
            && self.seats.iter().all(|seat| seat.equal)
            && self.host.stalled_at.is_some()
            && self.host.reclaimed_at.is_some()
            && dropped.stats.granted
    }

    /// GGRS-style SyncTest over the played tape: after every tick, rewind
    /// `distance` ticks to a snapshot, replay the authoritative inputs and
    /// compare each replayed checkpoint with the server's. Any mismatch is a
    /// determinism bug, never a tolerance; the report names the lowest tick.
    ///
    /// # Errors
    ///
    /// [`BoutError::BadConfig`] when `distance` is zero or the arena cannot
    /// snapshot (the bout plays the v1 rules, which can).
    pub fn sync_test(&self, distance: u32) -> Result<SyncTestReport, BoutError> {
        if distance == 0 {
            return Err(BoutError::BadConfig("distance"));
        }
        let arena = self.config.arena();
        let mut main = LiveMatch::new(&arena).ok_or(BoutError::BadConfig("arena"))?;
        let mut trail = LiveMatch::new(&arena).ok_or(BoutError::BadConfig("arena"))?;
        let mut report = SyncTestReport {
            distance,
            checked: 0,
            mismatches: 0,
            first_mismatch: None,
        };
        let mut compare = |tick: u32, got: Checkpoint| {
            report.checked += 1;
            if self.checkpoints.get(tick as usize) != Some(&got) {
                report.mismatches += 1;
                report.first_mismatch = report
                    .first_mismatch
                    .map_or(Some(tick), |t| Some(t.min(tick)));
            }
        };
        for tick in 0..self.config.ticks {
            let step = main
                .step(tick_slice(&self.tape, tick))
                .ok_or(BoutError::BadConfig("tape"))?;
            compare(tick, step.checkpoint);
            if tick + 1 < distance {
                continue;
            }
            let snapshot = trail.snapshot().ok_or(BoutError::BadConfig("snapshot"))?;
            let mut replay =
                LiveMatch::from_snapshot(snapshot).ok_or(BoutError::BadConfig("snapshot"))?;
            for at in trail.tick()..=tick {
                let step = replay
                    .step(tick_slice(&self.tape, at))
                    .ok_or(BoutError::BadConfig("tape"))?;
                compare(at, step.checkpoint);
            }
            let at = trail.tick();
            trail
                .step(tick_slice(&self.tape, at))
                .ok_or(BoutError::BadConfig("tape"))?;
        }
        Ok(report)
    }

    /// Fold every deterministic number into one digest: two runs of the same
    /// config over the same wire must agree on it.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(self.config.seed);
        h.write_u32(self.config.ticks);
        h.write_u64(self.server_tip.hash);
        h.write_u64(self.tape_hash);
        h.write_u64(u64::try_from(self.tape_len).unwrap_or(u64::MAX));
        h.write_bool(self.replay_equal);
        for cp in &self.checkpoints {
            h.write_u64(cp.hash);
        }
        for seat in &self.seats {
            h.write_u64(seat.tip.hash);
            h.write_bool(seat.equal);
            h.write_u64(seat.mispredict_ticks);
            let s = &seat.stats;
            for word in [
                s.applied,
                s.rejected,
                s.rollback_max,
                s.over_cap,
                s.catchups,
                s.resync_depth,
            ] {
                h.write_u32(word);
            }
            h.write_bool(s.granted);
            h.write_u32(s.last_catchup_step.unwrap_or(u32::MAX));
            h.write_u32(s.gap_resyncs);
            for count in s.rollback {
                h.write_u32(count);
            }
        }
        let host = &self.host;
        h.write_u32(host.stalled_at.unwrap_or(u32::MAX));
        h.write_u32(host.reclaimed_at.unwrap_or(u32::MAX));
        for word in [
            host.ai_cover_ticks,
            host.catchup_packets,
            host.heals_applied,
        ] {
            h.write_u32(word);
        }
        h.write_u64(host.catchup_bytes);
        for receipt in &self.admission {
            h.write_u32(receipt.accepted);
            h.write_u32(receipt.repeats);
            for count in receipt.refused {
                h.write_u32(count);
            }
        }
        h.write_u64(self.hits.hash());
        h.write_u32(self.hold_ticks);
        for word in [
            self.fight.before_drop,
            self.fight.during_drop,
            self.fight.after_rejoin,
        ] {
            h.write_u32(word);
        }
        for count in &self.fight.per_second {
            h.write_u32(*count);
        }
        for link in &self.links {
            let s = &link.stats;
            for word in [
                s.sent,
                s.dropped,
                s.outage,
                s.delivered,
                s.reordered,
                s.streams,
            ] {
                h.write_u32(word);
            }
            h.write_u64(s.bytes);
            for count in s.delay {
                h.write_u32(count);
            }
        }
        h.finish()
    }

    /// Ticks from the scripted link return to the last catch-up batch the
    /// dropped seat applied: the reconnect time. `None` without a rejoin.
    #[must_use]
    pub fn rejoin_ticks(&self) -> Option<u32> {
        let dropped = &self.seats[usize::from(self.config.drop.seat)];
        dropped
            .stats
            .last_catchup_step
            .map(|step| step.saturating_sub(self.config.drop.back))
    }

    /// Link counters folded by direction: all uplinks, all downlinks.
    #[must_use]
    pub fn totals(&self) -> (LinkStats, LinkStats) {
        let (mut up, mut down) = (LinkStats::zero(), LinkStats::zero());
        for link in &self.links {
            match link.to {
                Peer::Server => up.absorb(&link.stats),
                Peer::Client(_) => down.absorb(&link.stats),
            }
        }
        (up, down)
    }
}

impl std::fmt::Display for BoutReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (up, down) = self.totals();
        write!(
            f,
            "BOUT seed={} fighters={} ({}h+{}ai) ticks={} tip={:016x} tape={}in/{:016x} replay={} \
             seats={}/{} drop={}..{} stalled={:?} reclaimed={:?} ai_cover={} catchup={}pk/{}B \
             fight={}/{}/{} up_loss={}bp down_loss={}bp in_loss={}/{}pm rollback_max={}/{} delay={}/{} digest={:016x} converged={}",
            self.config.seed,
            self.config.fighters,
            self.humans,
            self.ai,
            self.config.ticks,
            self.server_tip.hash,
            self.tape_len,
            self.tape_hash,
            self.replay_equal,
            self.seats[0].equal,
            self.seats[1].equal,
            self.config.drop.at,
            self.config.drop.back,
            self.host.stalled_at,
            self.host.reclaimed_at,
            self.host.ai_cover_ticks,
            self.host.catchup_packets,
            self.host.catchup_bytes,
            self.fight.before_drop,
            self.fight.during_drop,
            self.fight.after_rejoin,
            up.loss_bp(),
            down.loss_bp(),
            self.seats[0].stats.delivery.input_loss_permille,
            self.seats[1].stats.delivery.input_loss_permille,
            self.seats[0].stats.rollback_max,
            self.seats[1].stats.rollback_max,
            self.seats[0].stats.input_delay,
            self.seats[1].stats.input_delay,
            self.digest(),
            self.converged(),
        )
    }
}

/// Form the fight in the lobby: both seats join and ready up, the rest fill
/// with AI. Returns each seat's slot.
fn form_match(cfg: &BoutConfig) -> Result<[u8; SEAT_COUNT], BoutError> {
    let mut lobby = Lobby::new(BOUT_BUILD, cfg.fighters, cfg.seed)?;
    let mut slots = [0u8; SEAT_COUNT];
    for (seat, token) in cfg.tokens.iter().enumerate() {
        slots[seat] = lobby.join(*token, BOUT_BUILD)?;
        lobby.set_ready(*token, true)?;
    }
    lobby.ai_fill();
    let start = lobby.try_start()?;
    let ai = usize::try_from(start.ai_mask.count_ones()).unwrap_or(usize::MAX);
    if start.seed != cfg.seed
        || start.fighters != cfg.fighters
        || lobby.human_count() != usize::from(SEATS)
        || ai != usize::from(cfg.fighters - SEATS)
    {
        return Err(BoutError::BadConfig("lobby"));
    }
    Ok(slots)
}

/// Play one bout over `wire`: the server holds each tick open for late
/// inputs, the clients predict, the wire loses and delays, one seat drops
/// mid-fight and rejoins, and the report says whether everything converged.
///
/// # Errors
///
/// [`BoutError`] when the config, lobby, server or a client refuses; a bout
/// that runs but fails to converge is a report with `converged() == false`.
pub fn run_bout<W: Wire>(cfg: &BoutConfig, wire: &mut W) -> Result<BoutReport, BoutError> {
    let mut part = BoutPart::new(cfg, true, &[0, 1])?;
    for step in 0..part.steps() {
        part.step(step, wire)?;
        wire.tick();
    }
    part.report(&part.outcomes(), wire.report())
}

/// What one seat's client ends a bout with: its checkpoint chain and
/// counters. A seat played on another machine ships this to the host.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SeatOutcome {
    /// Seat index, below [`SEATS`].
    pub seat: u8,
    /// Seat token.
    pub token: u64,
    /// Arena slot the seat played.
    pub slot: u8,
    /// Every checkpoint the client holds, in tick order.
    pub checkpoints: Vec<Checkpoint>,
    /// Ticks the server truth corrected over the whole bout.
    pub mispredict_ticks: u64,
    /// Rollback counters.
    pub stats: SeatStats,
}

/// The parts of one bout played in this process: the host when it runs
/// here, plus the seats that play here. [`run_bout`] plays every part; a
/// bout split across two machines plays the host on one and a seat on the
/// other, each over its own side of the wire, in the same step order.
pub struct BoutPart {
    cfg: BoutConfig,
    host: Option<Host>,
    seats: Vec<Seat>,
}

impl BoutPart {
    /// Form the match in the lobby, then open the host (when `host`) and
    /// every seat in `seats`, in that order.
    ///
    /// # Errors
    ///
    /// [`BoutError`] when the config, lobby, server or a client refuses;
    /// [`BoutError::BadConfig`] when `seats` repeats a seat or names one
    /// past [`SEATS`].
    pub fn new(cfg: &BoutConfig, host: bool, seats: &[u8]) -> Result<Self, BoutError> {
        cfg.validate()?;
        let slots = form_match(cfg)?;
        let arena = cfg.arena();
        let host = if host {
            Some(Host::new(arena, cfg.hold, slots, cfg.tokens)?)
        } else {
            None
        };
        let mut opened: Vec<Seat> = Vec::with_capacity(seats.len());
        let delay = crate::scale::adaptive_input_delay_for_hold(cfg.hold);
        for &seat in seats {
            if seat >= SEATS || opened.iter().any(|open| open.seat == seat) {
                return Err(BoutError::BadConfig("seats"));
            }
            let index = usize::from(seat);
            let rejoin = (cfg.drop.seat == seat).then_some(cfg.drop.back);
            let peer_slot = if seat == 0 { slots[1] } else { slots[0] };
            let mut opened_seat = Seat::new(
                arena,
                seat,
                slots[index],
                peer_slot,
                cfg.tokens[index],
                rejoin,
            )?;
            opened_seat.delay = delay;
            opened_seat.stats.input_delay = delay;
            opened.push(opened_seat);
        }
        Ok(Self {
            cfg: *cfg,
            host,
            seats: opened,
        })
    }

    /// Bout steps: every authored tick, the last hold, then the drain. The
    /// adaptive input delay holds predictions back, so the run also covers
    /// the delay: delayed seats predict tick `ticks - 1` at step
    /// `ticks + delay - 1`.
    #[must_use]
    pub fn steps(&self) -> u32 {
        let delay = crate::scale::adaptive_input_delay_for_hold(self.cfg.hold);
        self.cfg
            .ticks
            .saturating_add(self.cfg.hold.max(delay))
            .saturating_add(DRAIN_STEPS)
    }

    /// Play one step of the parts held here: the seats predict and send,
    /// the host reads, authors and broadcasts, the seats read and apply.
    /// The caller ticks the wire after.
    ///
    /// # Errors
    ///
    /// [`BoutError`] when the server or a client refuses.
    pub fn step<W: Wire>(&mut self, step: u32, wire: &mut W) -> Result<(), BoutError> {
        for seat in &mut self.seats {
            seat.play(step, wire)?;
        }
        if let Some(host) = self.host.as_mut() {
            for (from, frame) in wire.recv(Peer::Server) {
                if let Peer::Client(seat) = from {
                    host.on_frame(seat, frame, wire, step);
                }
            }
            host.author(wire, step)?;
            host.repeat_tip(wire);
        }
        for seat in &mut self.seats {
            for (_, frame) in wire.recv(Peer::Client(seat.seat)) {
                seat.on_frame(frame);
            }
            seat.heal_gap(step, wire)?;
            seat.apply(step);
        }
        Ok(())
    }

    /// What every seat played here ends with, in the order opened.
    #[must_use]
    pub fn outcomes(&self) -> Vec<SeatOutcome> {
        self.seats
            .iter()
            .map(|seat| {
                let mut stats = seat.stats;
                // The paced clock's evidence: the largest slew the pacer ran
                // with and the frames it ran off the nominal rate. Logged,
                // never hashed, like `input_delay`, so the digests stay put.
                stats.max_slew_permille = seat.pacer.max_permille();
                stats.slewed_frames = seat.pacer.slewed_frames();
                // The frame-skip guard: the bout ticks the client's history
                // does not cover. Zero, because the pacer holds a frame
                // rather than skipping the tick it would have run.
                stats.skipped_frames = seat
                    .ticks
                    .saturating_sub(u32::try_from(seat.client.checkpoints().len()).unwrap_or(0));
                SeatOutcome {
                    seat: seat.seat,
                    token: seat.token,
                    slot: seat.client.local().get(),
                    checkpoints: seat.client.checkpoints().to_vec(),
                    mispredict_ticks: seat.client.total_mispredicts(),
                    stats: SeatStats {
                        delivery: seat.delivery.stats(),
                        ..stats
                    },
                }
            })
            .collect()
    }

    /// Fold every seat's outcome and the measured links into the report.
    ///
    /// # Errors
    ///
    /// [`BoutError::BadConfig`] when the host is not played here, or
    /// `seats` is not every seat in seat order.
    pub fn report(
        &self,
        seats: &[SeatOutcome],
        links: Vec<LinkReport>,
    ) -> Result<BoutReport, BoutError> {
        let host = self.host.as_ref().ok_or(BoutError::BadConfig("host"))?;
        let [first, second] = seats else {
            return Err(BoutError::BadConfig("seats"));
        };
        if first.seat != 0 || second.seat != 1 {
            return Err(BoutError::BadConfig("seats"));
        }
        Ok(finish(&self.cfg, host, [first, second], links))
    }
}

/// The loopback fault injector of one bout: `up` is every client-to-server
/// link, `down` every server-to-client link, and the dropped seat's links
/// go dark for the scripted outage. Every machine of a split bout builds
/// the same wire, so each link draws the same stream wherever it runs.
#[must_use]
pub fn loopback_wire(cfg: &BoutConfig, up: LinkProfile, down: LinkProfile) -> LoopbackWire {
    let mut wire = LoopbackWire::new(SEATS, up, down, mix64(cfg.seed ^ 0x5749_5245));
    wire.set_outage(cfg.drop.seat, cfg.drop.at, cfg.drop.back);
    wire
}

/// Run the canonical bout over a seeded loopback fault injector, built by
/// [`loopback_wire`].
///
/// # Errors
///
/// As [`run_bout`].
pub fn run_loopback(
    cfg: &BoutConfig,
    up: LinkProfile,
    down: LinkProfile,
) -> Result<BoutReport, BoutError> {
    run_bout(cfg, &mut loopback_wire(cfg, up, down))
}

fn finish(
    cfg: &BoutConfig,
    host: &Host,
    seats: [&SeatOutcome; SEAT_COUNT],
    links: Vec<LinkReport>,
) -> BoutReport {
    let arena = cfg.arena();
    let truth = host.server.checkpoints();
    let tape = host
        .server
        .snapshot()
        .map(|snap| snap.tape)
        .unwrap_or_default();
    let mut bytes = Vec::with_capacity(tape.len() * 12);
    for input in &tape {
        input.append_bytes(&mut bytes);
    }
    let replay_equal = run(&arena, &BodyTape(tape.clone())) == truth;
    let back = host.ledger.reclaimed_at.unwrap_or(cfg.drop.back);
    let mut fight = FightLedger::default();
    for row in run_events(&arena, &BodyTape(tape.clone())) {
        let count = u32::try_from(row.events.len()).unwrap_or(u32::MAX);
        let second = usize::try_from(row.tick / 60).unwrap_or(usize::MAX);
        if fight.per_second.len() <= second {
            fight.per_second.resize(second + 1, 0);
        }
        fight.per_second[second] += count;
        if row.tick < cfg.drop.at {
            fight.before_drop += count;
        } else if row.tick < back {
            fight.during_drop += count;
        } else {
            fight.after_rejoin += count;
        }
    }
    let seat_report = |seat: &SeatOutcome| {
        let mine = seat.checkpoints.as_slice();
        let equal = mine == truth;
        let split = mine
            .iter()
            .zip(truth)
            .find(|(a, b)| a != b)
            .map(|(a, _)| a.tick)
            .or_else(|| u32::try_from(mine.len().min(truth.len())).ok());
        SeatReport {
            token: seat.token,
            slot: seat.slot,
            tip: mine.last().copied().unwrap_or(NO_CHECKPOINT),
            equal,
            first_divergence: if equal { None } else { split },
            mispredict_ticks: seat.mispredict_ticks,
            stats: seat.stats,
        }
    };
    BoutReport {
        config: *cfg,
        humans: usize::from(SEATS),
        ai: usize::from(cfg.fighters - SEATS),
        server_tip: truth.last().copied().unwrap_or(NO_CHECKPOINT),
        tape_len: tape.len(),
        tape_hash: engine_core::hash_bytes(&bytes),
        tape,
        replay_equal,
        seats: [seat_report(seats[0]), seat_report(seats[1])],
        host: host.ledger,
        admission: host
            .slots
            .map(|slot| host.server.admission()[usize::from(slot)]),
        hits: host.server.hit_receipts().clone(),
        hold_ticks: host.server.hold_ticks(),
        fight,
        links,
        checkpoints: truth.to_vec(),
    }
}

/// Placeholder tip for an empty history.
const NO_CHECKPOINT: Checkpoint = Checkpoint { tick: 0, hash: 0 };

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Short bout for unit tests: same shape, 300 ticks, drop 100..160.
    fn short(seed: u64, profile: LinkProfile) -> BoutConfig {
        BoutConfig {
            ticks: 300,
            drop: DropScript {
                seat: 1,
                at: 100,
                back: 160,
            },
            ..BoutConfig::for_profile(seed, profile)
        }
    }

    #[test]
    fn played_lan_bout_converges_through_drop_and_rejoin() {
        let cfg = short(2040, LinkProfile::LAN);
        let report = run_loopback(&cfg, LinkProfile::LAN, LinkProfile::LAN).expect("bout runs");
        assert_eq!((report.humans, report.ai), (2, 6), "{report}");
        assert!(report.converged(), "{report}");
        assert_eq!(report.checkpoints.len(), 300);
        assert!(
            report.replay_equal,
            "the recorded tape must replay to the tip"
        );
        for seat in &report.seats {
            assert!(
                seat.equal,
                "seat {} diverged at {:?}",
                seat.slot, seat.first_divergence
            );
            assert_eq!(seat.tip, report.server_tip);
        }
        // The drop really happened: stalled out, AI held the slot, reclaimed,
        // caught up from the authoritative history, then played on.
        let host = report.host;
        let stalled = host.stalled_at.expect("stall watch drops the silent seat");
        assert!(
            (99 + STALL_TICKS..=99 + STALL_TICKS + 12).contains(&stalled),
            "stalled out at {stalled}"
        );
        assert!(host.reclaimed_at.expect("reclaimed") >= 160);
        assert!(host.ai_peak > host.ai_base, "the dropped human is AI-held");
        assert!(
            host.ai_cover_ticks >= 15,
            "{} ticks of cover",
            host.ai_cover_ticks
        );
        assert!(host.catchup_packets > 0 && host.catchup_bytes > 0);
        assert!(report.seats[1].stats.granted && report.seats[1].stats.catchups > 0);
        assert!(
            !report.seats[0].stats.granted,
            "the other seat never rejoined"
        );
        // Mid-fight: blows landed before the drop and after the rejoin.
        assert!(report.fight.before_drop > 0, "{report}");
        assert!(report.fight.after_rejoin > 0, "{report}");
        assert!(report.tape_len > 0);
    }

    #[test]
    fn played_sync_test_at_distance_7_finds_no_mismatch_on_three_tapes() {
        // QN11: after every tick rewind 7 ticks, replay the played tape and
        // compare each replayed checkpoint with the server's. Three bouts
        // over the fault profiles, each checked twice.
        for (seed, profile) in [
            (2040, LinkProfile::LAN),
            (7, LinkProfile::WAN),
            (99, LinkProfile::WAN),
        ] {
            let cfg = short(seed, profile);
            let report = run_loopback(&cfg, profile, profile).expect("bout runs");
            assert!(report.tape_len > 0 && report.tape.len() == report.tape_len);
            let first = report.sync_test(SYNC_CHECK_DISTANCE).expect("sync test");
            let again = report.sync_test(SYNC_CHECK_DISTANCE).expect("sync test");
            println!("sync-test seed={seed} {first:?}");
            assert_eq!(first, again, "same tape, same sync report");
            assert_eq!(first.mismatches, 0, "seed {seed}: {first:?}");
            assert_eq!(first.first_mismatch, None);
            let windows = cfg.ticks - SYNC_CHECK_DISTANCE + 1;
            assert_eq!(first.checked, cfg.ticks + windows * SYNC_CHECK_DISTANCE);
        }
    }

    #[test]
    fn played_sync_test_names_the_first_wrong_tick() {
        let cfg = short(2040, LinkProfile::LAN);
        let mut report = run_loopback(&cfg, LinkProfile::LAN, LinkProfile::LAN).expect("bout");
        assert_eq!(
            report.sync_test(0),
            Err(BoutError::BadConfig("distance")),
            "a zero distance checks nothing"
        );
        report.checkpoints[200].hash ^= 1;
        report.checkpoints[150].hash ^= 1;
        let found = report.sync_test(SYNC_CHECK_DISTANCE).expect("sync test");
        assert_eq!(found.first_mismatch, Some(150));
        assert!(found.mismatches >= 2, "{found:?}");
    }

    #[test]
    fn played_wan_bout_converges_over_loss_jitter_and_reorder() {
        let cfg = short(7, LinkProfile::WAN);
        let report = run_loopback(&cfg, LinkProfile::WAN, LinkProfile::WAN).expect("bout runs");
        assert!(report.converged(), "{report}");
        let (up, down) = report.totals();
        assert!(up.dropped > 0 && down.dropped > 0, "the wire must lose");
        assert!(up.reordered + down.reordered > 0, "the wire must reorder");
        assert!(up.delay_max() >= up.delay_pct(50));
        // Lossy links mean corrections, never a different match.
        assert!(report.seats.iter().all(|seat| seat.mispredict_ticks > 0));
    }

    #[test]
    fn played_wan_adaptive_delay_keeps_every_resim_inside_cap() {
        // MP24: the simulated round trip exceeds the 8-tick window (WAN
        // deepest 19 before the fix), so the client delays its own input by
        // ceil(rtt) - 8 and prediction never runs deeper than 8 ticks.
        let cfg = short(7, LinkProfile::WAN);
        let delay = crate::scale::adaptive_input_delay_for_hold(cfg.hold);
        println!("WAN adaptive delay {delay} for hold {}", cfg.hold);
        assert!(delay > 0, "WAN must hold predictions back");
        let report = run_loopback(&cfg, LinkProfile::WAN, LinkProfile::WAN).expect("bout runs");
        assert!(report.converged(), "{report}");
        for seat in &report.seats {
            assert_eq!(
                seat.stats.input_delay, delay,
                "every WAN profile logs its input delay"
            );
            assert_eq!(
                seat.stats.over_cap, 0,
                "every resim inside cap, deepest {}",
                seat.stats.rollback_max
            );
            assert!(
                seat.stats.rollback_max <= RESIM_CAP_TICKS,
                "deepest {} above cap {RESIM_CAP_TICKS}",
                seat.stats.rollback_max
            );
        }
        let lan = short(2040, LinkProfile::LAN);
        assert_eq!(
            crate::scale::adaptive_input_delay_for_hold(lan.hold),
            0,
            "LAN stays undelayed so its digests never move"
        );
    }

    #[test]
    fn played_wan_bout_paces_the_client_clock_inside_the_cap() {
        // MP26: the seat whose inputs reach the authority first runs ahead
        // of its peer, so the 30-frame window trims its clock: the posted
        // slew stays inside 5 percent, the paced clock holds frames instead
        // of skipping them, and the bout still converges.
        let cfg = short(7, LinkProfile::WAN);
        let report = run_loopback(&cfg, LinkProfile::WAN, LinkProfile::WAN).expect("bout runs");
        assert!(report.converged(), "{report}");
        let mut slewed = 0u32;
        for seat in &report.seats {
            assert!(
                seat.stats.max_slew_permille <= crate::delay::MAX_SLEW_PERMILLE,
                "slot {} slewed past the cap: {}",
                seat.slot,
                seat.stats.max_slew_permille
            );
            assert_eq!(
                seat.stats.skipped_frames, 0,
                "slot {} skipped a frame",
                seat.slot
            );
            slewed += seat.stats.slewed_frames;
        }
        assert!(slewed > 0, "the paced clock never trimmed a frame");
    }

    #[test]
    fn adaptive_steps_cover_delay_for_every_hold() {
        // MP24: the bout must run long enough for delayed predictions to
        // finish: steps cover ticks plus the adaptive delay, so a large
        // hold never cuts the last predictions short.
        for hold in [3u32, 10, 30, 60, 100] {
            let cfg = BoutConfig {
                ticks: 300,
                hold,
                ..short(7, LinkProfile::LAN)
            };
            let part = BoutPart::new(&cfg, true, &[0, 1]).expect("part opens");
            let delay = crate::scale::adaptive_input_delay_for_hold(hold);
            assert!(
                part.steps() >= cfg.ticks + delay,
                "hold {hold} steps {} must cover ticks {} plus delay {delay}",
                part.steps(),
                cfg.ticks,
            );
        }
    }

    #[test]
    fn played_bout_part_refuses_bad_seats_and_reports_only_on_the_host() {
        let cfg = short(3, LinkProfile::LAN);
        let seats = BoutError::BadConfig("seats");
        assert_eq!(BoutPart::new(&cfg, true, &[0, 0]).err(), Some(seats));
        assert_eq!(BoutPart::new(&cfg, false, &[SEATS]).err(), Some(seats));
        let guest = BoutPart::new(&cfg, false, &[1]).expect("seat part");
        assert_eq!(guest.outcomes().len(), 1);
        assert_eq!(
            guest.report(&guest.outcomes(), Vec::new()).err(),
            Some(BoutError::BadConfig("host")),
            "only the host folds a report"
        );
        let host = BoutPart::new(&cfg, true, &[0]).expect("host part");
        let mut both = host.outcomes();
        both.extend(guest.outcomes());
        both.reverse();
        assert_eq!(
            host.report(&both, Vec::new()).err(),
            Some(seats),
            "seats come in seat order"
        );
        assert_eq!(host.report(&host.outcomes(), Vec::new()).err(), Some(seats));
    }

    #[test]
    fn played_bout_is_a_pure_function_of_config_x2() {
        let cfg = short(2040, LinkProfile::WAN);
        let a = run_loopback(&cfg, LinkProfile::WAN, LinkProfile::WAN).expect("first");
        let b = run_loopback(&cfg, LinkProfile::WAN, LinkProfile::WAN).expect("second");
        assert_eq!(a, b);
        assert_eq!(a.digest(), b.digest());
        let other = short(2041, LinkProfile::WAN);
        let c = run_loopback(&other, LinkProfile::WAN, LinkProfile::WAN).expect("other seed");
        assert_ne!(a.digest(), c.digest(), "another seed plays another bout");
        assert_ne!(a.server_tip, c.server_tip);
    }

    #[test]
    fn played_ai_broadcasts_identical_with_machine_ids_swapped() {
        // Machine IDs are the seat tokens. Swapping them moves each machine to
        // the other seat, but the AI fill and the authoritative tape key on
        // seats and slots, so the tape bytes and checkpoints must not move.
        let cfg = short(2040, LinkProfile::LAN);
        let mut swapped = cfg;
        swapped.tokens = [TOKENS[1], TOKENS[0]];
        let a = run_loopback(&cfg, LinkProfile::LAN, LinkProfile::LAN).expect("bout");
        let b = run_loopback(&swapped, LinkProfile::LAN, LinkProfile::LAN).expect("swapped");
        assert!(a.converged(), "{a}");
        assert!(b.converged(), "{b}");
        assert_eq!(a.tape_len, b.tape_len);
        assert_eq!(a.tape_hash, b.tape_hash, "AI and human tape bytes");
        assert_eq!(a.checkpoints, b.checkpoints, "checkpoints");
        assert_eq!(a.server_tip, b.server_tip);
    }

    #[test]
    fn played_bout_without_a_rejoin_does_not_converge() {
        // The link stays dark to the end and the seat never asks back in: the
        // equality check has teeth, the dropped client diverges.
        let mut cfg = short(2040, LinkProfile::LAN);
        cfg.drop.back = cfg.ticks + 100;
        let report = run_loopback(&cfg, LinkProfile::LAN, LinkProfile::LAN).expect("bout runs");
        assert!(!report.converged());
        assert!(report.seats[0].equal, "the connected seat still converges");
        assert!(!report.seats[1].equal);
        assert!(report.seats[1].first_divergence.is_some());
        assert_eq!(report.host.reclaimed_at, None);
    }

    #[test]
    fn played_rejects_bad_config_and_foreign_rejoins() {
        let good = short(1, LinkProfile::LAN);
        let mut bad = good;
        bad.fighters = SEATS;
        assert_eq!(bad.validate(), Err(BoutError::BadConfig("fighters")));
        bad = good;
        bad.hold = 0;
        assert_eq!(bad.validate(), Err(BoutError::BadConfig("hold")));
        bad = good;
        bad.drop.back = bad.drop.at;
        assert_eq!(bad.validate(), Err(BoutError::BadConfig("drop")));
        bad = good;
        bad.drop.seat = SEATS;
        assert_eq!(bad.validate(), Err(BoutError::BadConfig("drop")));
        bad = good;
        bad.half_extent_m = 1.0;
        assert_eq!(bad.validate(), Err(BoutError::BadConfig("arena")));
        let mut wire = LoopbackWire::new(SEATS, LinkProfile::LAN, LinkProfile::LAN, 1);
        assert!(run_bout(&bad, &mut wire).is_err());
        // A rejoin with the wrong token or build is refused, not granted.
        let mut host = Host::new(good.arena(), good.hold, [0, 1], TOKENS).expect("host");
        let mut wire = LoopbackWire::new(SEATS, LinkProfile::LAN, LinkProfile::LAN, 2);
        let forged = Msg::JoinRequest {
            token: 0xDEAD,
            build: BOUT_BUILD,
        };
        host.on_control(1, &forged.encode().expect("encode"), &mut wire, 5);
        let stale = Msg::JoinRequest {
            token: TOKENS[1],
            build: BuildId {
                version: 9,
                rules: 1,
            },
        };
        host.on_control(1, &stale.encode().expect("encode"), &mut wire, 5);
        assert_eq!(host.ledger.refused, 2);
        assert_eq!(host.ledger.reclaimed_at, None);
    }

    #[test]
    fn played_coverage_extends_only_over_contiguous_windows() {
        let mut cover = Coverage::new();
        assert!(
            cover.extend(3, false),
            "the opening window starts at tick zero"
        );
        assert!(cover.extend(10, false), "overlapping windows chain");
        assert!(cover.extend(7, false), "a late older window still fits");
        assert!(
            !cover.extend(30, false),
            "a window that skips ticks is a gap"
        );
        assert_eq!(cover.through, 10);
        assert!(cover.extend(30, true), "streamed history fills the gap");
        assert!(cover.extend(33, false));
        assert!(!cover.extend(60, false));
    }

    #[test]
    fn played_seat_refuses_a_checkpoint_past_a_gap_and_asks_to_resync() {
        let cfg = short(5, LinkProfile::LAN);
        let slots = form_match(&cfg).expect("lobby");
        let mut seat =
            Seat::new(cfg.arena(), 1, slots[1], slots[0], TOKENS[1], None).expect("seat");
        let clean = LinkProfile {
            latency: 1,
            jitter: 0,
            loss_pct: 0,
            reorder_pct: 0,
        };
        let mut wire = LoopbackWire::new(SEATS, clean, clean, 1);
        let far = Checkpoint { tick: 40, hash: 1 };
        seat.on_frame(Frame::Datagram(encode_state(far, &[]).expect("encode")));
        assert!(seat.top.is_none() && seat.gap, "nothing applies past a gap");
        seat.heal_gap(10, &mut wire).expect("request");
        assert_eq!(seat.stats.gap_resyncs, 1);
        // Inside the wait a second gap raises no second request.
        seat.gap = true;
        seat.heal_gap(11, &mut wire).expect("quiet");
        assert_eq!(seat.stats.gap_resyncs, 1);
        seat.gap = true;
        seat.heal_gap(10 + RESYNC_WAIT_STEPS, &mut wire)
            .expect("again");
        assert_eq!(seat.stats.gap_resyncs, 2);
        wire.tick();
        wire.tick();
        let asked = wire.recv(Peer::Server);
        assert_eq!(asked.len(), 2);
        for (from, frame) in asked {
            assert_eq!(from, Peer::Client(1));
            let Frame::Stream(bytes) = frame else {
                panic!("a resync request rides the reliable lane");
            };
            assert!(matches!(
                Msg::decode(&bytes),
                Ok(Msg::JoinRequest { token, .. }) if token == TOKENS[1]
            ));
        }
        // Streamed history closes the gap, then the live window applies.
        seat.on_frame(Frame::Stream(encode_state(far, &[]).expect("encode")));
        assert_eq!(seat.top, Some(far));
    }
}
