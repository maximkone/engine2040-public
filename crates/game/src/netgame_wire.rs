//! netgame_wire: the invitation code and the game-level datagrams of a
//! [`NetGame`](super::NetGame) session (hello, welcome, coverage ack, pong),
//! plus the socket and tape-window helpers the session uses.
//!
//! Consumer: `netgame` (re-exported there) and `net_seat`. Pure except
//! [`open_socket`]; integer only; no hash reads any of it. Errors:
//! [`GameError`], every malformed code or datagram refuses loudly.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// A raw UDP session socket: the real one natively; a browser tab has no raw
/// sockets, so the wasm32 build carries a stub whose calls refuse. Callers
/// keep one name on both targets.
#[cfg(not(target_arch = "wasm32"))]
pub(super) use std::net::UdpSocket as SessionSocket;

/// The wasm32 socket stub: it only has to type-check; every call refuses.
#[cfg(target_arch = "wasm32")]
pub(super) struct SessionSocket;

#[cfg(target_arch = "wasm32")]
impl SessionSocket {
    /// Always refuses: a browser tab has no raw sockets.
    pub(super) fn local_addr(&self) -> std::io::Result<SocketAddr> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    /// Always refuses: a browser tab has no raw sockets.
    pub(super) fn send_to(&self, _bytes: &[u8], _to: SocketAddr) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::Unsupported.into())
    }

    /// Always refuses: a browser tab has no raw sockets.
    pub(super) fn recv_from(&self, _buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

use engine_input::FighterInput;
use engine_net::session::{NetError, WIRE_MAGIC, WIRE_VERSION};

use crate::{GameConfig, GameError};

/// Session-code prefix: invitation codes are `E2040NET-v<ver>-<seed>-<ticks>-<fighters>-<extent>`.
pub const SESSION_CODE_PREFIX: &str = "E2040NET";
/// Guest to host: the join hello, `[header][identity u64][match code]`.
pub const KIND_HELLO: u8 = 0x10;
/// Host to guest: the answer to a hello, same layout with the host's identity
/// and code. Equal identities pair the session; a guest that reads another
/// identity refuses with [`GameError::SpecMismatch`].
pub const KIND_WELCOME: u8 = 0x11;
/// Lockstep guest to host: `[header][need u32][stamp u32]`, "I hold every
/// authoritative tick below `need`". Heartbeat, coverage ack and hole report
/// in one; the host answers with a [`KIND_PONG`] and, for a hole, re-sent windows.
pub const KIND_NEED: u8 = 0x12;
/// Host to lockstep guest: `[header][stamp u32]`, the echo of a need's stamp
/// (the guest's round-trip clock).
pub const KIND_PONG: u8 = 0x13;
/// Longest match code a hello or welcome carries.
pub const MAX_CODE_BYTES: usize = 160;
/// Header bytes of every datagram: magic 2, version 1, kind 1.
pub(super) const HEADER_BYTES: usize = 4;

/// The rules a session plays. The invitation code names them, so both peers
/// build one match; the hello's identity check catches any other drift.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum NetRules {
    /// The frozen v1 open-sand match (the LAN proofs).
    V1,
    /// The window's duel: rules v2 on the kit walls.
    Duel,
    /// The window's duel with the spear grapple on.
    DuelSpear,
}

impl NetRules {
    /// The duel rules, with the spear when `spear`.
    #[must_use]
    pub const fn duel(spear: bool) -> Self {
        if spear {
            Self::DuelSpear
        } else {
            Self::Duel
        }
    }

    /// Short name for logs: `v1`, `duel` or `duel+spear`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::Duel => "duel",
            Self::DuelSpear => "duel+spear",
        }
    }
}

/// A decoded invitation: the match, its rules and, for a session through the
/// self-hosted relay, the host's relay peer id. The text form is
/// `E2040NET-v<ver>-<seed>-<ticks>-<fighters>-<extent>[-duel[-spear]][-r<id>]`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Invite {
    /// The match config both peers build.
    pub config: GameConfig,
    /// The rules both peers build.
    pub rules: NetRules,
    /// The host's peer id on the relay; `None` for a direct (LAN) session.
    pub relay_host: Option<u64>,
}

impl Invite {
    /// The match part of the code (no relay id): what a hello carries.
    #[must_use]
    pub fn match_code(&self) -> String {
        let config = &self.config;
        let base = format!(
            "{SESSION_CODE_PREFIX}-v{WIRE_VERSION}-{:016x}-{}-{}-{:08x}",
            config.seed,
            config.ticks,
            config.fighters,
            config.half_extent_m.to_bits(),
        );
        match self.rules {
            NetRules::V1 => base,
            NetRules::Duel => format!("{base}-{SESSION_CODE_DUEL}"),
            NetRules::DuelSpear => format!("{base}-{SESSION_CODE_DUEL}-{SESSION_CODE_SPEAR}"),
        }
    }

    /// The whole code a guest types: the match code plus the relay id when
    /// the host plays through a relay.
    #[must_use]
    pub fn code(&self) -> String {
        match self.relay_host {
            Some(id) => format!("{}-r{id:016x}", self.match_code()),
            None => self.match_code(),
        }
    }

    /// Parse a code. A version field other than [`WIRE_VERSION`] fails as
    /// [`NetError::VersionMismatch`]; anything unparseable, out of range or
    /// with an unknown tail fails as [`GameError::BadConfig`]. Pure.
    pub fn decode(code: &str) -> Result<Self, GameError> {
        let bad = GameError::BadConfig("session_code");
        let parts: Vec<&str> = code.trim().split('-').collect();
        if parts.len() < 6 || parts[0] != SESSION_CODE_PREFIX {
            return Err(bad);
        }
        let version: u8 = parts[1]
            .strip_prefix('v')
            .and_then(|digits| digits.parse().ok())
            .ok_or(bad)?;
        if version != WIRE_VERSION {
            return Err(GameError::Net(NetError::VersionMismatch { got: version }));
        }
        let seed = u64::from_str_radix(parts[2], 16).map_err(|_| bad)?;
        let ticks = parts[3].parse::<u32>().map_err(|_| bad)?;
        let fighters = parts[4].parse::<u8>().map_err(|_| bad)?;
        let extent_bits = u32::from_str_radix(parts[5], 16).map_err(|_| bad)?;
        let (rules, tail) = match &parts[6..] {
            [SESSION_CODE_DUEL, SESSION_CODE_SPEAR, tail @ ..] => (NetRules::DuelSpear, tail),
            [SESSION_CODE_DUEL, tail @ ..] => (NetRules::Duel, tail),
            tail => (NetRules::V1, tail),
        };
        let relay_host = match tail {
            [] => None,
            [id] => {
                let hex = id.strip_prefix('r').filter(|hex| hex.len() == 16);
                let id = hex.and_then(|hex| u64::from_str_radix(hex, 16).ok());
                Some(id.filter(|id| *id != 0).ok_or(bad)?)
            }
            _ => return Err(bad),
        };
        let config = GameConfig {
            seed,
            ticks,
            fighters,
            half_extent_m: f32::from_bits(extent_bits),
        };
        config.validate()?;
        Ok(Self {
            config,
            rules,
            relay_host,
        })
    }
}

/// Encode the invitation code binding the match config plus the wire version:
/// `E2040NET-v<ver>-<seed:016x>-<ticks>-<fighters>-<extent_bits:08x>`. Pure.
#[must_use]
pub fn encode_session_code(config: &GameConfig) -> String {
    Invite {
        config: *config,
        rules: NetRules::V1,
        relay_host: None,
    }
    .code()
}

/// Suffix of an invitation code that names the duel match (rules v2 on the
/// kit walls, the match the window plays): `<v1 code>-duel`.
pub const SESSION_CODE_DUEL: &str = "duel";
/// Suffix after [`SESSION_CODE_DUEL`] that turns the spear grapple on:
/// `<v1 code>-duel-spear`.
pub const SESSION_CODE_SPEAR: &str = "spear";

/// [`encode_session_code`] for the duel match: the same fields plus the
/// [`SESSION_CODE_DUEL`] suffix, so the guest builds the host's spec. Pure.
#[must_use]
pub fn encode_duel_session_code(config: &GameConfig) -> String {
    Invite {
        config: *config,
        rules: NetRules::Duel,
        relay_host: None,
    }
    .code()
}

/// Decode an invitation code back into the match config. A version field other
/// than [`WIRE_VERSION`] fails as [`NetError::VersionMismatch`]; anything
/// unparseable or out of range fails as [`GameError::BadConfig`]. Pure. A duel
/// code decodes to its config too; [`decode_session_mode`] also names the mode.
pub fn decode_session_code(code: &str) -> Result<GameConfig, GameError> {
    Invite::decode(code).map(|invite| invite.config)
}

/// [`decode_session_code`] plus the match mode: `true` for a duel code (with
/// or without the spear), `false` for the plain v1 code. Pure.
pub fn decode_session_mode(code: &str) -> Result<(GameConfig, bool), GameError> {
    Invite::decode(code).map(|invite| (invite.config, invite.rules != NetRules::V1))
}

/// Peek the packet kind after checking magic plus version, so role dispatch
/// refuses version mismatches before any kind-specific parsing.
pub(super) fn peek_kind(bytes: &[u8]) -> Result<u8, GameError> {
    if bytes.len() < HEADER_BYTES {
        return Err(GameError::Net(NetError::Truncated));
    }
    if u16::from_le_bytes([bytes[0], bytes[1]]) != WIRE_MAGIC {
        return Err(GameError::Net(NetError::BadMagic));
    }
    if bytes[2] != WIRE_VERSION {
        return Err(GameError::Net(NetError::VersionMismatch { got: bytes[2] }));
    }
    Ok(bytes[3])
}

/// The header every game-level datagram starts with.
pub(super) fn header(kind: u8, body: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES + body);
    out.extend_from_slice(&WIRE_MAGIC.to_le_bytes());
    out.push(WIRE_VERSION);
    out.push(kind);
    out
}

/// A hello or a welcome: the match identity plus the match code (cut at
/// [`MAX_CODE_BYTES`]; codes are ASCII).
#[must_use]
pub fn encode_greeting(kind: u8, identity: u64, code: &str) -> Vec<u8> {
    let code = &code.as_bytes()[..code.len().min(MAX_CODE_BYTES)];
    let mut out = header(kind, 8 + code.len());
    out.extend_from_slice(&identity.to_le_bytes());
    out.extend_from_slice(code);
    out
}

/// Read a hello or a welcome (after [`peek_kind`]): identity and code. A short
/// body, an oversize body or a code that is not printable ASCII refuses.
pub fn decode_greeting(bytes: &[u8]) -> Result<(u64, String), GameError> {
    let body = bytes
        .get(HEADER_BYTES..)
        .ok_or(GameError::Net(NetError::Truncated))?;
    if body.len() < 8 {
        return Err(GameError::Net(NetError::Truncated));
    }
    if body.len() > 8 + MAX_CODE_BYTES {
        return Err(GameError::Net(NetError::TooLarge));
    }
    let (id, code) = body.split_at(8);
    let mut word = [0u8; 8];
    word.copy_from_slice(id);
    if !code.iter().all(u8::is_ascii_graphic) {
        return Err(GameError::BadConfig("session_code"));
    }
    let code =
        String::from_utf8(code.to_vec()).map_err(|_| GameError::BadConfig("session_code"))?;
    Ok((u64::from_le_bytes(word), code))
}

/// A lockstep guest's coverage ack: `need` plus the round-trip `stamp`.
#[must_use]
pub fn encode_need(need: u32, stamp: u32) -> Vec<u8> {
    let mut out = header(KIND_NEED, 8);
    out.extend_from_slice(&need.to_le_bytes());
    out.extend_from_slice(&stamp.to_le_bytes());
    out
}

/// Exactly `N` body bytes after the header, or a loud refusal.
pub(super) fn body_words<const N: usize>(bytes: &[u8]) -> Result<[u8; N], GameError> {
    let body = bytes
        .get(HEADER_BYTES..)
        .ok_or(GameError::Net(NetError::Truncated))?;
    <[u8; N]>::try_from(body).map_err(|_| GameError::Net(NetError::Truncated))
}

/// One little-endian `u32` of a fixed body.
pub(super) fn word_at(body: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&body[at..at + 4]);
    u32::from_le_bytes(word)
}

/// The ticks `end - (cover - 1) ..= end` of a `(tick, fighter)`-ordered tape,
/// as a slice: the state window the host repeats, found by binary search so a
/// long match pays the same per tick as a short one.
pub(super) fn tape_window(tape: &[FighterInput], end: u32, cover: usize) -> &[FighterInput] {
    let back = u32::try_from(cover.saturating_sub(1)).unwrap_or(u32::MAX);
    let floor = end.saturating_sub(back);
    let from = tape.partition_point(|input| input.tick < floor);
    let to = tape.partition_point(|input| input.tick <= end);
    &tape[from..to.max(from)]
}

/// The unspecified address of `target`'s family on port `port`: what a peer
/// binds to reach `target`. A loopback target binds loopback, so a test or a
/// one-PC run never opens a port to the network.
#[must_use]
pub fn bind_for(target: SocketAddr, port: u16) -> SocketAddr {
    let ip = match (target.ip(), target.ip().is_loopback()) {
        (IpAddr::V4(_), true) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        (IpAddr::V4(_), false) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        (IpAddr::V6(_), true) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        (IpAddr::V6(_), false) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    SocketAddr::new(ip, port)
}

/// A nonblocking UDP socket bound at `at`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn open_socket(at: SocketAddr) -> Result<SessionSocket, GameError> {
    let sock = SessionSocket::bind(at).map_err(|err| GameError::Io(err.kind()))?;
    sock.set_nonblocking(true)
        .map_err(|err| GameError::Io(err.kind()))?;
    Ok(sock)
}

/// A browser tab has no raw sockets: the wasm32 build refuses loudly.
#[cfg(target_arch = "wasm32")]
pub(super) fn open_socket(_at: SocketAddr) -> Result<SessionSocket, GameError> {
    Err(GameError::Io(std::io::ErrorKind::Unsupported))
}
