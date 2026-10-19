//! STUN binding with MESSAGE-INTEGRITY-SHA256, ICE-lite candidates, pair
//! nomination and consent freshness: the conformance half of `relay`.
//!
//! Shapes follow RFC 8489 (STUN), RFC 8445 (ICE) and RFC 7675 (consent),
//! reimplemented here with no new dependency: HMAC-SHA256 under
//! `--features crypto` and the documented development-only fold without it.
//! Units: ticks at 60 Hz, bytes on the wire, addresses. Integer-only.

use crate::relay::{InviteTicket, RelayError, INVITE_BYTES};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// STUN magic cookie (RFC 5389 fixed word, checked on decode).
pub const STUN_MAGIC_COOKIE: u32 = 0x2112_A442;
/// STUN binding request type word.
pub const STUN_BINDING_REQUEST: u16 = 0x0001;
/// STUN binding success response type word.
pub const STUN_BINDING_RESPONSE: u16 = 0x0101;
/// STUN XOR-MAPPED-ADDRESS attribute type word.
pub const STUN_ATTR_XOR_MAPPED: u16 = 0x0020;
/// STUN USERNAME attribute type word (RFC 8489 section 14.3).
pub const STUN_ATTR_USERNAME: u16 = 0x0006;
/// STUN MESSAGE-INTEGRITY-SHA256 attribute type word (RFC 8489 section 14.6).
pub const STUN_ATTR_MESSAGE_INTEGRITY_SHA256: u16 = 0x001C;
/// STUN PRIORITY attribute type word (RFC 8445 section 7.1.1).
pub const STUN_ATTR_PRIORITY: u16 = 0x0024;
/// STUN USE-CANDIDATE attribute type word: this check nominates the pair.
pub const STUN_ATTR_USE_CANDIDATE: u16 = 0x0025;

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
/// With credentials it refuses every request whose tag does not verify.
pub struct StunServer {
    public: SocketAddr,
    creds: Option<StunCreds>,
}

impl StunServer {
    /// Serve mapped-address lookups as `public`.
    pub fn new(public: SocketAddr) -> Self {
        Self {
            public,
            creds: None,
        }
    }

    /// Require MESSAGE-INTEGRITY-SHA256 keyed by `creds`: unsigned requests
    /// refuse and every answer carries the same tag.
    #[must_use]
    pub fn requiring(mut self, creds: StunCreds) -> Self {
        self.creds = Some(creds);
        self
    }

    /// Advertised public address.
    pub fn public_addr(&self) -> SocketAddr {
        self.public
    }

    /// Answer one request with a success response, or refuse junk loudly.
    pub fn serve(&self, request: &[u8]) -> Result<Vec<u8>, RelayError> {
        if let Some(creds) = self.creds {
            let check = decode_binding_request_signed(request, &creds)?;
            return Ok(encode_binding_response_signed(
                check.tx,
                self.public,
                &creds,
            ));
        }
        let tx = decode_binding_request(request)?;
        Ok(encode_binding_response(tx, self.public))
    }
}

// ---------------------------------------------------------------------------
// STUN MESSAGE-INTEGRITY-SHA256 keyed by the invite ticket (RFC 8489 §14.6).
// ---------------------------------------------------------------------------

/// Integrity backend: HMAC-SHA256 with the `crypto` feature, the documented
/// development-only fold without it, exactly like `handshake`. A tag never
/// proves identity in a build that did not turn `crypto` on.
#[cfg(feature = "crypto")]
mod integrity {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    /// Tag bytes the crypto backend writes: one SHA-256 block.
    pub(super) const TAG_LEN: usize = 32;

    /// HMAC-SHA256 of `message` under `key`.
    pub(super) fn tag(key: &[u8], message: &[u8]) -> Vec<u8> {
        // HMAC accepts any key length, so the only error path is unreachable;
        // an empty tag fails closed (the peer refuses the message).
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
            return Vec::new();
        };
        mac.update(message);
        mac.finalize().into_bytes().to_vec()
    }

    /// Constant-time check of `tag` under `key`.
    pub(super) fn verify(key: &[u8], message: &[u8], tag: &[u8]) -> bool {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
            return false;
        };
        mac.update(message);
        mac.verify_slice(tag).is_ok()
    }
}

/// Development-only backend without `crypto`: an 8-byte FNV-1a fold of
/// `message || key`. Deterministic but NOT authentication; see `handshake`.
#[cfg(not(feature = "crypto"))]
mod integrity {
    use engine_core::Fnv64;

    /// Tag bytes the development backend writes: one folded `u64`.
    pub(super) const TAG_LEN: usize = 8;

    /// FNV-1a 64 over `message || key`, little-endian.
    fn fold(key: &[u8], message: &[u8]) -> [u8; TAG_LEN] {
        let mut hasher = Fnv64::new();
        hasher.write(message);
        hasher.write(key);
        hasher.finish().to_le_bytes()
    }

    /// The development-only fold as a tag.
    pub(super) fn tag(key: &[u8], message: &[u8]) -> Vec<u8> {
        fold(key, message).to_vec()
    }

    /// Compare the fold; equal bytes pass.
    pub(super) fn verify(key: &[u8], message: &[u8], tag: &[u8]) -> bool {
        tag == fold(key, message).as_slice()
    }
}

/// Bytes the STUN integrity attribute carries in this build: 32 with `crypto`
/// (HMAC-SHA256), 8 without it (the development-only fold).
pub const STUN_INTEGRITY_BYTES: usize = integrity::TAG_LEN;

/// Short-term STUN credential taken from an invite ticket (RFC 8445 §9.1.1.1):
/// the ticket's opaque user bytes are the USERNAME, the invite code is the
/// key. Both ends derive the same pair from the same ticket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StunCreds {
    username: u64,
    key: [u8; INVITE_BYTES],
}

impl StunCreds {
    /// Credential for `username` under `key`; zero names refuse, deny by
    /// default.
    pub fn new(username: u64, key: [u8; INVITE_BYTES]) -> Result<Self, RelayError> {
        if username == 0 {
            return Err(RelayError::Unauthorized);
        }
        Ok(Self { username, key })
    }

    /// The credential an issued ticket pins.
    pub fn from_ticket(ticket: &InviteTicket) -> Self {
        Self {
            username: ticket.user,
            key: ticket.code.as_bytes(),
        }
    }

    /// USERNAME value this credential sends.
    pub fn username(&self) -> u64 {
        self.username
    }

    fn key(&self) -> &[u8] {
        &self.key
    }
}

/// Verified STUN binding check: the transaction, the peer's pair priority and
/// whether USE-CANDIDATE asked to nominate the pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BindingCheck {
    /// Transaction id the response must echo.
    pub tx: [u8; 12],
    /// Peer's pair priority, network order on the wire.
    pub priority: u32,
    /// USE-CANDIDATE was present: a successful answer nominates the pair.
    pub use_candidate: bool,
}

/// One STUN attribute span: 4-byte header, value, padding to a 4-byte bound.
fn attr_span(len: usize) -> usize {
    4 + len.div_ceil(4) * 4
}

/// Append one attribute with zero padding.
fn push_attr(out: &mut Vec<u8>, kind: u16, value: &[u8]) {
    let Ok(len) = u16::try_from(value.len()) else {
        return;
    };
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value);
    out.resize(out.len() + (4 - value.len() % 4) % 4, 0);
}

/// Append MESSAGE-INTEGRITY-SHA256 over everything written so far. The header
/// length is set to the tag end first, so the HMAC covers exactly what RFC
/// 8489 §14.6 names. The relay never writes FINGERPRINT.
fn seal_stun(out: &mut Vec<u8>, key: &[u8]) {
    let Ok(tag_len) = u16::try_from(STUN_INTEGRITY_BYTES) else {
        return;
    };
    let mi_at = out.len();
    let Ok(declared) = u16::try_from(mi_at + 4 + STUN_INTEGRITY_BYTES - 20) else {
        return;
    };
    out[2..4].copy_from_slice(&declared.to_be_bytes());
    let tag = integrity::tag(key, out);
    out.extend_from_slice(&STUN_ATTR_MESSAGE_INTEGRITY_SHA256.to_be_bytes());
    out.extend_from_slice(&tag_len.to_be_bytes());
    out.extend_from_slice(&tag);
}

/// Verify the integrity tag on a STUN binding message under `key` and return
/// the attribute offset. A missing, misplaced or bad tag refuses as
/// [`RelayError::Unauthorized`]: every unsigned binding is a spoof until the
/// tag proves otherwise.
fn verify_stun_integrity(key: &[u8], bytes: &[u8]) -> Result<usize, RelayError> {
    if bytes.len() < 20 {
        return Err(RelayError::Truncated);
    }
    let kind = u16::from_be_bytes([bytes[0], bytes[1]]);
    if kind != STUN_BINDING_REQUEST && kind != STUN_BINDING_RESPONSE {
        return Err(RelayError::BadKind(bytes[0]));
    }
    if u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) != STUN_MAGIC_COOKIE {
        return Err(RelayError::BadMagic);
    }
    let declared = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
    if bytes.len() != 20 + declared {
        return Err(RelayError::Truncated);
    }
    let mut at = 20;
    while at + 4 <= bytes.len() {
        let attr = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        let span = attr_span(len);
        if at + span > bytes.len() {
            return Err(RelayError::Truncated);
        }
        if attr == STUN_ATTR_MESSAGE_INTEGRITY_SHA256 {
            // The tag is the last attribute here: no FINGERPRINT follows, so
            // trailing bytes would change what the HMAC covers.
            if len != STUN_INTEGRITY_BYTES || at + span != bytes.len() {
                return Err(RelayError::Unauthorized);
            }
            let mut input = bytes[..at].to_vec();
            let Ok(mi_end) = u16::try_from(at + span - 20) else {
                return Err(RelayError::TooLarge);
            };
            input[2..4].copy_from_slice(&mi_end.to_be_bytes());
            let tag = &bytes[at + 4..at + span];
            return if integrity::verify(key, &input, tag) {
                Ok(at)
            } else {
                Err(RelayError::Unauthorized)
            };
        }
        at += span;
    }
    Err(RelayError::Unauthorized)
}

/// Encode an authenticated binding request: USERNAME, PRIORITY, optional
/// USE-CANDIDATE, then the integrity tag over all of it.
pub fn encode_binding_request_signed(
    tx: [u8; 12],
    creds: &StunCreds,
    priority: u32,
    nominate: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + STUN_INTEGRITY_BYTES);
    out.extend_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(&tx);
    push_attr(&mut out, STUN_ATTR_USERNAME, &creds.username.to_le_bytes());
    push_attr(&mut out, STUN_ATTR_PRIORITY, &priority.to_be_bytes());
    if nominate {
        push_attr(&mut out, STUN_ATTR_USE_CANDIDATE, &[]);
    }
    seal_stun(&mut out, creds.key());
    out
}

/// Encode a signed binding success response for `mapped`.
pub fn encode_binding_response_signed(
    tx: [u8; 12],
    mapped: SocketAddr,
    creds: &StunCreds,
) -> Vec<u8> {
    let mut out = encode_binding_response(tx, mapped);
    seal_stun(&mut out, creds.key());
    out
}

/// Decode an authenticated binding request deny-by-default: the tag must
/// verify under `creds` and the USERNAME must be the ticket's.
pub fn decode_binding_request_signed(
    bytes: &[u8],
    creds: &StunCreds,
) -> Result<BindingCheck, RelayError> {
    let mi_at = verify_stun_integrity(creds.key(), bytes)?;
    let mut tx = [0u8; 12];
    tx.copy_from_slice(&bytes[8..20]);
    let mut username = None;
    let mut priority = None;
    let mut use_candidate = false;
    let mut at = 20;
    while at < mi_at {
        if at + 4 > mi_at {
            return Err(RelayError::Truncated);
        }
        let attr = u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        let len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        let span = attr_span(len);
        if at + span > mi_at {
            return Err(RelayError::Truncated);
        }
        let value = &bytes[at + 4..at + span];
        match attr {
            STUN_ATTR_USERNAME => {
                if len != 8 {
                    return Err(RelayError::BadInput);
                }
                let raw = value[..8].try_into().map_err(|_| RelayError::Truncated)?;
                username = Some(u64::from_le_bytes(raw));
            }
            STUN_ATTR_PRIORITY => {
                if len != 4 {
                    return Err(RelayError::BadInput);
                }
                let raw = value[..4].try_into().map_err(|_| RelayError::Truncated)?;
                priority = Some(u32::from_be_bytes(raw));
            }
            STUN_ATTR_USE_CANDIDATE => {
                if len != 0 {
                    return Err(RelayError::BadInput);
                }
                use_candidate = true;
            }
            _ => {}
        }
        at += span;
    }
    if username != Some(creds.username) {
        return Err(RelayError::Unauthorized);
    }
    let priority = priority.ok_or(RelayError::BadInput)?;
    Ok(BindingCheck {
        tx,
        priority,
        use_candidate,
    })
}

/// Decode a signed binding response: verify the tag, then read the mapped
/// address from the prefix the tag covers.
pub fn decode_binding_response_signed(
    bytes: &[u8],
    tx: [u8; 12],
    creds: &StunCreds,
) -> Result<SocketAddr, RelayError> {
    let mi_at = verify_stun_integrity(creds.key(), bytes)?;
    let mut prefix = bytes[..mi_at].to_vec();
    let Ok(len) = u16::try_from(mi_at - 20) else {
        return Err(RelayError::TooLarge);
    };
    prefix[2..4].copy_from_slice(&len.to_be_bytes());
    decode_binding_response(&prefix, tx)
}

/// True when the bytes look like a STUN message: a binding type word or the
/// magic cookie. Relay frames start with `RL`, so the two never collide.
pub fn is_stun(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    let kind = u16::from_be_bytes([bytes[0], bytes[1]]);
    kind == STUN_BINDING_REQUEST
        || kind == STUN_BINDING_RESPONSE
        || u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == STUN_MAGIC_COOKIE
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
        if port == 0 {
            return Err(RelayError::BadInput); // port zero never gathers, so wire bytes carrying it are junk too
        }
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

/// Pair priority per RFC 8445 §6.1.2.3: 2^32*MIN(G,D) + 2*MAX(G,D) + (G>D),
/// with G the controlling side's candidate priority. Integer-only, no floats.
pub const fn pair_priority(controlling: u64, controlled: u64) -> u64 {
    let (min, max) = if controlling < controlled {
        (controlling, controlled)
    } else {
        (controlled, controlling)
    };
    (min << 32) + 2 * max + if controlling > controlled { 1 } else { 0 }
}

/// One candidate pair: the local and the remote side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CandidatePair {
    /// Local candidate.
    pub local: Candidate,
    /// Remote candidate.
    pub remote: Candidate,
}

impl CandidatePair {
    /// Pair priority with the local side controlling.
    pub fn priority(&self) -> u64 {
        pair_priority(self.local.priority(), self.remote.priority())
    }
}

/// Regular nomination (RFC 8445 §7.1.1): the highest-priority pair is tried
/// first and becomes the path only when its check succeeds with
/// USE-CANDIDATE. The first nomination is final; later checks never replace
/// it, so both ends settle on one pair.
pub struct Nomination {
    best: Option<CandidatePair>,
    nominated: Option<CandidatePair>,
}

impl Nomination {
    /// Nominate among `local` x `remote`; the highest pair priority wins and
    /// ties keep the first pair in list order, so the choice is stable.
    pub fn new(local: &[Candidate], remote: &[Candidate]) -> Self {
        let mut best: Option<CandidatePair> = None;
        for local in local {
            for remote in remote {
                let pair = CandidatePair {
                    local: *local,
                    remote: *remote,
                };
                if best.is_none_or(|held| pair.priority() > held.priority()) {
                    best = Some(pair);
                }
            }
        }
        Self {
            best,
            nominated: None,
        }
    }

    /// Highest-priority pair, the one a USE-CANDIDATE check asks for.
    pub fn best(&self) -> Option<CandidatePair> {
        self.best
    }

    /// Record one check result; true once a pair is nominated. Only a success
    /// on the best pair nominates, and only while nothing is nominated yet.
    pub fn on_check(&mut self, pair: CandidatePair, ok: bool) -> bool {
        if ok && self.nominated.is_none() && self.best == Some(pair) {
            self.nominated = Some(pair);
        }
        self.nominated.is_some()
    }

    /// The nominated pair: the only path game bytes may take.
    pub fn nominated(&self) -> Option<CandidatePair> {
        self.nominated
    }
}

/// Consent check interval in ticks: 5 s at 60 Hz (RFC 7675 §5).
pub const CONSENT_INTERVAL_TICKS: u32 = 300;
/// Consent lifetime in ticks: 30 s at 60 Hz (RFC 7675 §5).
pub const CONSENT_LIFETIME_TICKS: u32 = 1_800;

/// Consent freshness for one nominated pair: a keepalive is due every
/// [`CONSENT_INTERVAL_TICKS`], and the path is stale once
/// [`CONSENT_LIFETIME_TICKS`] pass with no authenticated answer. Tick counts
/// only, so the same answer sequence gives the same verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Consent {
    last_ok: u32,
    responses: u32,
}

impl Consent {
    /// Consent granted at `now` (the nomination tick) with no answer yet.
    pub fn new(now: u32) -> Self {
        Self {
            last_ok: now,
            responses: 0,
        }
    }

    /// True when a keepalive is due: the interval elapsed since the last ok.
    pub fn keepalive_due(&self, now: u32) -> bool {
        now.wrapping_sub(self.last_ok) >= CONSENT_INTERVAL_TICKS
    }

    /// True while the pair still holds consent; false once it is stale.
    pub fn fresh(&self, now: u32) -> bool {
        now.wrapping_sub(self.last_ok) < CONSENT_LIFETIME_TICKS
    }

    /// Record an authenticated answer; a stale pair never revives.
    pub fn on_response(&mut self, now: u32) {
        if self.fresh(now) {
            self.last_ok = now;
            self.responses = self.responses.saturating_add(1);
        }
    }

    /// Authenticated answers seen.
    pub fn responses(&self) -> u32 {
        self.responses
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{
        HolePunch, InviteCode, NatKind, RelayConfig, RelayHub, PUNCH_TIMEOUT_TICKS,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn socket(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), port)
    }

    /// RFC 8489 Appendix B.1 as corrected by verified erratum 6290: a
    /// long-term-authenticated binding request with MESSAGE-INTEGRITY-SHA256
    /// and PASSWORD-ALGORITHM SHA-256. The tag sits at offset 128.
    #[cfg(feature = "crypto")]
    const RFC8489_B1: [u8; 164] = [
        0x00, 0x01, 0x00, 0x90, 0x21, 0x12, 0xa4, 0x42, 0x78, 0xad, 0x34, 0x33, 0xc6, 0xad, 0x72,
        0xc0, 0x29, 0xda, 0x41, 0x2e, 0x00, 0x1e, 0x00, 0x20, 0x4a, 0x3c, 0xf3, 0x8f, 0xef, 0x69,
        0x92, 0xbd, 0xa9, 0x52, 0xc6, 0x78, 0x04, 0x17, 0xda, 0x0f, 0x24, 0x81, 0x94, 0x15, 0x56,
        0x9e, 0x60, 0xb2, 0x05, 0xc4, 0x6e, 0x41, 0x40, 0x7f, 0x17, 0x04, 0x00, 0x15, 0x00, 0x29,
        0x6f, 0x62, 0x4d, 0x61, 0x74, 0x4a, 0x6f, 0x73, 0x32, 0x41, 0x41, 0x41, 0x43, 0x66, 0x2f,
        0x2f, 0x34, 0x39, 0x39, 0x6b, 0x39, 0x35, 0x34, 0x64, 0x36, 0x4f, 0x4c, 0x33, 0x34, 0x6f,
        0x4c, 0x39, 0x46, 0x53, 0x54, 0x76, 0x79, 0x36, 0x34, 0x73, 0x41, 0x00, 0x00, 0x00, 0x00,
        0x14, 0x00, 0x0b, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x6f, 0x72, 0x67, 0x00,
        0x00, 0x1d, 0x00, 0x04, 0x00, 0x02, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x20, 0xb5, 0xc7, 0xbf,
        0x00, 0x5b, 0x6c, 0x52, 0xa2, 0x1c, 0x51, 0xc5, 0xe8, 0x92, 0xf8, 0x19, 0x24, 0x13, 0x62,
        0x96, 0xcb, 0x92, 0x7c, 0x43, 0x14, 0x93, 0x09, 0x27, 0x8c, 0xc6, 0x51, 0x8e, 0x65,
    ];

    #[cfg(feature = "crypto")]
    #[test]
    fn stun_message_integrity_matches_rfc8489_b1() {
        use sha2::{Digest, Sha256};
        // RFC 8489 §18.5.1.2: key = SHA-256(username ":" realm ":" password).
        let mut key = Sha256::new();
        key.update("\u{30DE}\u{30C8}\u{30EA}\u{30C3}\u{30AF}\u{30B9}".as_bytes());
        key.update(b":example.org:TheMatrIX");
        let key = key.finalize();
        assert_eq!(
            verify_stun_integrity(&key, &RFC8489_B1),
            Ok(128),
            "RFC 8489 B.1 MESSAGE-INTEGRITY-SHA256 verifies over the adjusted length"
        );
        let mut forged = RFC8489_B1;
        forged[133] ^= 0x01;
        assert_eq!(
            verify_stun_integrity(&key, &forged),
            Err(RelayError::Unauthorized),
            "one flipped tag byte refuses"
        );
        assert_eq!(
            verify_stun_integrity(b"wrong-key", &RFC8489_B1),
            Err(RelayError::Unauthorized),
            "a wrong key refuses the vector"
        );
    }

    #[test]
    fn signed_stun_bindings_refuse_every_spoof() {
        let mut state = 0x5EED_2040u64;
        let ticket =
            InviteTicket::issue(InviteCode::mint(&mut state), 3, 0x77, 10).expect("ticket");
        let creds = StunCreds::from_ticket(&ticket);
        assert_eq!(creds.username(), ticket.user, "ticket creds pin the user");
        assert!(matches!(
            StunCreds::new(0, [1u8; INVITE_BYTES]),
            Err(RelayError::Unauthorized)
        ));
        let tx = [0xA5u8; 12];
        let request = encode_binding_request_signed(tx, &creds, 0x6E00_01FF, true);
        assert_eq!(
            decode_binding_request_signed(&request, &creds),
            Ok(BindingCheck {
                tx,
                priority: 0x6E00_01FF,
                use_candidate: true
            }),
            "an honest ticket-signed request verifies"
        );
        let unsigned = {
            let request = encode_binding_request_signed(tx, &creds, 1, false);
            decode_binding_request_signed(&request, &creds)
        };
        assert!(
            unsigned.is_ok_and(|check| !check.use_candidate),
            "a request without USE-CANDIDATE names no nomination"
        );

        let mut other_state = 0x5EEF_2040u64;
        let other = StunCreds::from_ticket(
            &InviteTicket::issue(InviteCode::mint(&mut other_state), 3, 0x77, 10).expect("ticket"),
        );
        let mut flipped_tag = request.clone();
        let last = flipped_tag.len() - 1;
        flipped_tag[last] ^= 0x01;
        let mut flipped_tx = request.clone();
        flipped_tx[8] ^= 0x01;
        let mut flipped_user = request.clone();
        flipped_user[24] ^= 0x01;
        let mut flipped_type = request.clone();
        flipped_type[0] = 0x01;
        let truncated = request[..request.len() - 1].to_vec();
        let spoofs = [
            (request.clone(), other),
            (flipped_tag, creds),
            (flipped_tx, creds),
            (flipped_user, creds),
            (flipped_type, creds),
            (encode_binding_request(tx).to_vec(), creds),
            (truncated, creds),
        ];
        let refused = spoofs
            .iter()
            .filter(|(bytes, creds)| decode_binding_request_signed(bytes, creds).is_err())
            .count();
        assert_eq!(
            refused,
            spoofs.len(),
            "every spoofed binding is refused, 100%"
        );
    }

    #[test]
    fn relay_hub_requires_ticket_integrity_on_stun() {
        let mut state = 0x0A11_CE00_2040u64;
        let ticket = InviteTicket::issue(InviteCode::mint(&mut state), 3, 0x10, 0).expect("ticket");
        let creds = StunCreds::from_ticket(&ticket);
        let mut hub = RelayHub::new(RelayConfig::default()).with_creds(creds);
        let a = socket(7001);
        let tx = [9u8; 12];
        assert!(
            hub.handle(a, &encode_binding_request(tx)).is_empty(),
            "an unsigned binding is a spoof now"
        );
        assert_eq!(hub.stats().refused, 1);
        assert_eq!(hub.stats().stun, 0);
        let reply = hub.handle(a, &encode_binding_request_signed(tx, &creds, 1, false));
        assert_eq!(reply.len(), 1, "a ticket-signed binding answers");
        assert_eq!(
            decode_binding_response_signed(&reply[0].1, tx, &creds),
            Ok(a)
        );
        assert_eq!(hub.stats().stun, 1);
        let mut other_state = 0x0A11_CE00_2041u64;
        let other = StunCreds::from_ticket(
            &InviteTicket::issue(InviteCode::mint(&mut other_state), 3, 0x10, 0).expect("ticket"),
        );
        assert!(
            hub.handle(a, &encode_binding_request_signed(tx, &other, 1, false))
                .is_empty(),
            "another ticket never answers"
        );
        assert_eq!(hub.stats().refused, 2);
        // The standalone STUN server refuses the same way.
        let server = StunServer::new(socket(3478)).requiring(creds);
        assert!(server.serve(&encode_binding_request(tx)).is_err());
        let signed = server
            .serve(&encode_binding_request_signed(tx, &creds, 1, false))
            .expect("signed requests answer");
        assert_eq!(
            decode_binding_response_signed(&signed, tx, &creds),
            Ok(socket(3478))
        );
    }

    #[test]
    fn nomination_settles_on_the_best_pair() {
        let local =
            gather_candidates(socket(100), Some(socket(150)), Some(socket(200))).expect("gather");
        let remote = gather_candidates(socket(300), None, Some(socket(400))).expect("gather");
        let relay_pair = CandidatePair {
            local: local[2],
            remote: remote[1],
        };
        let mut nomination = Nomination::new(&local, &remote);
        let best = nomination.best().expect("pairs exist");
        assert_eq!(
            (best.local.kind, best.remote.kind),
            (CandidateKind::Host, CandidateKind::Host),
            "the host pair outranks every relayed pair"
        );
        assert_eq!(
            best.priority(),
            pair_priority(local[0].priority(), remote[0].priority())
        );
        assert!(best.priority() > relay_pair.priority());
        assert_eq!(
            pair_priority(126 << 24, 100 << 24),
            (100u64 << 56) + 2 * (126 << 24) + 1,
            "RFC 8445 pair priority: min shifted, max doubled, controlling breaks ties"
        );
        assert_eq!(
            pair_priority(100 << 24, 126 << 24),
            pair_priority(126 << 24, 100 << 24) - 1,
            "the controlling side breaks the tie"
        );
        assert!(
            !nomination.on_check(best, false),
            "a failed check nominates nothing"
        );
        assert_eq!(nomination.nominated(), None);
        assert!(
            nomination.on_check(best, true),
            "the best pair nominates on success"
        );
        assert_eq!(nomination.nominated(), Some(best));
        assert!(
            nomination.on_check(relay_pair, true),
            "a pair is already nominated"
        );
        assert_eq!(
            nomination.nominated(),
            Some(best),
            "nomination is final: a later relay check never replaces it"
        );
        assert_eq!(Nomination::new(&[], &remote).best(), None);
    }

    #[test]
    fn consent_keepalives_stay_fresh_and_the_punch_still_falls_back_at_300() {
        let mut consent = Consent::new(0);
        assert!(!consent.keepalive_due(CONSENT_INTERVAL_TICKS - 1));
        assert!(consent.keepalive_due(CONSENT_INTERVAL_TICKS));
        assert!(consent.fresh(CONSENT_LIFETIME_TICKS - 1));
        assert!(
            !consent.fresh(CONSENT_LIFETIME_TICKS),
            "30 s with no answer is stale"
        );
        consent.on_response(CONSENT_INTERVAL_TICKS);
        assert_eq!(consent.responses(), 1);
        assert!(
            consent.fresh(CONSENT_INTERVAL_TICKS * 2),
            "an answer renews consent"
        );
        let mut stale = Consent::new(0);
        stale.on_response(CONSENT_LIFETIME_TICKS + 1);
        assert_eq!(stale.responses(), 0, "a stale pair never revives");

        // Keepalives share the punch clock: a symmetric NAT still lands on
        // relay fallback at exactly PUNCH_TIMEOUT_TICKS.
        let mut punch = HolePunch::new(NatKind::Symmetric);
        punch.on_gathered();
        let mut consent = Consent::new(0);
        let mut keepalives = 0u32;
        for tick in 1..=PUNCH_TIMEOUT_TICKS {
            if consent.keepalive_due(tick) {
                keepalives += 1;
                consent.on_response(tick);
            }
            punch.tick(true);
            if tick < PUNCH_TIMEOUT_TICKS {
                assert!(!punch.needs_relay(), "fallback never moves earlier");
            }
        }
        assert_eq!(keepalives, 1, "one keepalive inside the punch budget");
        assert!(punch.needs_relay(), "fallback still wins at the deadline");
        assert_eq!(punch.attempts(), PUNCH_TIMEOUT_TICKS);
        assert_eq!(punch.attempts(), 300, "the budget is still 300 ticks");
    }
}
