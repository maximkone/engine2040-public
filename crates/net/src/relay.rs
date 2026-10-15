//! relay: internet join over invite codes with hole punch plus relay fallback.
//!
//! Purpose: let two machines on different networks join one server match by a
//! short invite code, trying a direct path first and falling back to a
//! self-hosted forward-only relay that always works.
//! Owner: net lane. Units: ticks at 60 Hz, bytes on the wire, addresses.
//! Determinism: invite mint, STUN/ICE shapes and the punch machine are seeded
//! integer-only; sockets and wall clocks never enter sim hashes.
//! Errors: RelayError; invalid wire or stale tickets refuse loudly.
//!
//! Runtime: the self-hosted binary drives RelayServer on workspace tokio off
//! the fixed tick; the sim consumes only validated inputs at tick bounds.

use engine_core::{hash::GOLDEN_GAMMA, mix64};
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

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
/// STUN magic cookie (RFC 5389 fixed word, checked on decode).
pub const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
/// STUN binding request type word.
pub const STUN_BINDING_REQUEST: u16 = 0x0001;
/// STUN binding success response type word.
pub const STUN_BINDING_RESPONSE: u16 = 0x0101;
/// STUN XOR-MAPPED-ADDRESS attribute type word.
pub const STUN_ATTR_XOR_MAPPED: u16 = 0x0020;

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

/// One SplitMix64 step over held state through the shared owner.
fn splitmix_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(GOLDEN_GAMMA);
    mix64(*state)
}

/// Short join secret: 6 bytes minted from a seeded stream, read aloud as 10
/// chars. Same seed gives the same code on every machine.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InviteCode(pub [u8; INVITE_BYTES]);

/// Human alphabet: Crockford-style, no I/L/O/U, so codes read aloud cleanly.
const INVITE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKMNPQRSTVWXYZ0123456789";

impl InviteCode {
    /// Mint one code from `state`; deterministic per seed.
    pub fn mint(state: &mut u64) -> Self {
        let mut bytes = [0u8; INVITE_BYTES];
        let mut word = splitmix_next(state);
        for chunk in bytes.chunks_mut(4) {
            let part = word.to_le_bytes();
            chunk.copy_from_slice(&part[..chunk.len()]);
            word = splitmix_next(state);
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
/// Expiry is tick counts, never wall time, so replays stay exact.
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
}

impl InviteTicket {
    /// Issue a ticket for `now` with the standard lifetime.
    pub fn issue(code: InviteCode, version: u16, rules: u64, now: u32) -> Result<Self, RelayError> {
        if version == 0 {
            return Err(RelayError::BadInput);
        }
        Ok(Self {
            code,
            version,
            rules,
            issued_at: now,
            ttl_ticks: INVITE_TTL_TICKS,
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
// STUN binding shape: request/response plus XOR-MAPPED-ADDRESS, IPv4/IPv6.
// ---------------------------------------------------------------------------

/// Encode a 20-byte binding request for transaction `tx`.
pub fn encode_binding_request(tx: [u8; 12]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out[0..2].copy_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    out[2..4].copy_from_slice(&0u16.to_be_bytes());
    out[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    out[8..20].copy_from_slice(&tx);
    out
}

/// Decode a binding request, returning its transaction id.
pub fn decode_binding_request(bytes: &[u8]) -> Result<[u8; 12], RelayError> {
    if bytes.len() != 20 {
        return Err(RelayError::Truncated);
    }
    if u16::from_be_bytes([bytes[0], bytes[1]]) != STUN_BINDING_REQUEST {
        return Err(RelayError::BadKind(bytes[0]));
    }
    if u16::from_be_bytes([bytes[2], bytes[3]]) != 0 {
        return Err(RelayError::BadInput);
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != STUN_MAGIC_COOKIE {
        return Err(RelayError::BadMagic);
    }
    let mut tx = [0u8; 12];
    tx.copy_from_slice(&bytes[8..20]);
    Ok(tx)
}

/// Encode a binding success response echoing `tx` with the observed address
/// XOR-mapped per RFC 5389 (obfuscation by fixed xor, not crypto).
pub fn encode_binding_response(tx: [u8; 12], mapped: SocketAddr) -> Vec<u8> {
    let attr: Vec<u8> = match mapped.ip() {
        IpAddr::V4(v4) => {
            let mut attr = vec![0u8; 8];
            attr[0] = 0;
            attr[1] = 1;
            let xport = mapped.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16;
            attr[2..4].copy_from_slice(&xport.to_be_bytes());
            let octets = v4.octets();
            let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
            for (i, octet) in octets.iter().enumerate() {
                attr[4 + i] = octet ^ cookie[i];
            }
            attr
        }
        IpAddr::V6(v6) => {
            let mut attr = vec![0u8; 20];
            attr[0] = 0;
            attr[1] = 2;
            let xport = mapped.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16;
            attr[2..4].copy_from_slice(&xport.to_be_bytes());
            let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
            for (i, octet) in v6.octets().iter().enumerate() {
                let mask = if i < 4 { cookie[i] } else { tx[i - 4] };
                attr[4 + i] = octet ^ mask;
            }
            attr
        }
    };
    let mut out = vec![0u8; 20 + 4 + attr.len()];
    out[0..2].copy_from_slice(&STUN_BINDING_RESPONSE.to_be_bytes());
    out[2..4].copy_from_slice(&(attr.len() as u16).to_be_bytes());
    out[4..8].copy_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    out[8..20].copy_from_slice(&tx);
    out[20..22].copy_from_slice(&STUN_ATTR_XOR_MAPPED.to_be_bytes());
    out[22..24].copy_from_slice(&(attr.len() as u16).to_be_bytes());
    out[24..].copy_from_slice(&attr);
    out
}

/// Decode a binding response, checking the `tx` echo and returning the
/// mapped address.
pub fn decode_binding_response(bytes: &[u8], tx: [u8; 12]) -> Result<SocketAddr, RelayError> {
    if bytes.len() < 28 {
        return Err(RelayError::Truncated);
    }
    if u16::from_be_bytes([bytes[0], bytes[1]]) != STUN_BINDING_RESPONSE {
        return Err(RelayError::BadKind(bytes[0]));
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != STUN_MAGIC_COOKIE {
        return Err(RelayError::BadMagic);
    }
    if bytes[8..20] != tx {
        return Err(RelayError::BadInput);
    }
    if u16::from_be_bytes([bytes[20], bytes[21]]) != STUN_ATTR_XOR_MAPPED {
        return Err(RelayError::BadInput);
    }
    let attr_len = usize::from(u16::from_be_bytes([bytes[22], bytes[23]]));
    if bytes.len() != 24 + attr_len {
        return Err(RelayError::Truncated);
    }
    let attr = &bytes[24..];
    if attr.len() < 8 || attr[0] != 0 {
        return Err(RelayError::BadInput);
    }
    let xport = u16::from_be_bytes([attr[2], attr[3]]);
    let port = xport ^ (STUN_MAGIC_COOKIE >> 16) as u16;
    let cookie = STUN_MAGIC_COOKIE.to_be_bytes();
    match attr[1] {
        1 => {
            if attr.len() != 8 {
                return Err(RelayError::Truncated);
            }
            let mut octets = [0u8; 4];
            for (i, octet) in octets.iter_mut().enumerate() {
                *octet = attr[4 + i] ^ cookie[i];
            }
            Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port))
        }
        2 => {
            if attr.len() != 20 {
                return Err(RelayError::Truncated);
            }
            let mut octets = [0u8; 16];
            for (i, octet) in octets.iter_mut().enumerate() {
                let mask = if i < 4 { cookie[i] } else { tx[i - 4] };
                *octet = attr[4 + i] ^ mask;
            }
            Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port))
        }
        family => Err(RelayError::BadKind(family)),
    }
}

/// Deterministic STUN stand-in: answers binding requests with the configured
/// public address. The deployment runs a real server; the shape is identical.
pub struct StunServer {
    public: SocketAddr,
}

impl StunServer {
    /// Serve mapped-address lookups as `public`.
    pub fn new(public: SocketAddr) -> Self {
        Self { public }
    }

    /// Advertised public address.
    pub fn public_addr(&self) -> SocketAddr {
        self.public
    }

    /// Answer one request with a success response, or refuse junk loudly.
    pub fn serve(&self, request: &[u8]) -> Result<Vec<u8>, RelayError> {
        let tx = decode_binding_request(request)?;
        Ok(encode_binding_response(tx, self.public))
    }
}

// ---------------------------------------------------------------------------
// ICE-lite candidates: host / server-reflexive / relay with fixed priority.
// ---------------------------------------------------------------------------

/// Candidate source, ordered host > srflx > relay by construction.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CandidateKind {
    /// Local interface address.
    Host,
    /// STUN-learned server-reflexive address.
    Srflx,
    /// Relay-allocated address, always works.
    Relay,
}

impl CandidateKind {
    /// Type preference per the ICE shape: host 126, srflx 100, relay 0.
    pub const fn preference(self) -> u32 {
        match self {
            Self::Host => 126,
            Self::Srflx => 100,
            Self::Relay => 0,
        }
    }

    /// Wire code for this kind.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Host => 0,
            Self::Srflx => 1,
            Self::Relay => 2,
        }
    }

    /// Decode a wire code; unknown codes fail.
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Host),
            1 => Some(Self::Srflx),
            2 => Some(Self::Relay),
            _ => None,
        }
    }
}

/// One connectivity candidate: source plus address plus fixed priority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// Source of this address.
    pub kind: CandidateKind,
    /// Address to try.
    pub addr: SocketAddr,
}

impl Candidate {
    /// Priority from type preference only (component and local parts fixed),
    /// so ordering is stable and needs no floats.
    pub fn priority(&self) -> u64 {
        u64::from(self.kind.preference()) << 24
    }

    /// Encode: kind 1, family 1, port 2, address 4 or 16.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; 4];
        out[0] = self.kind.as_u8();
        match self.addr.ip() {
            IpAddr::V4(v4) => {
                out[1] = 4;
                out[2..4].copy_from_slice(&self.addr.port().to_le_bytes());
                out.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                out[1] = 6;
                out[2..4].copy_from_slice(&self.addr.port().to_le_bytes());
                out.extend_from_slice(&v6.octets());
            }
        }
        out
    }

    /// Decode and validate one candidate; junk refuses loudly.
    pub fn decode(bytes: &[u8]) -> Result<Self, RelayError> {
        if bytes.len() != 8 && bytes.len() != 20 {
            return Err(RelayError::Truncated);
        }
        let kind = CandidateKind::from_u8(bytes[0]).ok_or(RelayError::BadKind(bytes[0]))?;
        let port = u16::from_le_bytes([bytes[2], bytes[3]]);
        let ip = match (bytes[1], bytes.len()) {
            (4, 8) => {
                let mut octets = [0u8; 4];
                octets.copy_from_slice(&bytes[4..8]);
                IpAddr::V4(Ipv4Addr::from(octets))
            }
            (6, 20) => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&bytes[4..20]);
                IpAddr::V6(Ipv6Addr::from(octets))
            }
            _ => return Err(RelayError::BadInput),
        };
        Ok(Self {
            kind,
            addr: SocketAddr::new(ip, port),
        })
    }
}

/// Gather the ICE-lite set: host always, srflx and relay when known, sorted
/// by priority highest first. Port zero never gathers.
pub fn gather_candidates(
    host: SocketAddr,
    srflx: Option<SocketAddr>,
    relay: Option<SocketAddr>,
) -> Result<Vec<Candidate>, RelayError> {
    if host.port() == 0 {
        return Err(RelayError::BadInput);
    }
    let mut out = vec![Candidate {
        kind: CandidateKind::Host,
        addr: host,
    }];
    if let Some(addr) = srflx {
        if addr.port() == 0 {
            return Err(RelayError::BadInput);
        }
        out.push(Candidate {
            kind: CandidateKind::Srflx,
            addr,
        });
    }
    if let Some(addr) = relay {
        if addr.port() == 0 {
            return Err(RelayError::BadInput);
        }
        out.push(Candidate {
            kind: CandidateKind::Relay,
            addr,
        });
    }
    out.sort_by_key(|cand| std::cmp::Reverse(cand.priority()));
    Ok(out)
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
    stats: HubStats,
}

impl RelayHub {
    /// Open a hub on `config`.
    pub fn new(config: RelayConfig) -> Self {
        Self {
            server: RelayServer::new(config),
            addrs: std::collections::BTreeMap::new(),
            stats: HubStats::default(),
        }
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
        if let Ok(tx) = decode_binding_request(bytes) {
            self.stats.stun += 1;
            return vec![(src, encode_binding_response(tx, src))];
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

/// Host-side join decision: check the presented code, lifetime and build,
/// then accept into `slot` or refuse with the exact reason plus digest pair.
pub fn decide_join(
    ticket: &InviteTicket,
    code: [u8; INVITE_BYTES],
    version: u16,
    rules: u64,
    token: u64,
    slot: u8,
    now: u32,
) -> Signal {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
