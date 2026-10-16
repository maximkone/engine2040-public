//! dedicated: headless server with LAN discovery, slot reclaim and AI fill.
//!
//! [`HeadlessServer`] authors one arena match: connected clients submit inputs,
//! empty or dropped slots AI-fill through the arena brain fill, and a desync
//! resyncs from the latest [`SnapshotDelta`]. LAN peers find the match
//! through [`Beacon`] datagrams on a well-known UDP port. Consumer: the
//! packaged game and the dedicated-server operator. Units: fixed ticks
//! at the arena tick rate, latency in whole milliseconds. Determinism scope:
//! same config plus same submitted tape gives identical checkpoints (the
//! arena guarantee); latency reports and beacon transit never enter the hash.
//! Errors: [`ServerError`]; bad input rejects loudly and changes nothing.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use engine_arena::{ArenaConfig, Checkpoint};
use engine_input::{FighterInput, MAX_PARTICIPANTS};

use crate::heal::{DesyncPlan, NetHeal, NetHealError, QuarantineNotice, ReconnectPlan};
use crate::session::{SnapshotDelta, WIRE_VERSION};
use engine_observe::supervisor::AppliedHeal;

/// Beacon wire magic `b"MP"` little-endian, first two bytes of every beacon.
pub const BEACON_MAGIC: u16 = 0x504D;
/// Beacon wire version; anything else is rejected.
pub const BEACON_VERSION: u8 = 1;
/// Beacon datagram length: magic 2 + version 1 + port 2 + fighters 1 +
/// tick 4 + hash 8. Fixed size, no padding, no trailing bytes.
pub const BEACON_LEN: usize = 18;
/// Upper bound on a reported per-client latency in milliseconds.
pub const MAX_LATENCY_MS: u64 = 10_000;

/// Dedicated-server failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServerError {
    /// Arena config invalid (`ticks`, `fighters` or extent out of range).
    BadConfig,
    /// A fighter input, token or beacon field failed validation.
    BadInput,
    /// No free slot: every slot is connected.
    SlotFull,
    /// Unknown, disconnected or zero token, or an input stamped for a tick
    /// the server already authored (or has not reached). Culprit carried.
    StaleClient {
        /// Token the client presented.
        token: u64,
    },
    /// Client hash disagrees with authority at `tick`; resync from
    /// [`HeadlessServer::resync`]. Server hash (`want`) and client hash
    /// (`got`) carried.
    Desync {
        /// Tick the hashes disagree on.
        tick: u32,
        /// Authoritative hash.
        want: u64,
        /// Client-reported hash.
        got: u64,
    },
    /// No authoritative snapshot exists yet (the match has no ticks).
    NoSnapshot,
    /// The match already authored `config.ticks` ticks.
    MatchOver,
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadConfig => write!(f, "invalid arena config"),
            Self::BadInput => write!(f, "invalid input, token or beacon field"),
            Self::SlotFull => write!(f, "no free slot"),
            Self::StaleClient { token } => write!(f, "stale client token {token}"),
            Self::Desync { tick, want, got } => write!(
                f,
                "desync at tick {tick}: server {want:016x}, client {got:016x}"
            ),
            Self::NoSnapshot => write!(f, "no authoritative snapshot yet"),
            Self::MatchOver => write!(f, "match already ran its configured ticks"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Fixed ticks at 60 Hz converted to whole milliseconds (integer math only).
pub const fn ticks_to_ms(ticks: u32) -> u64 {
    ticks as u64 * 1_000 / engine_arena::TICK_HZ as u64
}

/// Whole milliseconds converted to fixed ticks at 60 Hz (integer math only).
pub const fn ms_to_ticks(ms: u64) -> u64 {
    ms * engine_arena::TICK_HZ as u64 / 1_000
}

/// One fighter slot: sticky client token, presence, last reported latency.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// Last client to hold the slot; `None` on a never-joined slot.
    token: Option<u64>,
    /// True while the token holder is connected; AI fills the slot otherwise.
    connected: bool,
    /// Last reported round-trip latency in milliseconds.
    latency_ms: u64,
}

/// LAN discovery beacon: where the dedicated match lives and how far it ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Beacon {
    /// UDP port the server receives inputs on.
    pub port: u16,
    /// Fighter slots in the match (`2..=MAX_PARTICIPANTS`).
    pub fighters: u8,
    /// Latest authored tick at beacon time.
    pub tick: u32,
    /// Authoritative hash at `tick` (`0` before the first tick).
    pub hash: u64,
}

impl Beacon {
    /// Range-check every field; port zero and out-of-range slots fail.
    pub fn validate(self) -> Result<(), ServerError> {
        if self.port == 0 || self.fighters < 2 || self.fighters > MAX_PARTICIPANTS {
            return Err(ServerError::BadInput);
        }
        Ok(())
    }

    /// Encode one beacon datagram: magic, version, port, fighters, tick,
    /// hash. No padding, no trailing bytes.
    pub fn encode(self) -> Result<[u8; BEACON_LEN], ServerError> {
        self.validate()?;
        let mut out = [0u8; BEACON_LEN];
        out[0..2].copy_from_slice(&BEACON_MAGIC.to_le_bytes());
        out[2] = BEACON_VERSION;
        out[3..5].copy_from_slice(&self.port.to_le_bytes());
        out[5] = self.fighters;
        out[6..10].copy_from_slice(&self.tick.to_le_bytes());
        out[10..18].copy_from_slice(&self.hash.to_le_bytes());
        Ok(out)
    }

    /// Decode and fully validate one beacon datagram. Wrong magic, any
    /// version but [`BEACON_VERSION`], bad length or bad fields reject
    /// loudly and change nothing.
    pub fn decode(bytes: &[u8]) -> Result<Self, ServerError> {
        if bytes.len() != BEACON_LEN {
            return Err(ServerError::BadInput);
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) != BEACON_MAGIC {
            return Err(ServerError::BadInput);
        }
        if bytes[2] != BEACON_VERSION {
            return Err(ServerError::BadInput);
        }
        let beacon = Self {
            port: u16::from_le_bytes([bytes[3], bytes[4]]),
            fighters: bytes[5],
            tick: u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            hash: u64::from_le_bytes([
                bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16],
                bytes[17],
            ]),
        };
        beacon.validate()?;
        Ok(beacon)
    }

    /// Send this beacon as one datagram to `to`.
    pub fn announce(&self, sock: &UdpSocket, to: SocketAddr) -> io::Result<usize> {
        let bytes = self.encode().map_err(io::Error::other)?;
        sock.send_to(&bytes, to)
    }

    /// Read one beacon datagram without blocking: `Ok(None)` when no
    /// datagram waits or the waiting datagram is junk (LAN discovery must
    /// tolerate neighbors); `Ok(Some)` on a valid beacon. The socket must
    /// be nonblocking; other IO errors propagate.
    pub fn listen_once(sock: &UdpSocket) -> io::Result<Option<(Self, SocketAddr)>> {
        let mut buf = [0u8; BEACON_LEN];
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if n != BEACON_LEN {
                    return Ok(None);
                }
                Ok(Self::decode(&buf).ok().map(|beacon| (beacon, from)))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Dedicated headless server: authoritative arena match plus slot presence.
///
/// Slots are sticky: [`Self::connect`] with a known token reclaims its slot
/// after a drop; unknown tokens take never-joined slots first, then slots
/// whose holders never came back, else [`ServerError::SlotFull`]. Every tick
/// without a submitted input AI-fills through the arena brain fill, so empty
/// and dropped slots need no extra code here. The server never rolls back:
/// a late or mis-stamped input loses as [`ServerError::StaleClient`] and the
/// client resimulates to server truth from [`Self::resync`].
pub struct HeadlessServer {
    config: ArenaConfig,
    inner: crate::session::Server,
    slots: Vec<Slot>,
    pending: Vec<FighterInput>,
    tape: Vec<FighterInput>,
    checkpoints: Vec<Checkpoint>,
    last_ai_filled: usize,
    heal: NetHeal,
}

impl HeadlessServer {
    /// Validate `config` and open a match with every slot empty (AI-filled
    /// until clients connect).
    pub fn new(config: ArenaConfig) -> Result<Self, ServerError> {
        if config.validate().is_err() {
            return Err(ServerError::BadConfig);
        }
        let inner = crate::session::Server::new(config).map_err(|_| ServerError::BadConfig)?;
        let slots = vec![
            Slot {
                token: None,
                connected: false,
                latency_ms: 0,
            };
            usize::from(config.fighters)
        ];
        Ok(Self {
            config,
            inner,
            slots,
            pending: Vec::new(),
            tape: Vec::new(),
            checkpoints: Vec::new(),
            last_ai_filled: 0,
            heal: NetHeal::new(),
        })
    }

    /// Match config.
    pub fn config(&self) -> ArenaConfig {
        self.config
    }

    /// Next tick to author.
    pub fn tick_now(&self) -> u32 {
        self.inner.tick_now()
    }

    /// Authoritative checkpoints so far, one per authored tick.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Slots with a connected client.
    pub fn connected_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.connected).count()
    }

    /// Slots the arena AI-filled on the last authored tick.
    pub fn ai_filled_last_tick(&self) -> usize {
        self.last_ai_filled
    }

    /// Slot held by `token`, connected or not.
    pub fn slot_of(&self, token: u64) -> Option<u8> {
        self.slots
            .iter()
            .position(|slot| slot.token == Some(token))
            .map(|index| index as u8)
    }

    /// True while `token` holds a connected slot.
    pub fn is_connected(&self, token: u64) -> bool {
        self.slot_of(token)
            .is_some_and(|slot| self.slots[usize::from(slot)].connected)
    }

    /// Connect `token`, reclaiming its sticky slot when it has one.
    /// Idempotent: connecting a connected token returns its slot.
    pub fn connect(&mut self, token: u64) -> Result<u8, ServerError> {
        if token == 0 {
            return Err(ServerError::BadInput);
        }
        if let Some(slot) = self.slot_of(token) {
            self.slots[usize::from(slot)].connected = true;
            return Ok(slot);
        }
        if let Some(index) = self.slots.iter().position(|slot| slot.token.is_none()) {
            self.slots[index] = Slot {
                token: Some(token),
                connected: true,
                latency_ms: 0,
            };
            return Ok(index as u8);
        }
        if let Some(index) = self.slots.iter().position(|slot| !slot.connected) {
            self.slots[index] = Slot {
                token: Some(token),
                connected: true,
                latency_ms: 0,
            };
            return Ok(index as u8);
        }
        Err(ServerError::SlotFull)
    }

    /// Mark `token` dropped; its slot AI-fills until it reclaims it.
    pub fn disconnect(&mut self, token: u64) -> bool {
        if let Some(slot) = self.slot_of(token) {
            let holder = &mut self.slots[usize::from(slot)];
            if holder.connected {
                holder.connected = false;
                self.pending.retain(|input| input.fighter.0 != slot);
                return true;
            }
        }
        false
    }

    /// Queue one input for the tick being authored. The input must validate,
    /// name the token's own slot and stamp [`Self::tick_now`]; duplicates for
    /// the slot reject. Anything from an unknown or dropped token, or stamped
    /// for any other tick, loses as stale.
    pub fn submit(&mut self, token: u64, input: FighterInput) -> Result<(), ServerError> {
        let Some(slot) = self.slot_of(token) else {
            return Err(ServerError::StaleClient { token });
        };
        if !self.slots[usize::from(slot)].connected {
            return Err(ServerError::StaleClient { token });
        }
        if input.validate().is_err() || input.fighter.0 != slot {
            return Err(ServerError::BadInput);
        }
        if input.tick != self.tick_now() {
            return Err(ServerError::StaleClient { token });
        }
        if self.pending.iter().any(|queued| queued.fighter.0 == slot) {
            return Err(ServerError::BadInput);
        }
        self.pending.push(input);
        Ok(())
    }

    /// Author one fixed tick with the submitted human inputs, returning the
    /// authoritative checkpoint plus the full [`SnapshotDelta`]. Slots
    /// without an input AI-fill. Rejects the whole tick past
    /// `config.ticks`.
    pub fn tick(&mut self) -> Result<(Checkpoint, SnapshotDelta), ServerError> {
        if self.tick_now() >= self.config.ticks {
            return Err(ServerError::MatchOver);
        }
        let mut submitted = std::mem::take(&mut self.pending);
        submitted.sort_by_key(|input| input.fighter.0);
        let (checkpoint, delta) = self
            .inner
            .tick(&submitted)
            .map_err(|_| ServerError::BadInput)?;
        self.tape.extend_from_slice(&submitted);
        self.checkpoints.push(checkpoint);
        self.last_ai_filled = self.slots.len().saturating_sub(submitted.len());
        Ok((checkpoint, delta))
    }

    /// Latest authoritative snapshot, or `None` before the first tick.
    pub fn snapshot(&self) -> Option<SnapshotDelta> {
        let checkpoint = self.checkpoints.last().copied()?;
        Some(SnapshotDelta {
            version: WIRE_VERSION,
            tick: checkpoint.tick,
            hash: checkpoint.hash,
            tape: self.tape.clone(),
        })
    }

    /// Resync path for a connected client: the latest authoritative
    /// snapshot. Apply it through the client's server-state handler and
    /// resimulate to converge.
    pub fn resync(&self, token: u64) -> Result<SnapshotDelta, ServerError> {
        if !self.is_connected(token) {
            return Err(ServerError::StaleClient { token });
        }
        self.snapshot().ok_or(ServerError::NoSnapshot)
    }

    /// Compare one client-reported hash against authority. A match answers
    /// `Ok`; a mismatch names the authoritative hash so the client resyncs.
    /// Unknown or dropped tokens and ticks the server never authored answer
    /// stale.
    pub fn verify(&self, token: u64, tick: u32, hash: u64) -> Result<(), ServerError> {
        if !self.is_connected(token) {
            return Err(ServerError::StaleClient { token });
        }
        let Some(checkpoint) = self.checkpoints.get(tick as usize) else {
            return Err(ServerError::StaleClient { token });
        };
        if checkpoint.tick != tick || checkpoint.hash != hash {
            return Err(ServerError::Desync {
                tick,
                want: checkpoint.hash,
                got: hash,
            });
        }
        Ok(())
    }

    /// Record one round-trip latency sample in milliseconds for a connected
    /// client; samples above [`MAX_LATENCY_MS`] reject.
    pub fn report_latency(&mut self, token: u64, ms: u64) -> Result<(), ServerError> {
        let Some(slot) = self.slot_of(token) else {
            return Err(ServerError::StaleClient { token });
        };
        if !self.slots[usize::from(slot)].connected {
            return Err(ServerError::StaleClient { token });
        }
        if ms > MAX_LATENCY_MS {
            return Err(ServerError::BadInput);
        }
        self.slots[usize::from(slot)].latency_ms = ms;
        Ok(())
    }

    /// Last reported latency in milliseconds, if the token holds a slot.
    pub fn latency_ms(&self, token: u64) -> Option<u64> {
        self.slot_of(token)
            .map(|slot| self.slots[usize::from(slot)].latency_ms)
    }

    /// Borrow the supervisor routing (budgets, tape, escalation).
    pub fn heal(&self) -> &NetHeal {
        &self.heal
    }

    /// Operator quarantine notices in report order.
    pub fn heal_notices(&self) -> &[QuarantineNotice] {
        self.heal.notices()
    }

    /// True while `slot` is quarantined (held by AI, reclaim refused).
    pub fn is_slot_quarantined(&self, slot: u8) -> bool {
        self.heal.is_quarantined_slot(slot)
    }

    /// Supervised drop: mark `token` dropped so its slot AI-fills, then
    /// route the disconnect through the supervisor for the reconnect plan.
    /// Unknown or already-dropped tokens go stale without touching the
    /// heal budget.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::StaleToken`] for unknown or dropped tokens,
    /// [`NetHealError::Quarantined`] for quarantined slots and
    /// [`NetHealError::HealFull`] when the supervisor queue or tape needs
    /// draining.
    pub fn disconnect_via_supervisor(&mut self, token: u64) -> Result<ReconnectPlan, NetHealError> {
        let Some(slot) = self.slot_of(token) else {
            return Err(NetHealError::StaleToken { token });
        };
        if !self.slots[usize::from(slot)].connected {
            return Err(NetHealError::StaleToken { token });
        }
        if let Some(notice) = self.heal.quarantine_of(slot) {
            return Err(NetHealError::Quarantined {
                slot,
                attempts: notice.attempts,
            });
        }
        let fault_tick = u64::from(self.tick_now());
        self.disconnect(token);
        self.heal.on_disconnect(slot, token, fault_tick)
    }

    /// Supervised reclaim: connect `token` to its sticky slot, or to a free
    /// unquarantined slot for fresh tokens. Quarantined slots refuse loudly
    /// until the operator verifies recovery and calls
    /// [`Self::heal_reset_slot`]. The raw [`Self::connect`] stays for
    /// unmanaged joins; operators use this path.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::StaleToken`] for the zero token,
    /// [`NetHealError::Quarantined`] for quarantined slots and
    /// [`NetHealError::NoSlot`] when every unquarantined slot is connected.
    pub fn reclaim_slot(&mut self, token: u64) -> Result<u8, NetHealError> {
        if token == 0 {
            return Err(NetHealError::StaleToken { token });
        }
        if let Some(slot) = self.slot_of(token) {
            if let Some(notice) = self.heal.quarantine_of(slot) {
                return Err(NetHealError::Quarantined {
                    slot,
                    attempts: notice.attempts,
                });
            }
            self.slots[usize::from(slot)].connected = true;
            return Ok(slot);
        }
        let free = self
            .slots
            .iter()
            .enumerate()
            .position(|(index, slot)| {
                slot.token.is_none() && !self.heal.is_quarantined_slot(index as u8)
            })
            .or_else(|| {
                self.slots.iter().enumerate().position(|(index, slot)| {
                    !slot.connected && !self.heal.is_quarantined_slot(index as u8)
                })
            });
        if let Some(index) = free {
            self.slots[index] = Slot {
                token: Some(token),
                connected: true,
                latency_ms: 0,
            };
            return Ok(index as u8);
        }
        Err(NetHealError::NoSlot)
    }

    /// Supervised desync check: compare the client-reported `hash` at `tick`
    /// against authority. Agreement answers `Ok(None)` and routes nothing;
    /// a mismatch routes through the supervisor and answers the deferred
    /// resync plan (fetch the snapshot at `applies_at` and resimulate).
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::StaleToken`] for unknown, dropped or
    /// never-authored ticks, [`NetHealError::Quarantined`] for quarantined
    /// slots and [`NetHealError::HealFull`] when the supervisor queue or
    /// tape needs draining.
    pub fn note_desync_via_supervisor(
        &mut self,
        token: u64,
        tick: u32,
        hash: u64,
    ) -> Result<Option<DesyncPlan>, NetHealError> {
        let Some(slot) = self.slot_of(token) else {
            return Err(NetHealError::StaleToken { token });
        };
        if let Some(notice) = self.heal.quarantine_of(slot) {
            return Err(NetHealError::Quarantined {
                slot,
                attempts: notice.attempts,
            });
        }
        if !self.slots[usize::from(slot)].connected {
            return Err(NetHealError::StaleToken { token });
        }
        match self.verify(token, tick, hash) {
            Ok(()) => Ok(None),
            Err(ServerError::Desync { .. }) => self
                .heal
                .on_desync(slot, token, tick, u64::from(tick))
                .map(Some),
            Err(_) => Err(NetHealError::StaleToken { token }),
        }
    }

    /// Supervised resync: the latest authoritative snapshot for a connected,
    /// unquarantined client. Apply it with
    /// [`Client::resync_from_authority`](crate::session::Client::resync_from_authority)
    /// at the plan boundary to converge.
    ///
    /// # Errors
    ///
    /// Returns [`NetHealError::StaleToken`] for unknown or dropped tokens,
    /// [`NetHealError::Quarantined`] for quarantined slots and
    /// [`NetHealError::NoSnapshot`] before the first tick.
    pub fn resync_via_supervisor(&self, token: u64) -> Result<SnapshotDelta, NetHealError> {
        let Some(slot) = self.slot_of(token) else {
            return Err(NetHealError::StaleToken { token });
        };
        if let Some(notice) = self.heal.quarantine_of(slot) {
            return Err(NetHealError::Quarantined {
                slot,
                attempts: notice.attempts,
            });
        }
        self.resync(token).map_err(|error| match error {
            ServerError::NoSnapshot => NetHealError::NoSnapshot,
            _ => NetHealError::StaleToken { token },
        })
    }

    /// Clear the shared heal budget and the slot quarantine after a verified
    /// recovery (reclaim plus snapshot convergence). History stays: tape and
    /// notices are evidence, not budget. Returns true when the slot was
    /// quarantined.
    pub fn heal_reset_slot(&mut self, slot: u8) -> bool {
        self.heal.reset_slot(slot)
    }

    /// Apply every deferred net heal due at or before `boundary_tick`,
    /// oldest first. Drain after AI-cover ticks so takeover and resync
    /// advice lands in the tape on time.
    pub fn apply_heal_boundary(&mut self, boundary_tick: u64) -> Vec<AppliedHeal> {
        self.heal.apply_tick_boundary(boundary_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        decode_input, decode_state, encode_input, encode_state, redundancy_window, Client,
        NetError, Server as SessionServer, INPUT_DELAY_TICKS, MAX_PACKET_BYTES, REDUNDANCY,
        WIRE_VERSION,
    };
    use engine_arena::run;
    use engine_input::fighter::Buttons;
    use engine_input::{FighterId, FighterTape};

    const MATCH_TICKS: u32 = 64;

    fn config(fighters: u8) -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: MATCH_TICKS,
            fighters,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }

    fn wish(tick: u32, slot: u8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(slot).expect("slot"),
            mx: if tick % 20 < 10 { 100 } else { -100 },
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    /// Drain every waiting datagram on `sock` in send order.
    fn drain(sock: &UdpSocket) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; MAX_PACKET_BYTES];
        while let Ok((n, _)) = sock.recv_from(&mut buf) {
            out.push(buf[..n].to_vec());
        }
        out
    }

    #[test]
    fn loopback_drop_rejoin_converges() {
        const DROP_AT: u32 = 20;
        const BACK_AT: u32 = 30;
        const END: u32 = 50;
        const TOKEN: u64 = 0xA11CE;
        let cfg = config(2);
        let server_sock = crate::session::bind_loopback().expect("server socket");
        let client_sock = crate::session::bind_loopback().expect("client socket");
        server_sock.set_nonblocking(true).expect("nonblocking");
        client_sock.set_nonblocking(true).expect("nonblocking");
        let server_addr = server_sock.local_addr().expect("server addr");
        let client_addr = client_sock.local_addr().expect("client addr");

        let mut server = HeadlessServer::new(cfg).expect("server");
        let mut client = Client::new(cfg, FighterId::new(0).expect("slot 0")).expect("client");
        // Slot 1 never connects: pure AI fill for the whole match.
        let home = server.connect(TOKEN).expect("connect");
        assert_eq!(home, 0);
        let mut predicted: Vec<FighterInput> = Vec::new();
        let mut seen: Vec<FighterInput> = Vec::new();
        let mut states_applied = 0u32;
        let mut peak_ai = 0usize;

        // Phase 1: live play over real loopback UDP datagrams.
        for step in 0..DROP_AT {
            let input = wish(step, 0);
            client.predict(input).expect("predict");
            predicted.push(input);
            let window = redundancy_window(&predicted, step, REDUNDANCY);
            let bytes = encode_input(step, &window).expect("encode input");
            crate::session::send_packet(&client_sock, server_addr, &bytes).expect("send");
            for bytes in drain(&server_sock) {
                if let Ok(packet) = decode_input(&bytes) {
                    for input in packet.inputs {
                        let _ = server.submit(TOKEN, input);
                    }
                }
            }
            let (checkpoint, delta) = server.tick().expect("server tick");
            let window = redundancy_window(&delta.tape, checkpoint.tick, 8);
            let bytes = encode_state(checkpoint, &window).expect("encode state");
            crate::session::send_packet(&server_sock, client_addr, &bytes).expect("send");
            for bytes in drain(&client_sock) {
                if let Ok(packet) = decode_state(&bytes) {
                    if client
                        .server_tick()
                        .is_some_and(|done| packet.checkpoint.tick <= done)
                    {
                        continue;
                    }
                    seen.extend_from_slice(&packet.inputs);
                    seen.sort_by_key(|input| (input.tick, input.fighter.0));
                    seen.dedup_by_key(|input| (input.tick, input.fighter.0));
                    let delta = SnapshotDelta {
                        version: WIRE_VERSION,
                        tick: packet.checkpoint.tick,
                        hash: packet.checkpoint.hash,
                        tape: seen.clone(),
                    };
                    if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                        states_applied += 1;
                        client.resimulate();
                    }
                }
            }
        }
        assert!(states_applied > 0, "client must track server truth");
        assert_eq!(client.checkpoints(), server.checkpoints());

        // Phase 2: the client drops; the server holds the slot with AI.
        assert!(server.disconnect(TOKEN));
        assert_eq!(server.connected_count(), 0);
        for _ in DROP_AT..BACK_AT {
            let (_, _) = server.tick().expect("ai tick");
            peak_ai = peak_ai.max(server.ai_filled_last_tick());
        }
        assert!(peak_ai >= 1, "dropped slots must AI-fill (peak {peak_ai})");

        // Phase 3: the same token reclaims the same slot and resyncs.
        let reclaimed = server.connect(TOKEN).expect("reclaim");
        assert_eq!(reclaimed, home, "reconnect must reclaim its slot");
        let snapshot = server.resync(TOKEN).expect("resync snapshot");
        let mispredicts = client
            .on_server_state(
                Checkpoint {
                    tick: snapshot.tick,
                    hash: snapshot.hash,
                },
                &snapshot,
            )
            .expect("resync applies");
        assert!(
            mispredicts.is_empty(),
            "shared prefix must agree after resync"
        );
        client.resimulate();
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "rejoined client must converge to server truth"
        );

        // Phase 4: live play resumes on the reclaimed slot to the final tick.
        predicted.retain(|input| input.tick < BACK_AT);
        seen = snapshot.tape.clone();
        for step in BACK_AT..END {
            let input = wish(step, 0);
            client.predict(input).expect("predict");
            predicted.push(input);
            let window = redundancy_window(&predicted, step, REDUNDANCY);
            let bytes = encode_input(step, &window).expect("encode input");
            crate::session::send_packet(&client_sock, server_addr, &bytes).expect("send");
            for bytes in drain(&server_sock) {
                if let Ok(packet) = decode_input(&bytes) {
                    for input in packet.inputs {
                        let _ = server.submit(TOKEN, input);
                    }
                }
            }
            let (checkpoint, delta) = server.tick().expect("server tick");
            let window = redundancy_window(&delta.tape, checkpoint.tick, 8);
            let bytes = encode_state(checkpoint, &window).expect("encode state");
            crate::session::send_packet(&server_sock, client_addr, &bytes).expect("send");
            for bytes in drain(&client_sock) {
                if let Ok(packet) = decode_state(&bytes) {
                    if client
                        .server_tick()
                        .is_some_and(|done| packet.checkpoint.tick <= done)
                    {
                        continue;
                    }
                    seen.extend_from_slice(&packet.inputs);
                    seen.sort_by_key(|input| (input.tick, input.fighter.0));
                    seen.dedup_by_key(|input| (input.tick, input.fighter.0));
                    let delta = SnapshotDelta {
                        version: WIRE_VERSION,
                        tick: packet.checkpoint.tick,
                        hash: packet.checkpoint.hash,
                        tape: seen.clone(),
                    };
                    if client.on_server_state(packet.checkpoint, &delta).is_ok() {
                        client.resimulate();
                    }
                }
            }
        }
        client.resimulate();
        assert_eq!(server.checkpoints().len(), END as usize);
        assert_eq!(
            client.checkpoints(),
            server.checkpoints(),
            "loopback drop plus rejoin must converge end to end"
        );
    }

    #[test]
    fn sixteen_slot_drop_rejoin_converges() {
        // The full canonical cap plays, drops a client, reclaims its
        // slot and converges end to end through the dedicated consumer.
        const DROP_AT: u32 = 10;
        const BACK_AT: u32 = 20;
        const END: u32 = 32;
        const TOKENS: [u64; 3] = [0xA11CE, 0xB22DE, 0xC33EF];
        let cfg = ArenaConfig {
            seed: 99,
            ticks: END + 8,
            fighters: MAX_PARTICIPANTS,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        let mut server = HeadlessServer::new(cfg).expect("16-slot server");
        for (index, token) in TOKENS.iter().enumerate() {
            assert_eq!(server.connect(*token), Ok(index as u8));
        }
        assert_eq!(server.connected_count(), TOKENS.len());
        let mut witness = Client::new(cfg, FighterId::new(2).expect("slot 2")).expect("client");

        // Phase 1: three humans plus thirteen AI slots author together.
        for step in 0..DROP_AT {
            for (slot, token) in TOKENS.iter().enumerate() {
                server
                    .submit(*token, wish(step, slot as u8))
                    .expect("submit");
            }
            server.tick().expect("server tick");
            assert_eq!(server.ai_filled_last_tick(), 16 - TOKENS.len());
            let snapshot = server.snapshot().expect("snapshot");
            let tip = server.checkpoints().last().copied().expect("tip");
            witness
                .on_server_state(tip, &snapshot)
                .expect("witness tracks truth");
            witness.resimulate();
        }
        assert_eq!(witness.checkpoints(), server.checkpoints());

        // Phase 2: slot 2 drops; its slot AI-fills while the token is away.
        assert!(server.disconnect(TOKENS[2]));
        for step in DROP_AT..BACK_AT {
            for (slot, token) in TOKENS[..2].iter().enumerate() {
                server
                    .submit(*token, wish(step, slot as u8))
                    .expect("submit");
            }
            server.tick().expect("ai tick");
            assert_eq!(server.ai_filled_last_tick(), 16 - 2);
        }

        // Phase 3: the same token reclaims the same slot and resyncs.
        let reclaimed = server.connect(TOKENS[2]).expect("reclaim");
        assert_eq!(reclaimed, 2, "reconnect must reclaim its slot");
        let snapshot = server.resync(TOKENS[2]).expect("resync snapshot");
        let tip = Checkpoint {
            tick: snapshot.tick,
            hash: snapshot.hash,
        };
        witness
            .on_server_state(tip, &snapshot)
            .expect("resync applies");
        witness.resimulate();
        assert_eq!(
            witness.checkpoints(),
            server.checkpoints(),
            "rejoined slot must converge to 16-slot server truth"
        );

        // Phase 4: all three humans play to the final tick and stay converged.
        for step in BACK_AT..END {
            for (slot, token) in TOKENS.iter().enumerate() {
                server
                    .submit(*token, wish(step, slot as u8))
                    .expect("submit");
            }
            server.tick().expect("server tick");
            let snapshot = server.snapshot().expect("snapshot");
            let tip = server.checkpoints().last().copied().expect("tip");
            witness
                .on_server_state(tip, &snapshot)
                .expect("witness tracks truth");
            witness.resimulate();
        }
        assert_eq!(server.checkpoints().len(), END as usize);
        assert_eq!(
            witness.checkpoints(),
            server.checkpoints(),
            "16-slot drop plus rejoin must converge end to end"
        );
    }

    #[test]
    fn slot_full_stale_desync_errors_with_resync_path() {
        let cfg = config(2);
        let mut server = HeadlessServer::new(cfg).expect("server");
        assert_eq!(server.connect(11), Ok(0));
        assert_eq!(server.connect(22), Ok(1));
        assert_eq!(server.connect(33), Err(ServerError::SlotFull));
        assert_eq!(server.connect(0), Err(ServerError::BadInput));
        // Unknown token submits lose as stale.
        let input = wish(0, 0);
        assert_eq!(
            server.submit(99, input),
            Err(ServerError::StaleClient { token: 99 })
        );
        // Wrong slot and out-of-range wishes reject without moving the tick.
        let mut wrong_slot = wish(0, 1);
        wrong_slot.fighter = FighterId::new(1).expect("slot 1");
        assert_eq!(server.submit(11, wrong_slot), Err(ServerError::BadInput));
        let mut wild = wish(0, 0);
        wild.mx = 101;
        assert_eq!(server.submit(11, wild), Err(ServerError::BadInput));
        assert_eq!(server.tick_now(), 0);
        // Right input authors tick 0; replaying it afterwards is stale.
        server.submit(11, input).expect("submit");
        let (first, _) = server.tick().expect("tick");
        assert_eq!(first.tick, 0);
        assert_eq!(
            server.submit(11, input),
            Err(ServerError::StaleClient { token: 11 })
        );
        // Dropped tokens go stale until they reclaim.
        assert!(server.disconnect(11));
        assert_eq!(
            server.submit(11, wish(1, 0)),
            Err(ServerError::StaleClient { token: 11 })
        );
        assert_eq!(server.connect(11), Ok(0));
        // Latency units are whole milliseconds with a validated bound.
        server.report_latency(11, 23).expect("latency");
        assert_eq!(server.latency_ms(11), Some(23));
        assert_eq!(
            server.report_latency(11, MAX_LATENCY_MS + 1),
            Err(ServerError::BadInput)
        );
        assert_eq!(server.latency_ms(11), Some(23));
        assert_eq!(
            server.report_latency(99, 5),
            Err(ServerError::StaleClient { token: 99 })
        );
        // Tick-unit helpers pin the 60 Hz mapping (3 ticks == 50 ms).
        assert_eq!(ticks_to_ms(3), 50);
        assert_eq!(ticks_to_ms(INPUT_DELAY_TICKS), 33);
        assert_eq!(ms_to_ticks(50), 3);
        // Desync names the authoritative hash; the resync path converges.
        for tick in 1..4 {
            server.submit(11, wish(tick, 0)).expect("submit");
            server.tick().expect("tick");
        }
        let truth = server.checkpoints()[2];
        assert_eq!(server.verify(11, 2, truth.hash), Ok(()));
        assert_eq!(
            server.verify(11, 2, truth.hash.wrapping_add(1)),
            Err(ServerError::Desync {
                tick: 2,
                want: truth.hash,
                got: truth.hash.wrapping_add(1),
            })
        );
        assert_eq!(
            server.verify(99, 2, truth.hash),
            Err(ServerError::StaleClient { token: 99 })
        );
        assert_eq!(
            server.verify(11, 99, truth.hash),
            Err(ServerError::StaleClient { token: 11 })
        );
        let snapshot = server.resync(11).expect("resync");
        let mut witness = Client::new(cfg, FighterId::new(0).expect("slot 0")).expect("client");
        let checkpoint = Checkpoint {
            tick: snapshot.tick,
            hash: snapshot.hash,
        };
        witness
            .on_server_state(checkpoint, &snapshot)
            .expect("resync applies");
        let converged = witness.resimulate();
        assert_eq!(converged, server.checkpoints());
        // A fresh server has no snapshot to resync from.
        let mut fresh = HeadlessServer::new(cfg).expect("server");
        fresh.connect(11).expect("connect");
        assert_eq!(fresh.resync(11), Err(ServerError::NoSnapshot));
        assert_eq!(
            fresh.resync(99),
            Err(ServerError::StaleClient { token: 99 })
        );
        // Past the configured tick count the match is over.
        let tiny = ArenaConfig {
            seed: 1,
            ticks: 1,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        let mut short = HeadlessServer::new(tiny).expect("server");
        short.connect(7).expect("connect");
        short.tick().expect("last tick");
        assert_eq!(short.tick(), Err(ServerError::MatchOver));
        // Bad configs never open a match.
        let bad = ArenaConfig { ticks: 0, ..cfg };
        assert_eq!(
            HeadlessServer::new(bad).map(|_| ()),
            Err(ServerError::BadConfig)
        );
        // Net-layer version errors stay loud through the witness path.
        let mut tampered = snapshot.clone();
        tampered.version = WIRE_VERSION + 1;
        assert_eq!(
            witness.on_server_state(checkpoint, &tampered),
            Err(NetError::VersionMismatch {
                got: WIRE_VERSION + 1,
            })
        );
    }

    #[test]
    fn beacon_loopback_roundtrip() {
        let a = crate::session::bind_loopback().expect("socket a");
        let b = crate::session::bind_loopback().expect("socket b");
        b.set_nonblocking(true).expect("nonblocking");
        let addr_b = b.local_addr().expect("addr b");
        // Junk on the LAN port is ignored, never a beacon.
        crate::session::send_packet(&a, addr_b, &[0u8; 3]).expect("send");
        assert!(Beacon::listen_once(&b).expect("listen").is_none());
        let beacon = Beacon {
            port: a.local_addr().expect("addr a").port(),
            fighters: 2,
            tick: 7,
            hash: 0xABCD,
        };
        let bytes = beacon.encode().expect("encode");
        assert_eq!(bytes.len(), BEACON_LEN);
        beacon.announce(&a, addr_b).expect("announce");
        // Loopback under load needs a short bounded poll; one nonblocking try
        // races the announce. Junk ahead of the valid datagram drains as none.
        let mut seen = None;
        for _ in 0..200 {
            if let Some(got) = Beacon::listen_once(&b).expect("listen") {
                seen = Some(got);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (found, _) = seen.expect("beacon arrives");
        assert_eq!(found, beacon);
        assert_eq!(Beacon::decode(&bytes), Ok(beacon));
        // Corrupt magic, future version, truncation and bad fields reject.
        let mut bad_magic = bytes;
        bad_magic[0] ^= 0xFF;
        assert_eq!(Beacon::decode(&bad_magic), Err(ServerError::BadInput));
        let mut future = bytes;
        future[2] = BEACON_VERSION + 1;
        assert_eq!(Beacon::decode(&future), Err(ServerError::BadInput));
        assert_eq!(
            Beacon::decode(&bytes[..BEACON_LEN - 1]),
            Err(ServerError::BadInput)
        );
        let mut bad_slots = bytes;
        bad_slots[5] = MAX_PARTICIPANTS + 1;
        assert_eq!(Beacon::decode(&bad_slots), Err(ServerError::BadInput));
        let mut bad_port = bytes;
        bad_port[3] = 0;
        bad_port[4] = 0;
        assert_eq!(Beacon::decode(&bad_port), Err(ServerError::BadInput));
        let zero_port = Beacon { port: 0, ..beacon };
        assert_eq!(zero_port.encode(), Err(ServerError::BadInput));
    }

    #[test]
    fn soak_8_bots_report() {
        const MATCHES: usize = 2;
        const TICKS: u32 = 240;
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: TICKS,
            fighters: 4,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        let mut hashes = Vec::with_capacity(MATCHES);
        for _ in 0..MATCHES {
            // No clients connect: all four slots AI-fill every tick.
            let mut server = HeadlessServer::new(cfg).expect("server");
            for _ in 0..TICKS {
                let (_, _) = server.tick().expect("tick");
            }
            assert_eq!(server.connected_count(), 0);
            assert_eq!(server.ai_filled_last_tick(), 4);
            assert_eq!(server.checkpoints().len(), TICKS as usize);
            hashes.push(server.checkpoints().last().copied().expect("checkpoint"));
        }
        // Same seed plus empty tape twice gives identical checkpoints.
        let mut reference = HeadlessServer::new(cfg).expect("server");
        for _ in 0..TICKS {
            let (_, _) = reference.tick().expect("tick");
        }
        assert_eq!(reference.checkpoints(), {
            let mut check = HeadlessServer::new(cfg).expect("server");
            for _ in 0..TICKS {
                let (_, _) = check.tick().expect("tick");
            }
            check.checkpoints().to_vec()
        });
        // The authoritative tape matches a direct arena run hash for hash.
        let direct = run(&cfg, &FighterTape::default());
        assert_eq!(direct.last(), reference.checkpoints().last());
        assert_ne!(hashes[0].hash, 0);
        println!(
            "SOAK bots=8 matches={MATCHES} ticks_per_match={TICKS} \
             ticks_total={} hash0={:016x} hash1={:016x} ai_per_tick=4",
            MATCHES as u32 * TICKS,
            hashes[0].hash,
            hashes[1].hash,
        );
        // The session authority agrees with the dedicated authority.
        let mut session = SessionServer::new(cfg).expect("session");
        for _ in 0..TICKS {
            let (_, _) = session.tick(&[]).expect("session tick");
        }
        assert_eq!(session.checkpoints(), reference.checkpoints());
    }
}
