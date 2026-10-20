//! relay: internet join over invite codes with hole punch plus relay fallback.
//!
//! Purpose: let two machines on different networks join one server match by a
//! short invite code, trying a direct path first and falling back to a
//! self-hosted forward-only relay that always works. The same module holds the
//! negative gate that keeps combat state authority-only.
//! Owner: net lane. Units: ticks at 60 Hz, bytes on the wire, addresses.
//! Determinism: invite mint, STUN/ICE shapes and the punch machine are seeded
//! integer-only; sockets and wall clocks never enter sim hashes.
//! Errors: RelayError; invalid wire or stale tickets refuse loudly.
//!
//! Runtime: the self-hosted binary drives RelayServer on workspace tokio off
//! the fixed tick; the sim consumes only validated inputs at tick bounds.

use engine_core::{hash::GOLDEN_GAMMA, mix64, splitmix64};
use std::collections::VecDeque;
use std::net::SocketAddr;

/// Wire magic `b"RL"`, first two bytes of every relay and signal datagram.
pub const RELAY_WIRE_MAGIC: u16 = 0x4C52;
/// Wire version accepted by every decoder here; others refuse loudly.
pub const RELAY_WIRE_VERSION: u8 = 1;
/// Hard datagram cap: 1200 B wins everywhere, the relay never fragments.
pub const MAX_RELAY_BYTES: usize = 1200;
/// Per-client forward queue bound; a full queue refuses, never drops.
pub const MAX_RELAY_QUEUE: usize = 256;
/// Peer bound per relay instance; beyond it registration refuses.
pub const MAX_RELAY_PEERS: usize = 64;
/// Invite code length in bytes (48 bits, 10 human chars).
pub const INVITE_BYTES: usize = 6;
/// Invite lifetime in ticks: 10 minutes at 60 Hz.
pub const INVITE_TTL_TICKS: u32 = 36_000;
/// Hole-punch budget in ticks: 5 s at 60 Hz, then relay fallback.
pub const PUNCH_TIMEOUT_TICKS: u32 = 300;
/// Relay protocol id every invite ticket carries: the netcode-style handshake
/// shape (protocol id, create/expire timestamps, opaque user bytes) with a
/// challenge round before the punch budget starts. Tickets minted elsewhere
/// (wrong protocol) refuse at the join decision, deny by default.
pub const RELAY_PROTOCOL_ID: u64 = 0xE204_0A11_CE00_0001;
/// Domain salt for the relay challenge round: keeps challenge answers off
/// the invite-code and relay-id streams.
const CHALLENGE_SALT: u64 = 0x51A7_C4A1_1E69_0A30;
/// STUN integrity, ICE candidates, pair nomination and consent freshness:
/// the conformance half of this module, re-exported unchanged.
pub use crate::relay_ice::*;

/// TURN allocation discipline: lifetimes, permissions and channel binds,
/// split to keep one owner file bounded. Re-exported unchanged.
#[path = "relay_turn.rs"]
mod relay_turn;
pub use relay_turn::*;

/// Relay and join failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelayError {
    /// Bytes fail magic, kind, version or field checks.
    BadInput,
    /// Wire magic does not match.
    BadMagic,
    /// Kind byte names no message.
    BadKind(u8),
    /// Datagram ends mid-field or has trailing bytes.
    Truncated,
    /// Payload exceeds [`MAX_RELAY_BYTES`].
    TooLarge,
    /// Unknown client on a forward or drain path.
    UnknownPeer(u64),
    /// Wrong relay token on registration.
    Unauthorized,
    /// Peers already at [`MAX_RELAY_PEERS`].
    SlotFull,
    /// Per-client queue already at `queue_cap`.
    QueueFull,
    /// Invite ticket past its lifetime.
    Expired,
    /// Build version or rules digest differed; pair carried.
    VersionMismatch {
        /// Ticket version.
        want_version: u16,
        /// Presented version.
        got_version: u16,
        /// Ticket rules digest.
        want_rules: u64,
        /// Presented rules digest.
        got_rules: u64,
    },
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadInput => write!(f, "invalid relay input"),
            Self::BadMagic => write!(f, "bad relay magic"),
            Self::BadKind(k) => write!(f, "unknown relay kind {k}"),
            Self::Truncated => write!(f, "truncated relay packet"),
            Self::TooLarge => write!(f, "relay payload exceeds size bound"),
            Self::UnknownPeer(id) => write!(f, "unknown relay peer {id}"),
            Self::Unauthorized => write!(f, "relay token rejected"),
            Self::SlotFull => write!(f, "relay peer slots full"),
            Self::QueueFull => write!(f, "relay forward queue full"),
            Self::Expired => write!(f, "invite ticket expired"),
            Self::VersionMismatch {
                want_version,
                got_version,
                want_rules,
                got_rules,
            } => write!(
                f,
                "build mismatch: want {want_version}/{want_rules:#x}, got {got_version}/{got_rules:#x}"
            ),
        }
    }
}

impl std::error::Error for RelayError {}

/// Short join secret: 6 bytes minted from a seeded stream, read aloud as 10
/// chars. Same seed gives the same code on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InviteCode(pub [u8; INVITE_BYTES]);

/// Human alphabet: Crockford-style, no I/L/O/U, so codes read aloud cleanly.
const INVITE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKMNPQRSTVWXYZ0123456789";

impl InviteCode {
    /// Mint one code from `state`; deterministic per seed. Steps the held
    /// SplitMix64 state through the shared `engine_core` owner.
    pub fn mint(state: &mut u64) -> Self {
        let step = |state: &mut u64| {
            let out = splitmix64(*state);
            *state = state.wrapping_add(GOLDEN_GAMMA);
            out
        };
        let mut bytes = [0u8; INVITE_BYTES];
        let mut word = step(state);
        for chunk in bytes.chunks_mut(4) {
            let part = word.to_le_bytes();
            chunk.copy_from_slice(&part[..chunk.len()]);
            word = step(state);
        }
        Self(bytes)
    }

    /// Raw bytes for the signal wire.
    pub fn as_bytes(&self) -> [u8; INVITE_BYTES] {
        self.0
    }

    /// Ten human chars, big-endian 5-bit groups, top two bits zero.
    pub fn encode_str(&self) -> String {
        let mut out = String::with_capacity(10);
        let mut acc: u32 = 0;
        let mut bits: u32 = 0;
        for byte in self.0 {
            acc = (acc << 8) | u32::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                let index = ((acc >> bits) & 31) as usize;
                out.push(INVITE_ALPHABET[index] as char);
            }
        }
        if bits > 0 {
            let index = ((acc << (5 - bits)) & 31) as usize;
            out.push(INVITE_ALPHABET[index] as char);
        }
        out
    }

    /// Parse ten human chars; wrong length, charset or padding refuses.
    pub fn decode_str(text: &str) -> Result<Self, RelayError> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 {
            return Err(RelayError::Truncated);
        }
        let mut acc: u32 = 0;
        let mut bits: u32 = 0;
        let mut out = [0u8; INVITE_BYTES];
        let mut pos = 0;
        for byte in bytes {
            let value = INVITE_ALPHABET
                .iter()
                .position(|cand| cand == byte)
                .ok_or(RelayError::BadInput)? as u32;
            acc = (acc << 5) | value;
            bits += 5;
            while bits >= 8 {
                bits -= 8;
                if pos >= INVITE_BYTES {
                    return Err(RelayError::BadInput);
                }
                out[pos] = ((acc >> bits) & 0xFF) as u8;
                pos += 1;
            }
        }
        // The 50-bit form carries 48 code bits plus two zero pad bits.
        if pos != INVITE_BYTES || (acc & 0x3) != 0 {
            return Err(RelayError::BadInput);
        }
        Ok(Self(out))
    }
}

/// One issued invite: code plus the build it joins plus its lifetime.
/// Expiry is tick counts, never wall time, so replays stay exact. The
/// protocol id plus opaque user bytes are the netcode-style token fields:
/// the host verifies them deny-by-default at the join decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InviteTicket {
    /// Secret the joiner must present.
    pub code: InviteCode,
    /// Executable version the joiner must match.
    pub version: u16,
    /// Rules digest the joiner must match.
    pub rules: u64,
    /// Issue tick.
    pub issued_at: u32,
    /// Lifetime in ticks.
    pub ttl_ticks: u32,
    /// Relay protocol this ticket joins; must equal [`RELAY_PROTOCOL_ID`].
    pub protocol: u64,
    /// Opaque user bytes bound to the code (shape only, never crypto here).
    pub user: u64,
}

impl InviteTicket {
    /// Issue a ticket for `now` with the standard lifetime.
    pub fn issue(code: InviteCode, version: u16, rules: u64, now: u32) -> Result<Self, RelayError> {
        if version == 0 {
            return Err(RelayError::BadInput);
        }
        let key = code
            .as_bytes()
            .iter()
            .fold(0u64, |acc, byte| (acc << 8) | u64::from(*byte));
        Ok(Self {
            code,
            version,
            rules,
            issued_at: now,
            ttl_ticks: INVITE_TTL_TICKS,
            protocol: RELAY_PROTOCOL_ID,
            user: mix64(key ^ RELAY_PROTOCOL_ID),
        })
    }

    /// True while `now` is inside `[issued_at, issued_at + ttl]`.
    pub fn is_live(&self, now: u32) -> bool {
        now.wrapping_sub(self.issued_at) <= self.ttl_ticks
    }

    /// Ticks of life left at `now`, zero when expired.
    pub fn remaining(&self, now: u32) -> u32 {
        if self.is_live(now) {
            self.ttl_ticks - now.wrapping_sub(self.issued_at)
        } else {
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Relay protocol: register, forward-only fallback, heartbeat, deny.
// ---------------------------------------------------------------------------

/// Relay message kind bytes.
const KIND_REGISTER: u8 = 0;
const KIND_REGISTERED: u8 = 1;
const KIND_FORWARD: u8 = 2;
const KIND_HEARTBEAT: u8 = 3;
const KIND_DENIED: u8 = 4;

/// Relay wire message: fixed header (magic 2, version 1, kind 1) plus body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RelayMsg {
    /// Client registers under the relay token.
    Register {
        /// Client id, nonzero.
        client_id: u64,
        /// Relay token, must match the server config.
        token: u64,
    },
    /// Server confirms registration.
    Registered {
        /// Confirmed client id.
        client_id: u64,
    },
    /// Forward-only game payload from one client to another.
    Forward {
        /// Sender id.
        from: u64,
        /// Destination id.
        to: u64,
        /// Game bytes, at most [`MAX_RELAY_BYTES`].
        payload: Vec<u8>,
    },
    /// Keepalive: NAT bindings stay warm without touching sim timing.
    Heartbeat {
        /// Sender id.
        client_id: u64,
    },
    /// Registration refused with a typed code.
    Denied {
        /// Refused client id.
        client_id: u64,
        /// Deny code: 0 token, 1 slots full, 2 bad input.
        code: u8,
    },
}

impl RelayMsg {
    fn kind(&self) -> u8 {
        match self {
            Self::Register { .. } => KIND_REGISTER,
            Self::Registered { .. } => KIND_REGISTERED,
            Self::Forward { .. } => KIND_FORWARD,
            Self::Heartbeat { .. } => KIND_HEARTBEAT,
            Self::Denied { .. } => KIND_DENIED,
        }
    }

    /// Encode one message; oversize payloads fail before any byte is built.
    /// One allocation: the body length is known up front, so the buffer is
    /// sized exactly and the field writes below never reallocate.
    pub fn encode(&self) -> Result<Vec<u8>, RelayError> {
        let body_len = match self {
            Self::Register { .. } => 16,
            Self::Registered { .. } | Self::Heartbeat { .. } => 8,
            Self::Forward { payload, .. } => 18 + payload.len(),
            Self::Denied { .. } => 9,
        };
        let mut out = Vec::with_capacity(4 + body_len);
        out.extend_from_slice(&RELAY_WIRE_MAGIC.to_le_bytes());
        out.push(RELAY_WIRE_VERSION);
        out.push(self.kind());
        match self {
            Self::Register { client_id, token } => {
                if *client_id == 0 {
                    return Err(RelayError::BadInput);
                }
                out.extend_from_slice(&client_id.to_le_bytes());
                out.extend_from_slice(&token.to_le_bytes());
            }
            Self::Registered { client_id } | Self::Heartbeat { client_id } => {
                if *client_id == 0 {
                    return Err(RelayError::BadInput);
                }
                out.extend_from_slice(&client_id.to_le_bytes());
            }
            Self::Forward { from, to, payload } => {
                if *from == 0 || *to == 0 || payload.is_empty() {
                    return Err(RelayError::BadInput);
                }
                if payload.len() > MAX_RELAY_BYTES {
                    return Err(RelayError::TooLarge);
                }
                out.extend_from_slice(&from.to_le_bytes());
                out.extend_from_slice(&to.to_le_bytes());
                let len = payload.len() as u16;
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(payload);
            }
            Self::Denied { client_id, code } => {
                if *code > 2 {
                    return Err(RelayError::BadInput);
                }
                out.extend_from_slice(&client_id.to_le_bytes());
                out.push(*code);
            }
        }
        if out.len() > MAX_RELAY_BYTES + 22 {
            return Err(RelayError::TooLarge);
        }
        Ok(out)
    }

    /// Decode and fully validate one message; junk refuses loudly.
    pub fn decode(bytes: &[u8]) -> Result<Self, RelayError> {
        if bytes.len() > MAX_RELAY_BYTES + 22 {
            return Err(RelayError::TooLarge);
        }
        if bytes.len() < 4 {
            return Err(RelayError::Truncated);
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) != RELAY_WIRE_MAGIC {
            return Err(RelayError::BadMagic);
        }
        if bytes[2] != RELAY_WIRE_VERSION {
            return Err(RelayError::VersionMismatch {
                want_version: u16::from(RELAY_WIRE_VERSION),
                got_version: u16::from(bytes[2]),
                want_rules: 0,
                got_rules: 0,
            });
        }
        let body = &bytes[4..];
        match bytes[3] {
            KIND_REGISTER => {
                if body.len() != 16 {
                    return Err(RelayError::Truncated);
                }
                let client_id =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                let token =
                    u64::from_le_bytes(body[8..16].try_into().map_err(|_| RelayError::Truncated)?);
                if client_id == 0 {
                    return Err(RelayError::BadInput);
                }
                Ok(Self::Register { client_id, token })
            }
            KIND_REGISTERED | KIND_HEARTBEAT => {
                if body.len() != 8 {
                    return Err(RelayError::Truncated);
                }
                let client_id =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                if client_id == 0 {
                    return Err(RelayError::BadInput);
                }
                if bytes[3] == KIND_REGISTERED {
                    Ok(Self::Registered { client_id })
                } else {
                    Ok(Self::Heartbeat { client_id })
                }
            }
            KIND_FORWARD => {
                if body.len() < 18 {
                    return Err(RelayError::Truncated);
                }
                let from =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                let to =
                    u64::from_le_bytes(body[8..16].try_into().map_err(|_| RelayError::Truncated)?);
                let len = usize::from(u16::from_le_bytes([body[16], body[17]]));
                if from == 0 || to == 0 || len == 0 {
                    return Err(RelayError::BadInput);
                }
                if len > MAX_RELAY_BYTES {
                    return Err(RelayError::TooLarge);
                }
                if body.len() != 18 + len {
                    return Err(RelayError::Truncated);
                }
                Ok(Self::Forward {
                    from,
                    to,
                    payload: body[18..].to_vec(),
                })
            }
            KIND_DENIED => {
                if body.len() != 9 {
                    return Err(RelayError::Truncated);
                }
                let client_id =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                if body[8] > 2 {
                    return Err(RelayError::BadInput);
                }
                Ok(Self::Denied {
                    client_id,
                    code: body[8],
                })
            }
            kind => Err(RelayError::BadKind(kind)),
        }
    }
}

/// Relay service config. Memory budget: at most `max_peers` queues of
/// `queue_cap` payloads of [`MAX_RELAY_BYTES`]; the defaults bound one
/// instance to about 20 MB worst case, steady state near zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RelayConfig {
    /// Shared token every client must present.
    pub token: u64,
    /// Peer bound, at most [`MAX_RELAY_PEERS`].
    pub max_peers: usize,
    /// Per-client queue bound, at most [`MAX_RELAY_QUEUE`].
    pub queue_cap: usize,
}

impl RelayConfig {
    /// Build a config; zero tokens and over-bound caps refuse.
    pub fn new(token: u64, max_peers: usize, queue_cap: usize) -> Result<Self, RelayError> {
        if token == 0 {
            return Err(RelayError::Unauthorized);
        }
        if max_peers == 0 || max_peers > MAX_RELAY_PEERS {
            return Err(RelayError::BadInput);
        }
        if queue_cap == 0 || queue_cap > MAX_RELAY_QUEUE {
            return Err(RelayError::BadInput);
        }
        Ok(Self {
            token,
            max_peers,
            queue_cap,
        })
    }
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            token: 0xE204_0A11_CE00_0001,
            max_peers: 32,
            queue_cap: 64,
        }
    }
}

/// Relay services counters: forwarded frames plus queue-full refusals.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RelayStats {
    /// Frames queued for delivery.
    pub forwarded: u64,
    /// Payload bytes queued for delivery.
    pub bytes_forwarded: u64,
    /// Forwards refused because the destination queue was full.
    pub dropped_full: u64,
}

/// Self-hostable forward-only relay: registered clients exchange game
/// payloads through per-client FIFO queues. Single-threaded and
/// tick-ordered, so the same call sequence always yields the same drains.
/// The binary wraps this on workspace tokio; it never runs on the sim tick.
pub struct RelayServer {
    config: RelayConfig,
    peers: Vec<u64>,
    queues: Vec<VecDeque<Vec<u8>>>,
    stats: RelayStats,
}

impl RelayServer {
    /// Open a relay on `config`.
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            peers: Vec::new(),
            queues: Vec::new(),
            stats: RelayStats::default(),
        }
    }

    /// Active config.
    pub fn config(&self) -> RelayConfig {
        self.config
    }

    /// Registered peer count.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Counters so far.
    pub fn stats(&self) -> RelayStats {
        self.stats
    }

    fn index(&self, client_id: u64) -> Option<usize> {
        self.peers.iter().position(|peer| *peer == client_id)
    }

    /// Register `client_id` under `token`. Wrong tokens refuse; full relays
    /// refuse; re-registration is idempotent.
    pub fn register(&mut self, client_id: u64, token: u64) -> Result<(), RelayError> {
        if client_id == 0 {
            return Err(RelayError::BadInput);
        }
        if token != self.config.token {
            return Err(RelayError::Unauthorized);
        }
        if self.index(client_id).is_some() {
            return Ok(());
        }
        if self.peers.len() >= self.config.max_peers {
            return Err(RelayError::SlotFull);
        }
        self.peers.push(client_id);
        self.queues
            .push(VecDeque::with_capacity(self.config.queue_cap));
        Ok(())
    }

    /// Queue `payload` from `from` for `to`. Unknown peers, oversize payloads
    /// and full queues refuse; nothing is ever silently dropped.
    pub fn forward(&mut self, from: u64, to: u64, payload: &[u8]) -> Result<(), RelayError> {
        // One pass names both slots; two index() scans would walk the peer
        // list twice on this per-payload hot path.
        let mut from_at = None;
        let mut to_at = None;
        for (index, peer) in self.peers.iter().enumerate() {
            if *peer == from {
                from_at = Some(index);
            }
            if *peer == to {
                to_at = Some(index);
            }
            if from_at.is_some() && to_at.is_some() {
                break;
            }
        }
        if from_at.is_none() {
            return Err(RelayError::UnknownPeer(from));
        }
        let at = to_at.ok_or(RelayError::UnknownPeer(to))?;
        if payload.is_empty() {
            return Err(RelayError::BadInput);
        }
        if payload.len() > MAX_RELAY_BYTES {
            return Err(RelayError::TooLarge);
        }
        if self.queues[at].len() >= self.config.queue_cap {
            self.stats.dropped_full += 1;
            return Err(RelayError::QueueFull);
        }
        self.queues[at].push_back(payload.to_vec());
        self.stats.forwarded += 1;
        self.stats.bytes_forwarded += payload.len() as u64;
        Ok(())
    }

    /// Drain every payload queued for `to`, oldest first. Unknown peers drain
    /// empty; known peers drain exactly what `forward` queued, in order.
    pub fn drain(&mut self, to: u64) -> Vec<Vec<u8>> {
        let Some(at) = self.index(to) else {
            return Vec::new();
        };
        self.queues[at].drain(..).collect()
    }
}

/// Counters of one [`RelayHub`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HubStats {
    /// STUN binding requests answered.
    pub stun: u64,
    /// Registrations accepted, re-registrations included.
    pub registered: u64,
    /// Registrations denied.
    pub denied: u64,
    /// Forwards and heartbeats refused: unknown or spoofed sender, unknown
    /// destination, full queue.
    pub refused: u64,
    /// Datagrams that were neither relay nor STUN bytes.
    pub junk: u64,
}

/// The self-hosted relay's socket-free core. It answers STUN binding
/// requests with the observed source address, registers clients under the
/// token (a re-register moves a client to its new address, so a NAT rebind
/// heals), and forwards only from the address a client registered from.
/// Each forward drains at once, so a queue never holds a stale payload.
pub struct RelayHub {
    server: RelayServer,
    addrs: std::collections::BTreeMap<u64, SocketAddr>,
    creds: Option<StunCreds>,
    turn: Option<TurnAllocator>,
    stats: HubStats,
}

impl RelayHub {
    /// Open a hub on `config`.
    pub fn new(config: RelayConfig) -> Self {
        Self {
            server: RelayServer::new(config),
            addrs: std::collections::BTreeMap::new(),
            creds: None,
            turn: None,
            stats: HubStats::default(),
        }
    }

    /// Require MESSAGE-INTEGRITY-SHA256 on every STUN binding, keyed by the
    /// invite ticket: unsigned or forged bindings get no answer.
    #[must_use]
    pub fn with_creds(mut self, creds: StunCreds) -> Self {
        self.creds = Some(creds);
        self
    }

    /// Enable TURN allocation discipline: forwards then need a live
    /// allocation plus a channel or permission for the destination.
    pub fn with_turn(mut self, base_port: u16) -> Result<Self, RelayError> {
        self.turn = Some(TurnAllocator::new(base_port)?);
        Ok(self)
    }

    /// Move the TURN clock; lifetimes are tick counts.
    pub fn advance(&mut self, now: u32) {
        if let Some(turn) = &mut self.turn {
            turn.advance(now);
        }
    }

    /// TURN counters, `None` when the discipline is off.
    pub fn turn_stats(&self) -> Option<TurnStats> {
        self.turn.as_ref().map(TurnAllocator::stats)
    }

    /// Allocate a relayed port for a registered client.
    pub fn allocate(
        &mut self,
        client_id: u64,
        requested_ttl: u32,
    ) -> Result<TurnAllocation, RelayError> {
        if !self.addrs.contains_key(&client_id) {
            return Err(RelayError::UnknownPeer(client_id));
        }
        self.turn
            .as_mut()
            .ok_or(RelayError::Unauthorized)?
            .allocate(client_id, requested_ttl)
    }

    /// Refresh (a zero ttl releases) a registered client's allocation.
    pub fn refresh(
        &mut self,
        client_id: u64,
        requested_ttl: u32,
    ) -> Result<Option<TurnAllocation>, RelayError> {
        if !self.addrs.contains_key(&client_id) {
            return Err(RelayError::UnknownPeer(client_id));
        }
        self.turn
            .as_mut()
            .ok_or(RelayError::Unauthorized)?
            .refresh(client_id, requested_ttl)
    }

    /// CreatePermission for a registered peer.
    pub fn create_permission(
        &mut self,
        client_id: u64,
        peer_client: u64,
    ) -> Result<(), RelayError> {
        let peer = self
            .addrs
            .get(&peer_client)
            .copied()
            .ok_or(RelayError::UnknownPeer(peer_client))?;
        self.turn
            .as_mut()
            .ok_or(RelayError::Unauthorized)?
            .create_permission(client_id, peer)
    }

    /// ChannelBind `channel` to a registered peer.
    pub fn bind_channel(
        &mut self,
        client_id: u64,
        channel: u16,
        peer_client: u64,
    ) -> Result<(), RelayError> {
        let peer = self
            .addrs
            .get(&peer_client)
            .copied()
            .ok_or(RelayError::UnknownPeer(peer_client))?;
        self.turn
            .as_mut()
            .ok_or(RelayError::Unauthorized)?
            .bind_channel(client_id, channel, peer)
    }

    /// The forward-only relay inside.
    pub fn server(&self) -> &RelayServer {
        &self.server
    }

    /// Counters so far.
    pub fn stats(&self) -> HubStats {
        self.stats
    }

    /// Handle one datagram from `src`; returns every datagram to send, with
    /// its destination, in order.
    pub fn handle(&mut self, src: SocketAddr, bytes: &[u8]) -> Vec<(SocketAddr, Vec<u8>)> {
        if self.creds.is_some() && is_stun(bytes) {
            return self.handle_signed_binding(src, bytes);
        }
        if self.creds.is_none() {
            if let Ok(tx) = decode_binding_request(bytes) {
                self.stats.stun += 1;
                return vec![(src, encode_binding_response(tx, src))];
            }
        }
        let Ok(msg) = RelayMsg::decode(bytes) else {
            self.stats.junk += 1;
            return Vec::new();
        };
        let reply = match msg {
            RelayMsg::Register { client_id, token } => {
                match self.server.register(client_id, token) {
                    Ok(()) => {
                        self.addrs.insert(client_id, src);
                        self.stats.registered += 1;
                        RelayMsg::Registered { client_id }
                    }
                    Err(err) => {
                        self.stats.denied += 1;
                        let code = match err {
                            RelayError::Unauthorized => 0,
                            RelayError::SlotFull => 1,
                            _ => 2,
                        };
                        RelayMsg::Denied { client_id, code }
                    }
                }
            }
            RelayMsg::Heartbeat { client_id } if self.addrs.get(&client_id) == Some(&src) => {
                RelayMsg::Heartbeat { client_id }
            }
            RelayMsg::Forward { from, to, payload } => {
                return self.forward(src, from, to, &payload)
            }
            RelayMsg::Heartbeat { .. } => {
                self.stats.refused += 1;
                return Vec::new();
            }
            RelayMsg::Registered { .. } | RelayMsg::Denied { .. } => {
                self.stats.junk += 1;
                return Vec::new();
            }
        };
        reply
            .encode()
            .map(|bytes| vec![(src, bytes)])
            .unwrap_or_default()
    }

    /// Answer a binding only when the ticket key verifies its tag. Every
    /// other STUN byte is refused and counted, never answered.
    fn handle_signed_binding(
        &mut self,
        src: SocketAddr,
        bytes: &[u8],
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        let Some(creds) = self.creds else {
            return Vec::new();
        };
        match decode_binding_request_signed(bytes, &creds) {
            Ok(check) => {
                self.stats.stun += 1;
                vec![(src, encode_binding_response_signed(check.tx, src, &creds))]
            }
            Err(_) => {
                self.stats.refused += 1;
                Vec::new()
            }
        }
    }

    fn forward(
        &mut self,
        src: SocketAddr,
        from: u64,
        to: u64,
        payload: &[u8],
    ) -> Vec<(SocketAddr, Vec<u8>)> {
        let dest = self.addrs.get(&to).copied();
        let (Some(dest), true) = (dest, self.addrs.get(&from) == Some(&src)) else {
            self.stats.refused += 1;
            return Vec::new();
        };
        if let Some(turn) = &mut self.turn {
            if turn.permit(from, dest).is_err() {
                self.stats.refused += 1;
                return Vec::new();
            }
        }
        if self.server.forward(from, to, payload).is_err() {
            self.stats.refused += 1;
            return Vec::new();
        }
        self.server
            .drain(to)
            .into_iter()
            .filter_map(|payload| RelayMsg::Forward { from, to, payload }.encode().ok())
            .map(|bytes| (dest, bytes))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Hole punch: direct-first state machine with timed relay fallback.
// ---------------------------------------------------------------------------

/// Observed NAT shape drives the punch outcome.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NatKind {
    /// Public address: checks succeed immediately.
    Open,
    /// Cone NAT: one mapping, checks succeed after STUN.
    Cone,
    /// Symmetric NAT: every peer needs its own mapping, direct fails.
    Symmetric,
}

/// Punch state: gather, check, then exactly one of direct or relay.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PunchState {
    /// Gathering candidates.
    Gathering,
    /// Trying direct checks.
    Checking,
    /// Direct path won; game traffic skips the relay.
    Direct,
    /// Fallback won; game traffic rides the relay.
    ViaRelay,
}

/// Direct-first hole punch with a tick-counted fallback deadline.
/// Deterministic: same NAT plus same check sequence gives the same state.
pub struct HolePunch {
    nat: NatKind,
    state: PunchState,
    waited: u32,
}

impl HolePunch {
    /// Start gathering behind `nat`.
    pub fn new(nat: NatKind) -> Self {
        Self {
            nat,
            state: PunchState::Gathering,
            waited: 0,
        }
    }

    /// Current state.
    pub fn state(&self) -> PunchState {
        self.state
    }

    /// Ticks spent checking so far.
    pub fn attempts(&self) -> u32 {
        self.waited
    }

    /// True once the direct path won.
    pub fn is_direct(&self) -> bool {
        self.state == PunchState::Direct
    }

    /// True once fallback won; game traffic must ride the relay.
    pub fn needs_relay(&self) -> bool {
        self.state == PunchState::ViaRelay
    }

    /// Candidates gathered; start direct checks. Only Gathering moves.
    pub fn on_gathered(&mut self) {
        if self.state == PunchState::Gathering {
            self.state = PunchState::Checking;
        }
    }

    /// One tick of checking: `check_ok` names a successful direct check.
    /// Direct wins on the first success except behind symmetric NAT; the
    /// deadline moves to relay fallback so joins always complete.
    pub fn tick(&mut self, check_ok: bool) {
        match self.state {
            PunchState::Gathering | PunchState::Direct | PunchState::ViaRelay => {}
            PunchState::Checking => {
                if check_ok && self.nat != NatKind::Symmetric {
                    self.state = PunchState::Direct;
                    return;
                }
                self.waited += 1;
                if self.waited >= PUNCH_TIMEOUT_TICKS {
                    self.state = PunchState::ViaRelay;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Join signaling: invite-code request plus typed accept/refuse.
// ---------------------------------------------------------------------------

const SIG_JOIN_REQUEST: u8 = 0;
const SIG_JOIN_ACCEPT: u8 = 1;
const SIG_JOIN_REFUSE: u8 = 2;

/// Refusal code: typed, never a string.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefuseCode {
    /// Ticket expired.
    Expired,
    /// Build version or rules digest differed.
    VersionMismatch,
    /// Zero token or zero version on the request.
    BadInput,
}

impl RefuseCode {
    /// Wire code for this reason.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Expired => 0,
            Self::VersionMismatch => 1,
            Self::BadInput => 2,
        }
    }

    /// Decode a wire code; unknown codes fail.
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Expired),
            1 => Some(Self::VersionMismatch),
            2 => Some(Self::BadInput),
            _ => None,
        }
    }
}

/// Join signal: the invite handshake that rides reliable streams.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Signal {
    /// Client presents an invite ticket plus build identity.
    Request {
        /// Invite secret.
        code: [u8; INVITE_BYTES],
        /// Client executable version.
        version: u16,
        /// Client rules digest.
        rules: u64,
        /// Client token, nonzero.
        token: u64,
    },
    /// Host grants a slot.
    Accept {
        /// Echoed client token.
        token: u64,
        /// Slot granted.
        slot: u8,
    },
    /// Host refuses with the digest pair on version mismatch.
    Refuse {
        /// Echoed client token.
        token: u64,
        /// Typed reason.
        reason: RefuseCode,
        /// Host build version.
        want_version: u16,
        /// Client build version as seen by the host.
        got_version: u16,
        /// Host rules digest.
        want_rules: u64,
        /// Client rules digest as seen by the host.
        got_rules: u64,
    },
}

impl Signal {
    /// Client token carried by every variant.
    pub fn token(&self) -> u64 {
        match *self {
            Self::Request { token, .. }
            | Self::Accept { token, .. }
            | Self::Refuse { token, .. } => token,
        }
    }

    /// Encode one signal; invalid fields fail before any byte is built.
    /// One allocation: the body length is fixed per variant, so the buffer
    /// is sized exactly and the field writes below never reallocate.
    pub fn encode(&self) -> Result<Vec<u8>, RelayError> {
        let body_len = match self {
            Self::Request { .. } => INVITE_BYTES + 2 + 8 + 8,
            Self::Accept { .. } => 9,
            Self::Refuse { .. } => 29,
        };
        let mut out = Vec::with_capacity(4 + body_len);
        out.extend_from_slice(&RELAY_WIRE_MAGIC.to_le_bytes());
        out.push(RELAY_WIRE_VERSION);
        match self {
            Self::Request {
                code,
                version,
                rules,
                token,
            } => {
                if *token == 0 || *version == 0 {
                    return Err(RelayError::BadInput);
                }
                out.push(SIG_JOIN_REQUEST);
                out.extend_from_slice(code);
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(&rules.to_le_bytes());
                out.extend_from_slice(&token.to_le_bytes());
            }
            Self::Accept { token, slot } => {
                if *token == 0 {
                    return Err(RelayError::BadInput);
                }
                out.push(SIG_JOIN_ACCEPT);
                out.extend_from_slice(&token.to_le_bytes());
                out.push(*slot);
            }
            Self::Refuse {
                token,
                reason,
                want_version,
                got_version,
                want_rules,
                got_rules,
            } => {
                out.push(SIG_JOIN_REFUSE);
                out.extend_from_slice(&token.to_le_bytes());
                out.push(reason.as_u8());
                out.extend_from_slice(&want_version.to_le_bytes());
                out.extend_from_slice(&got_version.to_le_bytes());
                out.extend_from_slice(&want_rules.to_le_bytes());
                out.extend_from_slice(&got_rules.to_le_bytes());
            }
        }
        Ok(out)
    }

    /// Decode and fully validate one signal; junk refuses loudly.
    pub fn decode(bytes: &[u8]) -> Result<Self, RelayError> {
        if bytes.len() < 4 {
            return Err(RelayError::Truncated);
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) != RELAY_WIRE_MAGIC {
            return Err(RelayError::BadMagic);
        }
        if bytes[2] != RELAY_WIRE_VERSION {
            return Err(RelayError::VersionMismatch {
                want_version: u16::from(RELAY_WIRE_VERSION),
                got_version: u16::from(bytes[2]),
                want_rules: 0,
                got_rules: 0,
            });
        }
        let body = &bytes[4..];
        match bytes[3] {
            SIG_JOIN_REQUEST => {
                if body.len() != INVITE_BYTES + 2 + 8 + 8 {
                    return Err(RelayError::Truncated);
                }
                let mut code = [0u8; INVITE_BYTES];
                code.copy_from_slice(&body[..INVITE_BYTES]);
                let version = u16::from_le_bytes([body[6], body[7]]);
                let rules =
                    u64::from_le_bytes(body[8..16].try_into().map_err(|_| RelayError::Truncated)?);
                let token =
                    u64::from_le_bytes(body[16..24].try_into().map_err(|_| RelayError::Truncated)?);
                if token == 0 || version == 0 {
                    return Err(RelayError::BadInput);
                }
                Ok(Self::Request {
                    code,
                    version,
                    rules,
                    token,
                })
            }
            SIG_JOIN_ACCEPT => {
                if body.len() != 9 {
                    return Err(RelayError::Truncated);
                }
                let token =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                if token == 0 {
                    return Err(RelayError::BadInput);
                }
                Ok(Self::Accept {
                    token,
                    slot: body[8],
                })
            }
            SIG_JOIN_REFUSE => {
                if body.len() != 29 {
                    return Err(RelayError::Truncated);
                }
                let token =
                    u64::from_le_bytes(body[0..8].try_into().map_err(|_| RelayError::Truncated)?);
                let reason = RefuseCode::from_u8(body[8]).ok_or(RelayError::BadKind(body[8]))?;
                let want_version = u16::from_le_bytes([body[9], body[10]]);
                let got_version = u16::from_le_bytes([body[11], body[12]]);
                let want_rules =
                    u64::from_le_bytes(body[13..21].try_into().map_err(|_| RelayError::Truncated)?);
                let got_rules =
                    u64::from_le_bytes(body[21..29].try_into().map_err(|_| RelayError::Truncated)?);
                Ok(Self::Refuse {
                    token,
                    reason,
                    want_version,
                    got_version,
                    want_rules,
                    got_rules,
                })
            }
            kind => Err(RelayError::BadKind(kind)),
        }
    }
}

/// Host-side join decision: check the protocol, the presented code, lifetime
/// and build, then accept into `slot` or refuse with the exact reason plus
/// digest pair.
pub fn decide_join(
    ticket: &InviteTicket,
    code: [u8; INVITE_BYTES],
    version: u16,
    rules: u64,
    token: u64,
    slot: u8,
    now: u32,
) -> Signal {
    if ticket.protocol != RELAY_PROTOCOL_ID {
        return Signal::Refuse {
            token,
            reason: RefuseCode::VersionMismatch,
            want_version: ticket.version,
            got_version: version,
            want_rules: ticket.rules,
            got_rules: rules,
        };
    }
    if token == 0 || version == 0 {
        return Signal::Refuse {
            token,
            reason: RefuseCode::BadInput,
            want_version: ticket.version,
            got_version: version,
            want_rules: ticket.rules,
            got_rules: rules,
        };
    }
    if code != ticket.code.as_bytes() || !ticket.is_live(now) {
        return Signal::Refuse {
            token,
            reason: RefuseCode::Expired,
            want_version: ticket.version,
            got_version: version,
            want_rules: ticket.rules,
            got_rules: rules,
        };
    }
    if version != ticket.version || rules != ticket.rules {
        return Signal::Refuse {
            token,
            reason: RefuseCode::VersionMismatch,
            want_version: ticket.version,
            got_version: version,
            want_rules: ticket.rules,
            got_rules: rules,
        };
    }
    Signal::Accept { token, slot }
}

/// Mint the relay challenge for `client_id` from `server_seed`: integer-only
/// and seeded, no clock and no hash input. The punch budget starts only after
/// the answer verifies, so the challenge round orders before the first probe.
pub(crate) fn mint_challenge(server_seed: u64, client_id: u64) -> u64 {
    mix64(
        server_seed
            .wrapping_add(client_id)
            .wrapping_add(CHALLENGE_SALT),
    )
}

/// Answer a challenge with the relay token. The token stands in for the
/// encrypted user bytes: shape only, never crypto here.
pub(crate) fn answer_challenge(challenge: u64, token: u64) -> u64 {
    mix64(challenge ^ token)
}

/// Verify an answer deny-by-default: wrong answers and zero tokens refuse.
/// Pure integer check, so the same pair always gives the same verdict.
pub(crate) fn verify_answer(challenge: u64, token: u64, answer: u64) -> bool {
    token != 0 && answer == answer_challenge(challenge, token)
}

// ---------------------------------------------------------------------------
// P2P combat-sync gate: combat state stays authority-only.
// ---------------------------------------------------------------------------
// A peer-to-peer CRDT sync for VR (arXiv 2503.17826v1) merges state with no
// server and assumes non-Byzantine peers. Combat state cannot: the admission
// screen (`crate::guard`) exists because a peer may lie, and a merged state
// has no arbiter that can refuse a forged blow. This gate is the negative half
// of that evaluation; it has no production caller by design, because nothing
// in the tree may propose a peer-authored combat change.

/// Who authored one proposed combat-state change. Only the match's dedicated
/// server may author combat state; a peer-authored row is the P2P path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncAuthor {
    /// The match's dedicated server, named by its id.
    Authority(u64),
    /// A player peer, named by its id.
    Peer(u64),
}

/// One proposed combat-state change as a wire would carry it: the author, the
/// authoritative tape row it was recorded on (none when the row names none),
/// and the state digest it would set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CombatSync {
    /// Author of the change.
    pub author: SyncAuthor,
    /// Authoritative tape row the change was recorded on.
    pub tape: Option<u32>,
    /// State digest the change would set.
    pub state: u64,
}

/// Why the gate refused one proposed combat-state change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum P2pRefusal {
    /// A peer authored it: combat state is authority-only, so the P2P path has
    /// no arbiter that can refuse a forged blow.
    PeerAuthored,
    /// No tape row is named, so no authoritative action recorded the change.
    NoTape,
    /// The change names an authority this match does not serve; a second
    /// writer for one match is a merge.
    ForeignAuthority,
    /// The tape row is already held with different state; two writers for one
    /// tick is a merge, and combat state never merges.
    HeldRow,
}

/// The negative gate for combat state, one per match: a change applies only
/// when the single authority authors it and it names the tape row that
/// recorded it. Held rows prove no second writer merged a tick, and
/// [`Self::peer_rows_refused`] counts the P2P combat path turned away.
/// Integer-only and deterministic: the same changes give the same verdicts.
pub struct CombatSyncGate {
    authority: u64,
    state: u64,
    held: std::collections::BTreeMap<u32, u64>,
    peer_rows_refused: u32,
}

impl CombatSyncGate {
    /// Open a gate for `authority`.
    ///
    /// # Errors
    ///
    /// [`RelayError::Unauthorized`] when the authority id is zero: a match
    /// without an authority can hold no combat state.
    pub fn new(authority: u64) -> Result<Self, RelayError> {
        if authority == 0 {
            return Err(RelayError::Unauthorized);
        }
        Ok(Self {
            authority,
            state: 0,
            held: std::collections::BTreeMap::new(),
            peer_rows_refused: 0,
        })
    }

    /// The authority this gate serves.
    #[must_use]
    pub fn authority(&self) -> u64 {
        self.authority
    }

    /// Combat state digest the gate holds.
    #[must_use]
    pub fn state(&self) -> u64 {
        self.state
    }

    /// Tape rows held, ascending.
    #[must_use]
    pub fn held_rows(&self) -> Vec<u32> {
        self.held.keys().copied().collect()
    }

    /// Peer-authored changes refused so far.
    #[must_use]
    pub fn peer_rows_refused(&self) -> u32 {
        self.peer_rows_refused
    }

    /// Apply one proposed change: applied only for the match's own authority
    /// with a named tape row. A peer author, a missing tape row, a foreign
    /// authority and a rewrite of a held row all refuse.
    ///
    /// # Errors
    ///
    /// [`P2pRefusal`] naming why nothing applied; the held state and rows are
    /// untouched on every refusal.
    pub fn apply(&mut self, change: CombatSync) -> Result<(), P2pRefusal> {
        let author = match change.author {
            SyncAuthor::Authority(authority) => authority,
            SyncAuthor::Peer(_) => {
                self.peer_rows_refused = self.peer_rows_refused.saturating_add(1);
                return Err(P2pRefusal::PeerAuthored);
            }
        };
        let Some(tape) = change.tape else {
            return Err(P2pRefusal::NoTape);
        };
        if author != self.authority {
            return Err(P2pRefusal::ForeignAuthority);
        }
        if self
            .held
            .get(&tape)
            .is_some_and(|held| *held != change.state)
        {
            return Err(P2pRefusal::HeldRow);
        }
        self.held.insert(tape, change.state);
        self.state = change.state;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn socket(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    #[test]
    fn relay_hub_echoes_stun_registers_and_forwards_from_the_registered_address_only() {
        let config = RelayConfig::default();
        let mut hub = RelayHub::new(config);
        let (a, b, thief) = (socket(5001), socket(5002), socket(5003));
        let tx = [7u8; 12];
        let reply = hub.handle(a, &encode_binding_request(tx));
        assert_eq!(reply.len(), 1);
        assert_eq!(
            decode_binding_response(&reply[0].1, tx),
            Ok(a),
            "STUN echoes the source"
        );
        let register = |id, token| {
            RelayMsg::Register {
                client_id: id,
                token,
            }
            .encode()
            .expect("encode")
        };
        let denied = hub.handle(a, &register(1, 9));
        assert_eq!(
            RelayMsg::decode(&denied[0].1),
            Ok(RelayMsg::Denied {
                client_id: 1,
                code: 0
            })
        );
        for (id, addr) in [(1, a), (2, b)] {
            let ok = hub.handle(addr, &register(id, config.token));
            assert_eq!(
                ok,
                vec![(
                    addr,
                    RelayMsg::Registered { client_id: id }
                        .encode()
                        .expect("ack")
                )]
            );
        }
        let forward = |from, to| {
            RelayMsg::Forward {
                from,
                to,
                payload: b"blow".to_vec(),
            }
            .encode()
            .expect("forward")
        };
        let out = hub.handle(a, &forward(1, 2));
        assert_eq!(
            out,
            vec![(b, forward(1, 2))],
            "delivered to the registered address"
        );
        assert!(
            hub.handle(thief, &forward(1, 2)).is_empty(),
            "a spoofed sender is refused"
        );
        assert!(
            hub.handle(a, &forward(1, 3)).is_empty(),
            "an unknown peer is refused"
        );
        // A re-register from a new address (a NAT rebind) moves the client.
        let moved = socket(5004);
        hub.handle(moved, &register(2, config.token));
        assert_eq!(hub.handle(a, &forward(1, 2)), vec![(moved, forward(1, 2))]);
        assert!(hub.handle(a, b"junk").is_empty());
        let stats = hub.stats();
        assert_eq!((stats.stun, stats.registered, stats.denied), (1, 3, 1));
        assert_eq!((stats.refused, stats.junk), (2, 1));
        assert_eq!(hub.server().stats().forwarded, 2);
    }

    #[test]
    fn invite_mint_repeats_per_seed() {
        let mut first = 0x0A11_CE00_1234_5678;
        let mut second = 0x0A11_CE00_1234_5678;
        let (a, b) = (InviteCode::mint(&mut first), InviteCode::mint(&mut second));
        assert_eq!(a, b, "same seed mints the same code");
        let mut third = 0x0A11_CE00_1234_5679;
        assert_ne!(
            InviteCode::mint(&mut third),
            a,
            "neighbour seeds never alias"
        );
    }

    #[test]
    fn invite_string_roundtrips() {
        let mut state = 77u64;
        for _ in 0..16 {
            let code = InviteCode::mint(&mut state);
            let text = code.encode_str();
            assert_eq!(text.len(), 10, "six bytes read as ten chars");
            assert_eq!(
                InviteCode::decode_str(&text),
                Ok(code),
                "string parses back"
            );
            assert_eq!(code.as_bytes().len(), INVITE_BYTES);
        }
    }

    #[test]
    fn invite_string_rejects_junk() {
        assert_eq!(
            InviteCode::decode_str("SHORT"),
            Err(RelayError::Truncated),
            "short codes refuse"
        );
        assert_eq!(
            InviteCode::decode_str("AAAAAAAAAAI"),
            Err(RelayError::Truncated),
            "long codes refuse"
        );
        assert!(
            InviteCode::decode_str("AAAAAAAAA!").is_err(),
            "charset junk refuses"
        );
        assert!(
            InviteCode::decode_str("aaaaaaaaaa").is_err(),
            "lowercase refuses: codes are strict uppercase"
        );
        // Ten valid chars with nonzero pad bits are a different code space.
        assert_eq!(
            InviteCode::decode_str("AAAAAAAAAB"),
            Err(RelayError::BadInput),
            "nonzero pad bits refuse"
        );
    }

    #[test]
    fn ticket_lifetime_holds_then_expires() {
        let mut state = 9u64;
        let ticket = InviteTicket::issue(InviteCode::mint(&mut state), 3, 0xDEAD_BEEF, 100)
            .expect("nonzero version issues");
        assert!(ticket.is_live(100), "issued tickets are live");
        assert!(
            ticket.is_live(100 + INVITE_TTL_TICKS),
            "deadline tick is live"
        );
        assert!(
            !ticket.is_live(100 + INVITE_TTL_TICKS + 1),
            "past ttl expires"
        );
        assert_eq!(
            ticket.remaining(100 + INVITE_TTL_TICKS + 1),
            0,
            "expired tickets hold nothing"
        );
        assert_eq!(
            InviteTicket::issue(ticket.code, 0, 1, 0),
            Err(RelayError::BadInput),
            "version zero never issues"
        );
    }

    #[test]
    fn stun_binding_roundtrips_v4_and_v6() {
        let server = StunServer::new(socket(3478));
        assert_eq!(server.public_addr(), socket(3478));
        let tx = [7u8; 12];
        let request = encode_binding_request(tx);
        assert_eq!(decode_binding_request(&request), Ok(tx));
        let response = server.serve(&request).expect("valid requests answer");
        assert_eq!(
            decode_binding_response(&response, tx),
            Ok(socket(3478)),
            "mapped address echoes the server view"
        );
        let v6 = SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            3478,
        );
        let v6_response = encode_binding_response(tx, v6);
        assert_eq!(decode_binding_response(&v6_response, tx), Ok(v6));
    }

    #[test]
    fn stun_rejects_junk() {
        assert_eq!(
            decode_binding_request(&[0u8; 19]),
            Err(RelayError::Truncated),
            "short requests refuse"
        );
        let mut bad_kind = encode_binding_request([1u8; 12]);
        bad_kind[0] = 0xFF;
        assert!(
            decode_binding_request(&bad_kind).is_err(),
            "bad type refuses"
        );
        let mut bad_cookie = encode_binding_request([1u8; 12]);
        bad_cookie[4] ^= 0xFF;
        assert_eq!(
            decode_binding_request(&bad_cookie),
            Err(RelayError::BadMagic),
            "bad cookie refuses"
        );
        let server = StunServer::new(socket(1));
        assert!(server.serve(&[0u8; 5]).is_err(), "junk never answers");
        let tx = [2u8; 12];
        let response = encode_binding_response(tx, socket(9));
        assert_eq!(
            decode_binding_response(&response, [3u8; 12]),
            Err(RelayError::BadInput),
            "tx mismatch refuses"
        );
    }

    #[test]
    fn candidates_prioritize_host_over_relay() {
        let set = gather_candidates(socket(100), Some(socket(200)), Some(socket(300)))
            .expect("valid addrs gather");
        assert_eq!(
            set.iter().map(|c| c.kind).collect::<Vec<_>>(),
            vec![
                CandidateKind::Host,
                CandidateKind::Srflx,
                CandidateKind::Relay
            ],
            "host beats srflx beats relay"
        );
        assert!(set[0].priority() > set[1].priority());
        assert!(set[1].priority() > set[2].priority());
        assert_eq!(CandidateKind::preference(CandidateKind::Host), 126);
        assert_eq!(CandidateKind::from_u8(9), None, "unknown kinds fail");
        for cand in &set {
            assert_eq!(Candidate::decode(&cand.encode()), Ok(*cand), "roundtrip");
        }
        assert_eq!(
            gather_candidates(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                None,
                None
            ),
            Err(RelayError::BadInput),
            "port zero never gathers"
        );
        assert_eq!(
            Candidate::decode(&[9, 4, 0, 0, 1, 2, 3, 4]),
            Err(RelayError::BadKind(9)),
            "bad candidate kind refuses"
        );
    }

    #[test]
    fn candidate_decode_refuses_port_zero() {
        // Port zero never gathers, so wire bytes carrying it are junk too:
        // the punch must never try a zero-port candidate.
        let zero_v4 = Candidate {
            kind: CandidateKind::Host,
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        };
        assert_eq!(
            Candidate::decode(&zero_v4.encode()),
            Err(RelayError::BadInput),
            "port-zero wire candidates refuse"
        );
        let zero_v6 = Candidate {
            kind: CandidateKind::Srflx,
            addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0),
        };
        assert_eq!(
            Candidate::decode(&zero_v6.encode()),
            Err(RelayError::BadInput),
            "v6 port-zero wire candidates refuse"
        );
    }

    #[test]
    fn relay_register_forward_drain_fifo() {
        let mut server = RelayServer::new(RelayConfig::default());
        assert_eq!(server.peer_count(), 0);
        assert_eq!(server.config(), RelayConfig::default());
        server
            .register(11, server.config().token)
            .expect("token ok");
        server
            .register(22, server.config().token)
            .expect("token ok");
        server
            .register(11, server.config().token)
            .expect("re-register ok");
        assert_eq!(server.peer_count(), 2);
        server
            .forward(11, 22, b"hello")
            .expect("known peers forward");
        server.forward(11, 22, b"again").expect("fifo keeps order");
        assert_eq!(server.drain(22), vec![b"hello".to_vec(), b"again".to_vec()]);
        assert!(server.drain(22).is_empty(), "drains empty the queue");
        assert!(server.drain(99).is_empty(), "unknown peers drain empty");
        let stats = server.stats();
        assert_eq!((stats.forwarded, stats.bytes_forwarded), (2, 10));
        assert_eq!(stats.dropped_full, 0);
    }

    #[test]
    fn relay_refuses_loudly() {
        let mut server = RelayServer::new(RelayConfig::new(0xBEEF, 2, 1).expect("config"));
        assert_eq!(server.register(0, 0xBEEF), Err(RelayError::BadInput));
        assert_eq!(server.register(1, 0xBAD), Err(RelayError::Unauthorized));
        assert_eq!(
            RelayConfig::new(0, 2, 1),
            Err(RelayError::Unauthorized),
            "zero tokens never configure"
        );
        assert!(RelayConfig::new(1, 0, 1).is_err(), "zero peers refuse");
        assert!(RelayConfig::new(1, 1, 0).is_err(), "zero queues refuse");
        server.register(1, 0xBEEF).expect("first");
        server.register(2, 0xBEEF).expect("second");
        assert_eq!(server.register(3, 0xBEEF), Err(RelayError::SlotFull));
        assert_eq!(
            server.forward(9, 2, b"x"),
            Err(RelayError::UnknownPeer(9)),
            "unknown senders refuse"
        );
        assert_eq!(
            server.forward(1, 9, b"x"),
            Err(RelayError::UnknownPeer(9)),
            "unknown destinations refuse"
        );
        assert_eq!(server.forward(1, 2, b""), Err(RelayError::BadInput));
        assert_eq!(
            server.forward(1, 2, &vec![0u8; MAX_RELAY_BYTES + 1]),
            Err(RelayError::TooLarge),
            "oversize never queues"
        );
        server.forward(1, 2, b"fills").expect("one slot");
        assert_eq!(
            server.forward(1, 2, b"full"),
            Err(RelayError::QueueFull),
            "full queues refuse, never drop"
        );
        assert_eq!(server.stats().dropped_full, 1);
    }

    #[test]
    fn relay_wire_roundtrips_and_rejects() {
        let messages = [
            RelayMsg::Register {
                client_id: 5,
                token: 0xBEEF,
            },
            RelayMsg::Registered { client_id: 5 },
            RelayMsg::Forward {
                from: 5,
                to: 6,
                payload: b"game".to_vec(),
            },
            RelayMsg::Heartbeat { client_id: 5 },
            RelayMsg::Denied {
                client_id: 5,
                code: 1,
            },
        ];
        for message in messages {
            let bytes = message.encode().expect("valid messages encode");
            assert!(bytes.len() <= MAX_RELAY_BYTES + 22, "relay never fragments");
            assert_eq!(RelayMsg::decode(&bytes), Ok(message), "roundtrip");
        }
        assert_eq!(
            RelayMsg::decode(&[0u8; 3]),
            Err(RelayError::Truncated),
            "short headers refuse"
        );
        let mut bad_magic = RelayMsg::Heartbeat { client_id: 5 }
            .encode()
            .expect("encode");
        bad_magic[0] ^= 0xFF;
        assert_eq!(RelayMsg::decode(&bad_magic), Err(RelayError::BadMagic));
        let mut bad_kind = RelayMsg::Heartbeat { client_id: 5 }
            .encode()
            .expect("encode");
        bad_kind[3] = 0x7F;
        assert_eq!(RelayMsg::decode(&bad_kind), Err(RelayError::BadKind(0x7F)));
        let oversize = RelayMsg::Forward {
            from: 1,
            to: 2,
            payload: vec![0u8; MAX_RELAY_BYTES + 1],
        };
        assert_eq!(oversize.encode(), Err(RelayError::TooLarge));
        assert_eq!(
            RelayMsg::decode(&vec![0u8; MAX_RELAY_BYTES + 23]),
            Err(RelayError::TooLarge)
        );
    }

    #[test]
    fn punch_goes_direct_on_open_and_cone() {
        for nat in [NatKind::Open, NatKind::Cone] {
            let mut punch = HolePunch::new(nat);
            assert_eq!(punch.state(), PunchState::Gathering);
            punch.tick(true);
            assert_eq!(
                punch.state(),
                PunchState::Gathering,
                "gathering never jumps"
            );
            punch.on_gathered();
            assert_eq!(punch.state(), PunchState::Checking);
            punch.tick(true);
            assert!(punch.is_direct(), "open and cone go direct");
            assert!(!punch.needs_relay());
        }
    }

    #[test]
    fn punch_falls_back_on_symmetric_and_timeout() {
        let mut symmetric = HolePunch::new(NatKind::Symmetric);
        symmetric.on_gathered();
        for _ in 0..PUNCH_TIMEOUT_TICKS {
            assert!(!symmetric.needs_relay(), "fallback only at the deadline");
            symmetric.tick(true);
        }
        assert!(symmetric.needs_relay(), "symmetric always relays");
        assert_eq!(symmetric.attempts(), PUNCH_TIMEOUT_TICKS);
        let mut quiet = HolePunch::new(NatKind::Cone);
        quiet.on_gathered();
        for _ in 0..PUNCH_TIMEOUT_TICKS {
            quiet.tick(false);
        }
        assert!(quiet.needs_relay(), "silent checks time out to relay");
        // Terminal states stick.
        quiet.tick(true);
        assert!(quiet.needs_relay(), "fallback never flips back");
    }

    #[test]
    fn signal_roundtrips_and_decides() {
        let mut state = 4242u64;
        let ticket =
            InviteTicket::issue(InviteCode::mint(&mut state), 3, 0x1234_5678, 50).expect("issue");
        let request = Signal::Request {
            code: ticket.code.as_bytes(),
            version: 3,
            rules: 0x1234_5678,
            token: 11,
        };
        assert_eq!(request.token(), 11);
        let bytes = request.encode().expect("request encodes");
        assert_eq!(Signal::decode(&bytes), Ok(request));
        let accept = decide_join(&ticket, ticket.code.as_bytes(), 3, 0x1234_5678, 11, 1, 60);
        assert_eq!(accept, Signal::Accept { token: 11, slot: 1 });
        assert_eq!(accept.token(), 11);
        let stale = decide_join(
            &ticket,
            ticket.code.as_bytes(),
            3,
            0x1234_5678,
            11,
            1,
            50_000,
        );
        assert!(matches!(
            stale,
            Signal::Refuse {
                reason: RefuseCode::Expired,
                ..
            }
        ));
        let wrong_code = decide_join(&ticket, [9u8; INVITE_BYTES], 3, 0x1234_5678, 11, 1, 60);
        assert!(
            matches!(
                wrong_code,
                Signal::Refuse {
                    reason: RefuseCode::Expired,
                    ..
                }
            ),
            "unknown codes read as expired, never as valid"
        );
        let mismatched = decide_join(&ticket, ticket.code.as_bytes(), 4, 0x1234_5678, 11, 1, 60);
        match mismatched {
            Signal::Refuse {
                reason,
                want_version,
                got_version,
                want_rules,
                got_rules,
                ..
            } => {
                assert_eq!(reason, RefuseCode::VersionMismatch);
                assert_eq!((want_version, got_version), (3, 4));
                assert_eq!((want_rules, got_rules), (0x1234_5678, 0x1234_5678));
            }
            other => panic!("version drift must refuse, got {other:?}"),
        }
        let zero_token = decide_join(&ticket, ticket.code.as_bytes(), 3, 0x1234_5678, 0, 1, 60);
        assert!(matches!(
            zero_token,
            Signal::Refuse {
                reason: RefuseCode::BadInput,
                ..
            }
        ));
        let encoded = mismatched.encode().expect("refuse encodes");
        let decoded = Signal::decode(&encoded).expect("refuse decodes");
        assert_eq!(decoded, mismatched);
        assert_eq!(RefuseCode::from_u8(9), None, "unknown refuse codes fail");
        assert_eq!(Signal::decode(&[0u8; 2]), Err(RelayError::Truncated));
        let mut bad_magic = encoded.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(Signal::decode(&bad_magic), Err(RelayError::BadMagic));
        let mut bad_kind = encoded;
        bad_kind[3] = 0x7F;
        assert_eq!(Signal::decode(&bad_kind), Err(RelayError::BadKind(0x7F)));
    }

    #[test]
    fn ticket_carries_protocol_and_user_bound_to_the_code() {
        let mut state = 4242u64;
        let ticket =
            InviteTicket::issue(InviteCode::mint(&mut state), 3, 0x1234_5678, 50).expect("issue");
        assert_eq!(
            ticket.protocol, RELAY_PROTOCOL_ID,
            "issued tickets pin the relay"
        );
        let mut twin_state = 4242u64;
        let twin = InviteTicket::issue(InviteCode::mint(&mut twin_state), 3, 0x1234_5678, 50)
            .expect("issue");
        assert_eq!((twin.protocol, twin.user), (ticket.protocol, ticket.user));
        let mut other_state = 4243u64;
        let other = InviteTicket::issue(InviteCode::mint(&mut other_state), 3, 0x1234_5678, 50)
            .expect("issue");
        assert_ne!(other.user, ticket.user, "user bytes follow the code");
    }

    #[test]
    fn foreign_protocol_tickets_refuse_as_version_mismatch() {
        let mut state = 4242u64;
        let mut ticket =
            InviteTicket::issue(InviteCode::mint(&mut state), 3, 0x1234_5678, 50).expect("issue");
        ticket.protocol = 0xDEAD_BEEF;
        match decide_join(&ticket, ticket.code.as_bytes(), 3, 0x1234_5678, 11, 1, 60) {
            Signal::Refuse {
                reason,
                want_version,
                got_version,
                ..
            } => {
                assert_eq!(reason, RefuseCode::VersionMismatch);
                assert_eq!((want_version, got_version), (3, 3));
            }
            other => panic!("foreign protocol must refuse, got {other:?}"),
        }
    }

    #[test]
    fn challenge_roundtrips_per_pair_and_refuses_loudly() {
        let challenge = mint_challenge(0x0A30_2040, 7);
        assert_eq!(
            mint_challenge(0x0A30_2040, 7),
            challenge,
            "same pair, same challenge"
        );
        assert_ne!(
            mint_challenge(0x0A30_2040, 8),
            challenge,
            "another client mints another challenge"
        );
        let answer = answer_challenge(challenge, 0xBEEF);
        assert!(
            verify_answer(challenge, 0xBEEF, answer),
            "honest answers verify"
        );
        assert!(
            !verify_answer(challenge, 0xBEEF, answer.wrapping_add(1)),
            "forged answers refuse"
        );
        assert!(
            !verify_answer(challenge, 0, answer_challenge(challenge, 0)),
            "zero tokens refuse even with matching math"
        );
    }

    #[test]
    fn combat_snapshots_never_merge_peer_to_peer() {
        const AUTHORITY: u64 = 0xE204;
        let mut gate = CombatSyncGate::new(AUTHORITY).expect("a match serves one authority");
        assert_eq!(gate.authority(), AUTHORITY);
        let row = |author, tape, state| CombatSync {
            author,
            tape: Some(tape),
            state,
        };
        assert_eq!(
            gate.apply(row(SyncAuthor::Authority(AUTHORITY), 10, 0xA1)),
            Ok(())
        );
        let held = gate.state();
        // A peer's row for the same tick would merge two writers.
        assert_eq!(
            gate.apply(row(SyncAuthor::Peer(2), 10, 0xB2)),
            Err(P2pRefusal::PeerAuthored)
        );
        assert_eq!(gate.state(), held, "a peer row never moves combat state");
        // A peer's row for a fresh tick merges just as little.
        assert_eq!(
            gate.apply(row(SyncAuthor::Peer(3), 11, 0xC3)),
            Err(P2pRefusal::PeerAuthored)
        );
        assert_eq!(gate.state(), held);
        assert_eq!(gate.held_rows(), vec![10], "no peer row is ever held");
        assert_eq!(gate.peer_rows_refused(), 2);
        // A second authority claiming a tick is the same merge, refused.
        assert_eq!(
            gate.apply(row(SyncAuthor::Authority(AUTHORITY + 1), 11, 0xD4)),
            Err(P2pRefusal::ForeignAuthority)
        );
        assert_eq!(
            gate.apply(row(SyncAuthor::Authority(AUTHORITY), 10, 0xE5)),
            Err(P2pRefusal::HeldRow),
            "one tick never takes two states"
        );
        assert_eq!(gate.state(), held);
        // The authority's identical row is a resend, not a merge.
        assert_eq!(
            gate.apply(row(SyncAuthor::Authority(AUTHORITY), 10, 0xA1)),
            Ok(())
        );
        assert_eq!(gate.held_rows(), vec![10]);
    }

    #[test]
    fn every_combat_state_change_carries_authority_and_tape() {
        const AUTHORITY: u64 = 0xE204;
        let mut gate = CombatSyncGate::new(AUTHORITY).expect("a match serves one authority");
        // A peer stamp is a proposal, not a state change.
        assert_eq!(
            gate.apply(CombatSync {
                author: SyncAuthor::Peer(1),
                tape: Some(0),
                state: 1,
            }),
            Err(P2pRefusal::PeerAuthored)
        );
        // Authority but no tape row: nothing authoritative recorded it.
        assert_eq!(
            gate.apply(CombatSync {
                author: SyncAuthor::Authority(AUTHORITY),
                tape: None,
                state: 1,
            }),
            Err(P2pRefusal::NoTape)
        );
        // Another server's row is not this match's authority.
        assert_eq!(
            gate.apply(CombatSync {
                author: SyncAuthor::Authority(0xBEEF),
                tape: Some(0),
                state: 1,
            }),
            Err(P2pRefusal::ForeignAuthority)
        );
        assert_eq!(gate.state(), 0, "no half-stamped change moves state");
        assert!(gate.held_rows().is_empty(), "and none is ever held");
        // Authority plus tape is the only applied shape.
        assert_eq!(
            gate.apply(CombatSync {
                author: SyncAuthor::Authority(AUTHORITY),
                tape: Some(0),
                state: 7,
            }),
            Ok(())
        );
        assert_eq!(gate.state(), 7);
        assert_eq!(gate.held_rows(), vec![0]);
        assert!(matches!(
            CombatSyncGate::new(0),
            Err(RelayError::Unauthorized)
        ));
    }

    #[test]
    fn a_p2p_combat_path_fails_the_gate() {
        const AUTHORITY: u64 = 0xE204;
        let mut gate = CombatSyncGate::new(AUTHORITY).expect("a match serves one authority");
        // Two peers exchange their own combat rows with no server in the path.
        let rows = [
            CombatSync {
                author: SyncAuthor::Peer(2),
                tape: Some(0),
                state: 0x11,
            },
            CombatSync {
                author: SyncAuthor::Peer(3),
                tape: Some(1),
                state: 0x22,
            },
            CombatSync {
                author: SyncAuthor::Peer(2),
                tape: Some(2),
                state: 0x33,
            },
        ];
        for change in rows {
            assert_eq!(
                gate.apply(change),
                Err(P2pRefusal::PeerAuthored),
                "{change:?} must fail the gate"
            );
        }
        assert_eq!(gate.state(), 0, "a P2P match holds no combat state");
        assert!(gate.held_rows().is_empty(), "and no tape row");
        assert_eq!(gate.peer_rows_refused(), 3);
        // The same rows under the authority's stamp and tape apply: the gate
        // judges authorship, not the bytes.
        let mut stamped = CombatSyncGate::new(AUTHORITY).expect("a match serves one authority");
        for change in rows {
            let authoritative = CombatSync {
                author: SyncAuthor::Authority(AUTHORITY),
                ..change
            };
            assert_eq!(stamped.apply(authoritative), Ok(()));
        }
        assert_eq!(stamped.state(), 0x33);
        assert_eq!(stamped.held_rows(), vec![0, 1, 2]);
        assert_eq!(stamped.peer_rows_refused(), 0);
    }

    fn turn_ip(last: u8, port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, last)), port)
    }

    #[test]
    fn turn_allocation_grants_clamps_refreshes_and_expires() {
        let mut turn = TurnAllocator::new(49_152).expect("base port");
        assert!(matches!(TurnAllocator::new(0), Err(RelayError::BadInput)));
        turn.advance(100);
        let first = turn.allocate(1, 0).expect("allocate");
        assert_eq!(
            (first.relayed_port, first.ttl_ticks),
            (49_152, TURN_ALLOC_TTL_TICKS)
        );
        assert!(
            first.is_live(100 + TURN_ALLOC_TTL_TICKS),
            "the lifetime bar is inclusive"
        );
        assert!(!first.is_live(101 + TURN_ALLOC_TTL_TICKS));
        let second = turn
            .allocate(2, TURN_ALLOC_MAX_TICKS + 10)
            .expect("allocate");
        assert_eq!(second.ttl_ticks, TURN_ALLOC_MAX_TICKS, "a long ask clamps");
        assert_eq!(second.relayed_port, 49_153);
        assert!(
            matches!(turn.allocate(1, 0), Err(RelayError::SlotFull)),
            "a live allocation is never replaced"
        );
        turn.advance(100 + TURN_ALLOC_TTL_TICKS);
        assert!(turn.live(1).is_some(), "live exactly at the bar");
        assert!(
            turn.refresh(1, 0).expect("refresh").is_none(),
            "zero deletes"
        );
        assert!(turn.live(1).is_none());
        assert!(
            matches!(turn.refresh(1, 60), Err(RelayError::Expired)),
            "a refresh never revives"
        );
        assert_eq!(turn.stats().released, 1);
        turn.advance(100 + TURN_ALLOC_TTL_TICKS + TURN_ALLOC_MAX_TICKS + 1);
        assert!(turn.live(2).is_none(), "the long allocation lapsed");
        let fresh = turn.allocate(2, 60).expect("re-allocate");
        assert_eq!(fresh.relayed_port, 49_152, "a lapsed port is reusable");
    }

    #[test]
    fn turn_hub_forwards_only_after_create_permission_and_refuses_stale() {
        let config = RelayConfig::default();
        let mut hub = RelayHub::new(config).with_turn(49_160).expect("turn port");
        let (a, b, c) = (turn_ip(1, 5001), turn_ip(2, 5002), turn_ip(3, 5003));
        for (id, addr) in [(1, a), (2, b), (3, c)] {
            let register = RelayMsg::Register {
                client_id: id,
                token: config.token,
            }
            .encode()
            .expect("register");
            assert_eq!(hub.handle(addr, &register).len(), 1);
        }
        let forward = |from, to| {
            RelayMsg::Forward {
                from,
                to,
                payload: b"turn".to_vec(),
            }
            .encode()
            .expect("forward")
        };
        assert!(
            matches!(hub.allocate(9, 0), Err(RelayError::UnknownPeer(9))),
            "only registered clients allocate"
        );
        assert!(hub.handle(a, &forward(1, 2)).is_empty());
        assert_eq!(hub.turn_stats().expect("turn on").refused_stale, 1);
        assert_eq!(hub.server().stats().forwarded, 0);
        for id in [1, 2] {
            hub.allocate(id, 0).expect("allocate");
        }
        assert!(hub.handle(a, &forward(1, 2)).is_empty());
        assert_eq!(hub.turn_stats().expect("turn on").refused_unpermitted, 1);
        hub.create_permission(1, 2).expect("permission");
        assert_eq!(hub.handle(a, &forward(1, 2)), vec![(b, forward(1, 2))]);
        assert!(
            hub.handle(a, &forward(1, 3)).is_empty(),
            "a permission names one peer IP"
        );
        hub.advance(TURN_PERMISSION_TTL_TICKS);
        assert_eq!(hub.handle(a, &forward(1, 2)).len(), 1, "inclusive bar");
        hub.advance(TURN_PERMISSION_TTL_TICKS + 1);
        assert!(hub.handle(a, &forward(1, 2)).is_empty());
        hub.refresh(1, 0).expect("release");
        assert!(hub.handle(a, &forward(1, 2)).is_empty());
        let stats = hub.turn_stats().expect("turn on");
        assert_eq!(stats.permitted, 2, "two permitted forwards");
        assert_eq!(stats.refused_unpermitted, 3);
        assert_eq!(stats.refused_stale, 2, "before allocate and after release");
        assert_eq!(hub.server().stats().forwarded, 2);
    }

    #[test]
    fn turn_channel_binds_multiplex_peers_and_imply_permission() {
        let config = RelayConfig::default();
        let mut hub = RelayHub::new(config).with_turn(49_200).expect("turn port");
        let addrs = [
            turn_ip(1, 5001),
            turn_ip(2, 5002),
            turn_ip(3, 5003),
            turn_ip(4, 5004),
        ];
        for (index, addr) in addrs.iter().enumerate() {
            let id = u64::try_from(index + 1).expect("id");
            let register = RelayMsg::Register {
                client_id: id,
                token: config.token,
            }
            .encode()
            .expect("register");
            hub.handle(*addr, &register);
        }
        hub.allocate(1, TURN_ALLOC_MAX_TICKS).expect("allocate");
        let forward = |to| {
            RelayMsg::Forward {
                from: 1,
                to,
                payload: b"chan".to_vec(),
            }
            .encode()
            .expect("forward")
        };
        hub.bind_channel(1, TURN_CHANNEL_FIRST, 2).expect("bind 2");
        hub.bind_channel(1, TURN_CHANNEL_FIRST + 1, 3)
            .expect("bind 3");
        assert!(hub.handle(addrs[0], &forward(4)).is_empty(), "4 is unbound");
        assert_eq!(
            hub.handle(addrs[0], &forward(2)),
            vec![(addrs[1], forward(2))]
        );
        assert_eq!(
            hub.handle(addrs[0], &forward(3)),
            vec![(addrs[2], forward(3))]
        );
        assert!(matches!(
            hub.bind_channel(1, TURN_CHANNEL_FIRST - 1, 2),
            Err(RelayError::BadInput)
        ));
        assert!(
            matches!(
                hub.bind_channel(1, TURN_CHANNEL_FIRST, 3),
                Err(RelayError::BadInput)
            ),
            "a channel never moves peer"
        );
        assert!(
            matches!(
                hub.bind_channel(1, TURN_CHANNEL_FIRST + 2, 2),
                Err(RelayError::BadInput)
            ),
            "a peer holds one channel"
        );
        hub.advance(TURN_CHANNEL_TTL_TICKS);
        assert_eq!(hub.handle(addrs[0], &forward(2)).len(), 1, "inclusive bar");
        hub.bind_channel(1, TURN_CHANNEL_FIRST, 2).expect("refresh");
        hub.advance(TURN_CHANNEL_TTL_TICKS + 1);
        assert_eq!(
            hub.handle(addrs[0], &forward(2)),
            vec![(addrs[1], forward(2))]
        );
        hub.advance(TURN_CHANNEL_TTL_TICKS * 2 + 1);
        assert!(
            hub.handle(addrs[0], &forward(2)).is_empty(),
            "channels lapse"
        );
        assert_eq!(hub.turn_stats().expect("turn on").refused_unpermitted, 2);
    }

    #[test]
    fn turn_forward_fixture_has_zero_unpermitted_or_stale_forwards() {
        let config = RelayConfig::default();
        let mut hub = RelayHub::new(config).with_turn(49_240).expect("turn port");
        let addrs = [turn_ip(1, 6001), turn_ip(2, 6002), turn_ip(3, 6003)];
        for (index, addr) in addrs.iter().enumerate() {
            let id = u64::try_from(index + 1).expect("id");
            let register = RelayMsg::Register {
                client_id: id,
                token: config.token,
            }
            .encode()
            .expect("register");
            hub.handle(*addr, &register);
        }
        hub.allocate(1, TURN_ALLOC_MAX_TICKS).expect("allocate");
        for id in [2, 3] {
            hub.allocate(id, 0).expect("allocate");
        }
        hub.create_permission(1, 2).expect("permission");
        hub.bind_channel(1, TURN_CHANNEL_FIRST, 3).expect("bind");
        // (tick, from, to, delivered): the expected column follows from the
        // lifetimes alone, so a forward that slips a lapsed gate is a failure.
        let tape = [
            (0, 1, 2, true),
            (0, 1, 3, true),
            (0, 2, 1, false),
            (18_000, 1, 2, true),
            (18_001, 1, 2, false),
            (36_000, 1, 3, true),
            (36_001, 1, 3, false),
            (54_000, 1, 3, false),
        ];
        let forward = |from, to| {
            RelayMsg::Forward {
                from,
                to,
                payload: b"fixture".to_vec(),
            }
            .encode()
            .expect("forward")
        };
        let mut delivered = 0;
        for (at, from, to, expect) in tape {
            hub.advance(at);
            let src = addrs[usize::try_from(from).expect("from") - 1];
            let got = !hub.handle(src, &forward(from, to)).is_empty();
            assert_eq!(got, expect, "tick {at}: {from} -> {to}");
            delivered += usize::from(got);
        }
        // A released allocation is stale, never a permitted forward.
        hub.refresh(1, 0).expect("release");
        hub.advance(54_001);
        assert!(hub.handle(addrs[0], &forward(1, 2)).is_empty());
        let stats = hub.turn_stats().expect("turn on");
        assert_eq!(delivered, 4, "only the permitted forwards landed");
        assert_eq!(stats.permitted, 4);
        assert_eq!(stats.refused_unpermitted, 4);
        assert_eq!(stats.refused_stale, 1, "after release the path is stale");
        assert_eq!(hub.server().stats().forwarded, 4);
    }
}
