//! lobby: LAN lobby — beacon browser, ready-up, AI-fill, typed messages.
//! Owner: net lane, consumer: the packaged game and the ui lobby screen.
//! Units: whole ms for pacing/browser, fixed ticks for match start.
//! Determinism: slot order plus ready transitions are pure input order;
//! ms timing never enters match seed or state. Errors: [`LobbyError`].
//! Budget: at most `MAX_PARTICIPANTS` slots, 16 beacons, wire messages at
//! most 33 bytes; browser and lobby ops are O(16) with no per-tick sim
//! allocation.
//! Caps: match slots come from `engine_input::MAX_PARTICIPANTS` (the
//! canonical cap) and beacon wire constants come from `dedicated` (the one
//! owner); this module declares no private cap and no mirror constants.
//! Crate edges `engine-net -> engine-input` and `lobby -> dedicated` are in
//! `tools/workspace-check.mjs`.

use std::net::SocketAddr;

pub use crate::dedicated::Beacon;
use crate::relay::InviteCode;
use engine_input::MAX_PARTICIPANTS;

/// Fewest fighters a lobby can form with.
pub const MIN_LOBBY_SLOTS: u8 = 2;
/// The canonical slot cap as an array length (the one widening of it).
const SLOT_CAP: usize = MAX_PARTICIPANTS as usize;
/// Browser capacity: LAN only, bounded by construction.
pub const MAX_BEACONS: usize = 16;
/// Browser refresh rate in hertz.
pub const REFRESH_HZ: u32 = 4;
/// Browser refresh period in whole milliseconds (4 Hz).
pub const REFRESH_PERIOD_MS: u64 = 250;
/// Beacon expiry in whole milliseconds (eight refresh periods).
pub const STALE_BEACON_MS: u64 = 2000;
/// Lobby wire magic `b"LB"` little-endian, first two bytes of control msgs.
pub const LOBBY_MAGIC: u16 = 0x424C;
/// Lobby wire version; anything else is refused. Version 2 widened the
/// `Start` AI mask from 8 to 16 bits so every `MAX_PARTICIPANTS` slot has
/// a bit; version 1 `Start` datagrams refuse as `BadVersion`, loudly.
pub const LOBBY_VERSION: u8 = 2;

/// Party board point in the ludus hub (`map.json` party_board interact
/// board x -13 z 1.2) in whole millimetres: integer-only so the gate never
/// takes a float into sim state. Party size is 3 tokens, no allocation.
const PARTY_SIZE: u8 = 3;
const PARTY_BOARD_X_MM: i32 = -13_000;
const PARTY_BOARD_Z_MM: i32 = 1_200;

/// Control-message kinds; typed, never strings.
pub const KIND_JOIN_REQUEST: u8 = 0;
/// Control-message kinds; typed, never strings.
pub const KIND_JOIN_GRANT: u8 = 1;
/// Control-message kinds; typed, never strings.
pub const KIND_JOIN_REFUSE: u8 = 2;
/// Control-message kinds; typed, never strings.
pub const KIND_READY: u8 = 3;
/// Control-message kinds; typed, never strings.
pub const KIND_START: u8 = 4;

/// Total wire lengths: header 4 plus fixed payload, no trailing bytes.
pub const JOIN_REQUEST_LEN: usize = 22;
/// Total wire lengths: header 4 plus fixed payload, no trailing bytes.
pub const JOIN_GRANT_LEN: usize = 24;
/// Total wire lengths: header 4 plus fixed payload, no trailing bytes.
pub const JOIN_REFUSE_LEN: usize = 33;
/// Total wire lengths: header 4 plus fixed payload, no trailing bytes.
pub const READY_LEN: usize = 13;
/// Total wire lengths: header 4 plus fixed payload, no trailing bytes.
/// `Start` carries the mask as two little-endian bytes (16-bit AI mask).
pub const START_LEN: usize = 25;

/// Lobby failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LobbyError {
    /// Any range, shape or state violation that is not named below.
    BadInput,
    /// Every slot is taken.
    SlotFull,
    /// Token holds no slot.
    UnknownToken {
        /// Token presented.
        token: u64,
    },
    /// Build mismatch: version plus rules digest, both pairs carried so the
    /// refusal names what differed. Loud by design, never silent.
    VersionMismatch {
        /// Host build version.
        want_version: u16,
        /// Peer build version.
        got_version: u16,
        /// Host rules digest.
        want_rules: u64,
        /// Peer rules digest.
        got_rules: u64,
    },
    /// Start requested while a human is unready or a slot is open.
    NotReady,
    /// Join or ready after the fight started.
    AlreadyStarted,
    /// Datagram ends mid-field or has the wrong fixed length.
    Truncated,
    /// Magic bytes do not match [`LOBBY_MAGIC`].
    BadMagic,
    /// Kind byte is not a [`KIND_*`] constant.
    BadKind(u8),
    /// Wire version byte is not [`LOBBY_VERSION`]; culprit carried.
    BadVersion {
        /// Version byte found on the wire.
        got: u8,
    },
}

impl std::fmt::Display for LobbyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadInput => write!(f, "invalid lobby input"),
            Self::SlotFull => write!(f, "no free lobby slot"),
            Self::UnknownToken { token } => write!(f, "unknown lobby token {token}"),
            Self::VersionMismatch {
                want_version,
                got_version,
                want_rules,
                got_rules,
            } => write!(
                f,
                "lobby version mismatch: want v{want_version} rules {want_rules:016x}, \
                 got v{got_version} rules {got_rules:016x}"
            ),
            Self::NotReady => write!(f, "lobby not ready"),
            Self::AlreadyStarted => write!(f, "fight already started"),
            Self::Truncated => write!(f, "truncated lobby packet"),
            Self::BadMagic => write!(f, "bad lobby magic"),
            Self::BadKind(k) => write!(f, "unknown lobby kind {k}"),
            Self::BadVersion { got } => {
                write!(f, "lobby version mismatch: got {got}, want {LOBBY_VERSION}")
            }
        }
    }
}

impl std::error::Error for LobbyError {}

/// Build identity: executable version plus rules-digest. Both must match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BuildId {
    /// Executable version, nonzero.
    pub version: u16,
    /// Rules-digest both sides computed over the same move tables.
    pub rules: u64,
}

impl BuildId {
    /// Range-check: version zero is reserved and fails.
    pub fn validate(self) -> Result<(), LobbyError> {
        if self.version == 0 {
            return Err(LobbyError::BadInput);
        }
        Ok(())
    }

    /// Loud match check carrying both digest pairs on mismatch.
    pub fn check_against(self, want: BuildId) -> Result<(), LobbyError> {
        if self.version != want.version || self.rules != want.rules {
            return Err(LobbyError::VersionMismatch {
                want_version: want.version,
                got_version: self.version,
                want_rules: want.rules,
                got_rules: self.rules,
            });
        }
        Ok(())
    }
}

// LAN discovery beacon: `Beacon` is owned by `dedicated` (canonical wire
// layout: magic 2, version 1, port 2, fighters 1, tick 4, hash 8; fixed
// 18 bytes, no padding, no trailing bytes) and re-exported at the top of
// this module.
// Payload validation inside this module maps its failure to `LobbyError`;
// the byte layout is identical, so either side decodes the other's datagrams.

/// Refusal reason: typed code, never a string.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefuseReason {
    /// Every slot is taken.
    SlotFull,
    /// Build version or rules digest differed.
    VersionMismatch,
    /// Token zero, bad slot echo or otherwise malformed request.
    BadInput,
}

impl RefuseReason {
    /// Wire code for this reason.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::SlotFull => 0,
            Self::VersionMismatch => 1,
            Self::BadInput => 2,
        }
    }

    /// Decode a wire code; unknown codes fail.
    pub const fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::SlotFull),
            1 => Some(Self::VersionMismatch),
            2 => Some(Self::BadInput),
            _ => None,
        }
    }
}

/// Typed lobby control messages: fixed little-endian structs, no strings
/// anywhere on the wire protocol paths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Msg {
    /// Client asks to join: token plus build identity.
    JoinRequest {
        /// Client token, nonzero.
        token: u64,
        /// Client build identity.
        build: BuildId,
    },
    /// Host grants a slot: token echo plus slot index.
    JoinGrant {
        /// Granted client token.
        token: u64,
        /// Slot index granted.
        slot: u8,
        /// Lobby fighter count.
        fighters: u8,
        /// Host build identity.
        build: BuildId,
    },
    /// Host refuses: typed reason plus both digest pairs when versioned.
    JoinRefuse {
        /// Refused client token.
        token: u64,
        /// Typed refusal reason.
        reason: RefuseReason,
        /// Host build identity.
        want: BuildId,
        /// Client build identity as seen by the host.
        got: BuildId,
    },
    /// Either side posts ready state for its token.
    Ready {
        /// Token posting readiness.
        token: u64,
        /// Ready flag.
        ready: bool,
    },
    /// Host starts the fight: every peer loads the same seed and slots.
    Start {
        /// Fighter count.
        fighters: u8,
        /// Match seed both sides load.
        seed: u64,
        /// Agreed build identity.
        build: BuildId,
        /// Bitmask of AI-filled slots (bit `i` set means slot `i` is AI).
        /// Sixteen bits cover every `MAX_PARTICIPANTS` slot; bits at or
        /// above `fighters` refuse.
        ai_mask: u16,
    },
}

impl Msg {
    /// Kind byte for this message.
    pub const fn kind(&self) -> u8 {
        match self {
            Self::JoinRequest { .. } => KIND_JOIN_REQUEST,
            Self::JoinGrant { .. } => KIND_JOIN_GRANT,
            Self::JoinRefuse { .. } => KIND_JOIN_REFUSE,
            Self::Ready { .. } => KIND_READY,
            Self::Start { .. } => KIND_START,
        }
    }

    /// Fixed total wire length for this message.
    pub const fn wire_len(&self) -> usize {
        match self {
            Self::JoinRequest { .. } => JOIN_REQUEST_LEN,
            Self::JoinGrant { .. } => JOIN_GRANT_LEN,
            Self::JoinRefuse { .. } => JOIN_REFUSE_LEN,
            Self::Ready { .. } => READY_LEN,
            Self::Start { .. } => START_LEN,
        }
    }

    /// Encode one message: magic, version, kind, then fixed fields.
    pub fn encode(self) -> Result<Vec<u8>, LobbyError> {
        let mut out = Vec::with_capacity(self.wire_len());
        out.extend_from_slice(&LOBBY_MAGIC.to_le_bytes());
        out.push(LOBBY_VERSION);
        out.push(self.kind());
        match self {
            Self::JoinRequest { token, build } => {
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                out.extend_from_slice(&token.to_le_bytes());
                out.extend_from_slice(&build.version.to_le_bytes());
                out.extend_from_slice(&build.rules.to_le_bytes());
            }
            Self::JoinGrant {
                token,
                slot,
                fighters,
                build,
            } => {
                if token == 0
                    || slot >= MAX_PARTICIPANTS
                    || !(MIN_LOBBY_SLOTS..=MAX_PARTICIPANTS).contains(&fighters)
                    || slot >= fighters
                {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                out.extend_from_slice(&token.to_le_bytes());
                out.push(slot);
                out.push(fighters);
                out.extend_from_slice(&build.version.to_le_bytes());
                out.extend_from_slice(&build.rules.to_le_bytes());
            }
            Self::JoinRefuse {
                token,
                reason,
                want,
                got,
            } => {
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                out.extend_from_slice(&token.to_le_bytes());
                out.push(reason.as_u8());
                out.extend_from_slice(&want.version.to_le_bytes());
                out.extend_from_slice(&got.version.to_le_bytes());
                out.extend_from_slice(&want.rules.to_le_bytes());
                out.extend_from_slice(&got.rules.to_le_bytes());
            }
            Self::Ready { token, ready } => {
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                out.extend_from_slice(&token.to_le_bytes());
                out.push(u8::from(ready));
            }
            Self::Start {
                fighters,
                seed,
                build,
                ai_mask,
            } => {
                if !(MIN_LOBBY_SLOTS..=MAX_PARTICIPANTS).contains(&fighters) {
                    return Err(LobbyError::BadInput);
                }
                // Bits at or above the fighter count refuse; the 16-bit mask
                // covers every canonical slot including slots 8..15.
                if u32::from(ai_mask) >> fighters != 0 {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                out.push(fighters);
                out.extend_from_slice(&seed.to_le_bytes());
                out.extend_from_slice(&build.version.to_le_bytes());
                out.extend_from_slice(&build.rules.to_le_bytes());
                out.extend_from_slice(&ai_mask.to_le_bytes());
            }
        }
        debug_assert_eq!(out.len(), self.wire_len());
        Ok(out)
    }

    /// Decode and fully validate one message; wrong magic, version, kind,
    /// length or field ranges reject loudly. No trailing bytes accepted.
    pub fn decode(bytes: &[u8]) -> Result<Self, LobbyError> {
        if bytes.len() < 4 {
            return Err(LobbyError::Truncated);
        }
        if u16::from_le_bytes([bytes[0], bytes[1]]) != LOBBY_MAGIC {
            return Err(LobbyError::BadMagic);
        }
        if bytes[2] != LOBBY_VERSION {
            return Err(LobbyError::BadVersion { got: bytes[2] });
        }
        match bytes[3] {
            KIND_JOIN_REQUEST => {
                if bytes.len() != JOIN_REQUEST_LEN {
                    return Err(LobbyError::Truncated);
                }
                let token = u64::from_le_bytes(bytes_range_8(bytes, 4)?);
                let build = BuildId {
                    version: u16::from_le_bytes([bytes[12], bytes[13]]),
                    rules: u64::from_le_bytes(bytes_range_8(bytes, 14)?),
                };
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                Ok(Self::JoinRequest { token, build })
            }
            KIND_JOIN_GRANT => {
                if bytes.len() != JOIN_GRANT_LEN {
                    return Err(LobbyError::Truncated);
                }
                let token = u64::from_le_bytes(bytes_range_8(bytes, 4)?);
                let slot = bytes[12];
                let fighters = bytes[13];
                let build = BuildId {
                    version: u16::from_le_bytes([bytes[14], bytes[15]]),
                    rules: u64::from_le_bytes(bytes_range_8(bytes, 16)?),
                };
                if token == 0
                    || slot >= MAX_PARTICIPANTS
                    || !(MIN_LOBBY_SLOTS..=MAX_PARTICIPANTS).contains(&fighters)
                    || slot >= fighters
                {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                Ok(Self::JoinGrant {
                    token,
                    slot,
                    fighters,
                    build,
                })
            }
            KIND_JOIN_REFUSE => {
                if bytes.len() != JOIN_REFUSE_LEN {
                    return Err(LobbyError::Truncated);
                }
                let token = u64::from_le_bytes(bytes_range_8(bytes, 4)?);
                let reason = RefuseReason::from_u8(bytes[12]).ok_or(LobbyError::BadInput)?;
                let want = BuildId {
                    version: u16::from_le_bytes([bytes[13], bytes[14]]),
                    rules: u64::from_le_bytes(bytes_range_8(bytes, 17)?),
                };
                let got = BuildId {
                    version: u16::from_le_bytes([bytes[15], bytes[16]]),
                    rules: u64::from_le_bytes(bytes_range_8(bytes, 25)?),
                };
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                Ok(Self::JoinRefuse {
                    token,
                    reason,
                    want,
                    got,
                })
            }
            KIND_READY => {
                if bytes.len() != READY_LEN {
                    return Err(LobbyError::Truncated);
                }
                let token = u64::from_le_bytes(bytes_range_8(bytes, 4)?);
                if token == 0 {
                    return Err(LobbyError::BadInput);
                }
                let ready = match bytes[12] {
                    0 => false,
                    1 => true,
                    _ => return Err(LobbyError::BadInput),
                };
                Ok(Self::Ready { token, ready })
            }
            KIND_START => {
                if bytes.len() != START_LEN {
                    return Err(LobbyError::Truncated);
                }
                let fighters = bytes[4];
                let seed = u64::from_le_bytes(bytes_range_8(bytes, 5)?);
                let build = BuildId {
                    version: u16::from_le_bytes([bytes[13], bytes[14]]),
                    rules: u64::from_le_bytes(bytes_range_8(bytes, 15)?),
                };
                let ai_mask = u16::from_le_bytes([bytes[23], bytes[24]]);
                if !(MIN_LOBBY_SLOTS..=MAX_PARTICIPANTS).contains(&fighters) {
                    return Err(LobbyError::BadInput);
                }
                if u32::from(ai_mask) >> fighters != 0 {
                    return Err(LobbyError::BadInput);
                }
                build.validate()?;
                Ok(Self::Start {
                    fighters,
                    seed,
                    build,
                    ai_mask,
                })
            }
            kind => Err(LobbyError::BadKind(kind)),
        }
    }
}

/// Copy 8 bytes at `at`; the caller guarantees lengths, this only guards
/// slicing so decode never panics on hostile input.
fn bytes_range_8(bytes: &[u8], at: usize) -> Result<[u8; 8], LobbyError> {
    let slice = bytes.get(at..at + 8).ok_or(LobbyError::Truncated)?;
    let mut out = [0u8; 8];
    out.copy_from_slice(slice);
    Ok(out)
}

/// True when a 4 Hz refresh is due: `now_ms` is at least one period past
/// `last_ms`. Wrapping-subtract survives the u64 wrap; units are whole ms.
pub const fn refresh_due(last_ms: u64, now_ms: u64) -> bool {
    now_ms.wrapping_sub(last_ms) >= REFRESH_PERIOD_MS
}

/// One lobby slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// Beyond the lobby fighter count; never assigned.
    Closed,
    /// Waiting for a human join; blocks start until filled or AI-filled.
    Open,
    /// Human holder plus ready flag.
    Human {
        /// Holder token.
        token: u64,
        /// Ready flag.
        ready: bool,
    },
    /// AI-filled ordinary fighter slot.
    Ai,
}

/// Ready-up phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Joins, leaves, ready posts and AI fill accepted.
    Forming,
    /// Start issued; joins and ready posts refuse.
    Starting,
}

/// Match start both sides load: seed, slots and AI mask.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StartParams {
    /// Fighter count.
    pub fighters: u8,
    /// Match seed both sides load.
    pub seed: u64,
    /// Agreed build identity.
    pub build: BuildId,
    /// Bitmask of AI-filled slots: bit `i` set means slot `i` is AI.
    pub ai_mask: u16,
}

impl StartParams {
    /// Render as the typed [`Msg::Start`] wire message.
    pub const fn as_msg(self) -> Msg {
        Msg::Start {
            fighters: self.fighters,
            seed: self.seed,
            build: self.build,
            ai_mask: self.ai_mask,
        }
    }
}

/// Party form-up at the board plus queue-into-one-match over the MP3 lobby.
/// Reuses [`BuildId`] version-digest checks and [`InviteCode`] minting; never
/// mirrors either. Determinism: member order is invite order, queue order is
/// slot order; board check is integer mm, never float. Errors: [`LobbyError`].
struct PartyForm {
    build: BuildId,
    invite: InviteCode,
    members: [u64; 3],
    len: u8,
}

impl PartyForm {
    fn new(build: BuildId, invite: InviteCode) -> Result<Self, LobbyError> {
        build.validate()?;
        Ok(Self {
            build,
            invite,
            members: [0; 3],
            len: 0,
        })
    }

    fn invite(&mut self, token: u64) -> Result<u8, LobbyError> {
        if token == 0 {
            return Err(LobbyError::BadInput);
        }
        if let Some(at) = self.members[..usize::from(self.len)]
            .iter()
            .position(|held| *held == token)
        {
            return Ok(u8::try_from(at).map_err(|_| LobbyError::SlotFull)?);
        }
        if self.len >= PARTY_SIZE {
            return Err(LobbyError::SlotFull);
        }
        self.members[usize::from(self.len)] = token;
        self.len += 1;
        Ok(self.len - 1)
    }

    fn board_ok(x_mm: i32, z_mm: i32) -> bool {
        x_mm == PARTY_BOARD_X_MM && z_mm == PARTY_BOARD_Z_MM
    }

    fn queue(
        &self,
        fighters: u8,
        seed: u64,
        x_mm: i32,
        z_mm: i32,
    ) -> Result<StartParams, LobbyError> {
        if !Self::board_ok(x_mm, z_mm) {
            return Err(LobbyError::BadInput);
        }
        if self.invite.as_bytes() == [0; 6] {
            return Err(LobbyError::BadInput);
        }
        if self.len != PARTY_SIZE {
            return Err(LobbyError::NotReady);
        }
        if !(PARTY_SIZE..=MAX_PARTICIPANTS).contains(&fighters) {
            return Err(LobbyError::BadInput);
        }
        let mut lobby = Lobby::new(self.build, fighters, seed)?;
        for token in self.members.iter().copied() {
            lobby.join(token, self.build)?;
        }
        for token in self.members.iter().copied() {
            lobby.set_ready(token, true)?;
        }
        lobby.ai_fill();
        lobby.try_start()
    }
}

/// Host-owned ready-up state machine: sticky human slots, AI fill, loud
/// version plus rules-digest gate. Slot order is join order over the
/// lowest open index, so the same join sequence always assigns the same
/// slots; ms timing never enters the outcome.
pub struct Lobby {
    build: BuildId,
    fighters: u8,
    seed: u64,
    slots: [Slot; SLOT_CAP],
    phase: Phase,
}

impl Lobby {
    /// Open a lobby: `fighters` in `2..=MAX_PARTICIPANTS`, nonzero build
    /// version. Slots below `fighters` start open, the rest closed.
    pub fn new(build: BuildId, fighters: u8, seed: u64) -> Result<Self, LobbyError> {
        build.validate()?;
        if !(MIN_LOBBY_SLOTS..=MAX_PARTICIPANTS).contains(&fighters) {
            return Err(LobbyError::BadInput);
        }
        debug_assert!(
            u32::from(PARTY_SIZE) >= u32::from(MIN_LOBBY_SLOTS),
            "party of 3 always fits the lobby floor"
        );
        let mut slots = [Slot::Closed; SLOT_CAP];
        for slot in slots.iter_mut().take(usize::from(fighters)) {
            *slot = Slot::Open;
        }
        Ok(Self {
            build,
            fighters,
            seed,
            slots,
            phase: Phase::Forming,
        })
    }

    /// Host build identity.
    pub const fn build(&self) -> BuildId {
        self.build
    }

    /// Fighter count.
    pub const fn fighters(&self) -> u8 {
        self.fighters
    }

    /// Match seed issued at start.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Current phase.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Slot states in index order.
    pub const fn slots(&self) -> &[Slot; SLOT_CAP] {
        &self.slots
    }

    /// Slot held by `token`, if any.
    pub fn slot_of(&self, token: u64) -> Option<u8> {
        self.slots
            .iter()
            .position(|slot| match slot {
                Slot::Human { token: held, .. } => *held == token,
                _ => false,
            })
            .map(|index| index as u8)
    }

    /// Join `token` with its build: version plus rules digest must match
    /// the host build or the refusal names both pairs. Idempotent for a
    /// token that already holds a slot. Lowest open slot wins.
    pub fn join(&mut self, token: u64, build: BuildId) -> Result<u8, LobbyError> {
        build.check_against(self.build)?;
        if self.phase != Phase::Forming {
            return Err(LobbyError::AlreadyStarted);
        }
        if token == 0 {
            return Err(LobbyError::BadInput);
        }
        if let Some(slot) = self.slot_of(token) {
            return Ok(slot);
        }
        let open = self.slots[..usize::from(self.fighters)]
            .iter()
            .position(|slot| *slot == Slot::Open)
            .ok_or(LobbyError::SlotFull)?;
        self.slots[open] = Slot::Human {
            token,
            ready: false,
        };
        Ok(open as u8)
    }

    /// Leave: a human slot reopens, AI slots stay AI. Returns true when a
    /// human slot was released.
    pub fn leave(&mut self, token: u64) -> bool {
        if self.phase != Phase::Forming {
            return false;
        }
        if let Some(index) = self.slot_of(token) {
            self.slots[usize::from(index)] = Slot::Open;
            return true;
        }
        false
    }

    /// Post ready state for a human token.
    pub fn set_ready(&mut self, token: u64, ready: bool) -> Result<(), LobbyError> {
        if self.phase != Phase::Forming {
            return Err(LobbyError::AlreadyStarted);
        }
        let index = self
            .slot_of(token)
            .ok_or(LobbyError::UnknownToken { token })?;
        match &mut self.slots[usize::from(index)] {
            Slot::Human { ready: flag, .. } => {
                *flag = ready;
                Ok(())
            }
            _ => Err(LobbyError::BadInput),
        }
    }

    /// AI-fill every open slot as an ordinary fighter slot, lowest index
    /// first. Returns the count filled.
    pub fn ai_fill(&mut self) -> u8 {
        if self.phase != Phase::Forming {
            return 0;
        }
        let mut filled = 0u8;
        for slot in self.slots[..usize::from(self.fighters)].iter_mut() {
            if *slot == Slot::Open {
                *slot = Slot::Ai;
                filled += 1;
            }
        }
        filled
    }

    /// Bitmask of AI slots: bit `i` set means slot `i` is AI. Sixteen bits
    /// so every `MAX_PARTICIPANTS` slot has a bit; `u16` shifts never trap
    /// on slots 8..15.
    pub fn ai_mask(&self) -> u16 {
        let mut mask = 0u16;
        for (index, slot) in self.slots[..usize::from(self.fighters)].iter().enumerate() {
            if *slot == Slot::Ai {
                mask |= 1u16 << index;
            }
        }
        mask
    }

    /// Human-occupied slot count.
    pub fn human_count(&self) -> usize {
        self.slots[..usize::from(self.fighters)]
            .iter()
            .filter(|slot| matches!(slot, Slot::Human { .. }))
            .count()
    }

    /// AI-occupied slot count.
    pub fn ai_count(&self) -> usize {
        self.slots[..usize::from(self.fighters)]
            .iter()
            .filter(|slot| **slot == Slot::Ai)
            .count()
    }

    /// True when every playable slot is filled and every human is ready:
    /// no open slots, at least one human, all humans ready. AI counts as
    /// ready. Closed slots never block.
    pub fn all_ready(&self) -> bool {
        let playable = &self.slots[..usize::from(self.fighters)];
        let mut humans = 0usize;
        for slot in playable {
            match slot {
                Slot::Open | Slot::Closed => return false,
                Slot::Human { ready: false, .. } => return false,
                Slot::Human { ready: true, .. } => humans += 1,
                Slot::Ai => {}
            }
        }
        humans > 0
    }

    /// Start the fight when [`Self::all_ready`] holds: moves to
    /// [`Phase::Starting`] and returns the params both sides load. A second
    /// call refuses; a premature call reports not-ready and changes nothing.
    pub fn try_start(&mut self) -> Result<StartParams, LobbyError> {
        if self.phase != Phase::Forming {
            return Err(LobbyError::AlreadyStarted);
        }
        if !self.all_ready() {
            return Err(LobbyError::NotReady);
        }
        self.phase = Phase::Starting;
        Ok(StartParams {
            fighters: self.fighters,
            seed: self.seed,
            build: self.build,
            ai_mask: self.ai_mask(),
        })
    }
}

/// One browser sighting: beacon plus sender plus last-seen ms.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeenBeacon {
    /// Beacon payload.
    pub beacon: Beacon,
    /// Sender address the join reply goes to; learned from the beacon
    /// datagram, never typed by the operator.
    pub from: SocketAddr,
    /// Last sighting in whole ms.
    pub last_seen_ms: u64,
}

/// Beacon browser: bounded LAN list with 4 Hz refresh pacing. The client
/// learns host addresses from beacon datagrams, so two machines form a
/// fight with no manual addresses.
pub struct Browser {
    entries: [Option<SeenBeacon>; MAX_BEACONS],
    len: usize,
    last_refresh_ms: Option<u64>,
}

impl Browser {
    /// Empty browser; first refresh is immediately due.
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_BEACONS],
            len: 0,
            last_refresh_ms: None,
        }
    }

    /// Sightings held.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Last refresh instant in whole ms, if refreshed.
    pub const fn last_refresh_ms(&self) -> Option<u64> {
        self.last_refresh_ms
    }

    /// True when a 4 Hz refresh is due at `now_ms`.
    pub const fn poll_due(&self, now_ms: u64) -> bool {
        match self.last_refresh_ms {
            None => true,
            Some(last) => refresh_due(last, now_ms),
        }
    }

    /// Record a refresh at `now_ms`; answers true when it was due (and
    /// records), false when pacing refused it and nothing changed.
    pub fn mark_refreshed(&mut self, now_ms: u64) -> bool {
        if self.poll_due(now_ms) {
            self.last_refresh_ms = Some(now_ms);
            true
        } else {
            false
        }
    }

    /// Observe one beacon sighting: validated, deduped by sender address
    /// with the newest tick winning, evicting the stalest entry when full.
    pub fn observe(
        &mut self,
        beacon: Beacon,
        from: SocketAddr,
        now_ms: u64,
    ) -> Result<(), LobbyError> {
        beacon.validate().map_err(|_| LobbyError::BadInput)?;
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|seen| seen.from == from)
        {
            if beacon.tick >= entry.beacon.tick {
                entry.beacon = beacon;
            }
            entry.last_seen_ms = now_ms;
            return Ok(());
        }
        let sighting = SeenBeacon {
            beacon,
            from,
            last_seen_ms: now_ms,
        };
        if self.len < MAX_BEACONS {
            let free = self.entries.iter().position(|slot| slot.is_none());
            if let Some(index) = free {
                self.entries[index] = Some(sighting);
                self.len += 1;
                return Ok(());
            }
        }
        // Full: replace the stalest entry.
        let mut oldest = 0usize;
        let mut oldest_ms = u64::MAX;
        for (index, slot) in self.entries.iter().enumerate() {
            if let Some(seen) = slot {
                if seen.last_seen_ms < oldest_ms {
                    oldest_ms = seen.last_seen_ms;
                    oldest = index;
                }
            }
        }
        self.entries[oldest] = Some(sighting);
        Ok(())
    }

    /// Drop sightings older than `stale_ms` (whole ms).
    pub fn prune(&mut self, now_ms: u64, stale_ms: u64) {
        for slot in self.entries.iter_mut() {
            if let Some(seen) = slot {
                if now_ms.wrapping_sub(seen.last_seen_ms) > stale_ms {
                    *slot = None;
                    self.len -= 1;
                }
            }
        }
    }

    /// Sightings in slot order ( Nones skipped).
    pub fn sightings(&self) -> Vec<SeenBeacon> {
        self.entries.iter().filter_map(|slot| *slot).collect()
    }

    /// Freshest sighting by beacon tick, if any.
    pub fn best(&self) -> Option<SeenBeacon> {
        self.entries
            .iter()
            .filter_map(|slot| *slot)
            .max_by_key(|seen| seen.beacon.tick)
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;

    use crate::dedicated::{ServerError, BEACON_LEN, BEACON_MAGIC, BEACON_VERSION};

    const BUILD: BuildId = BuildId {
        version: 7,
        rules: 0x0123_4567_89AB_CDEF,
    };

    fn beacon(port: u16) -> Beacon {
        Beacon {
            port,
            fighters: 2,
            tick: 9,
            hash: 0xABCD,
        }
    }

    fn drain(sock: &UdpSocket) -> Vec<(Vec<u8>, SocketAddr)> {
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            out.push((buf[..n].to_vec(), from));
        }
        out
    }

    /// Drain until a datagram arrives or the deadline passes: loopback
    /// delivery is fast but not instant, and under full-workspace load a
    /// single drain races the sender (same settle discipline as the
    /// provider socket suite). Test-only.
    fn drain_settle(sock: &UdpSocket) -> Vec<(Vec<u8>, SocketAddr)> {
        // Bounded poll loop (400 x 5 ms matches the old 2 s ceiling) with no
        // wall clock: loopback delivery is fast but not instant, and under
        // full-workspace load a single drain races the sender.
        for _ in 0..400 {
            let got = drain(sock);
            if !got.is_empty() {
                return got;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        drain(sock)
    }

    #[test]
    fn sixteen_slot_lobby_forms_and_seventeen_refuses() {
        // 16-slot validation: the canonical cap forms, cap + 1 refuses.
        let full = Lobby::new(BUILD, MAX_PARTICIPANTS, 7).expect("16-slot lobby");
        assert_eq!(full.fighters(), MAX_PARTICIPANTS);
        assert_eq!(full.slots().len(), MAX_PARTICIPANTS as usize);
        assert_eq!(
            Lobby::new(BUILD, MAX_PARTICIPANTS + 1, 7).map(|_| ()),
            Err(LobbyError::BadInput)
        );
        assert_eq!(
            Lobby::new(BUILD, 1, 7).map(|_| ()),
            Err(LobbyError::BadInput)
        );
        assert_eq!(
            Beacon {
                port: 4242,
                fighters: MAX_PARTICIPANTS,
                tick: 0,
                hash: 0,
            }
            .validate(),
            Ok(())
        );
        assert_eq!(
            Beacon {
                port: 4242,
                fighters: MAX_PARTICIPANTS + 1,
                tick: 0,
                hash: 0,
            }
            .validate(),
            Err(ServerError::BadInput)
        );
    }

    #[test]
    fn sixteen_slot_ai_mask_covers_every_slot() {
        // The widened 16-bit mask names AI slots 8..15 that a `u8`
        // mask could never carry (debug builds trapped on `1 << 8`).
        let mut lobby = Lobby::new(BUILD, MAX_PARTICIPANTS, 7).expect("16-slot lobby");
        lobby.join(1, BUILD).expect("join slot 0");
        lobby.join(2, BUILD).expect("join slot 1");
        lobby.set_ready(1, true).expect("ready");
        lobby.set_ready(2, true).expect("ready");
        assert_eq!(lobby.ai_fill(), MAX_PARTICIPANTS - 2);
        let mask = lobby.ai_mask();
        assert_eq!(mask, 0xFFFFu16 ^ 0b11);
        assert_ne!(mask & 0xFF00, 0, "slots 8..15 need the upper byte");
        let start = lobby.try_start().expect("start");
        assert_eq!(start.ai_mask, mask);
        let bytes = start.as_msg().encode().expect("encode");
        assert_eq!(bytes.len(), START_LEN);
        assert_eq!(Msg::decode(&bytes), Ok(start.as_msg()));
        // Mask bits at or above the fighter count refuse on both paths.
        let bad = Msg::Start {
            fighters: 15,
            seed: 1,
            build: BUILD,
            ai_mask: 1u16 << 15,
        };
        assert_eq!(bad.encode(), Err(LobbyError::BadInput));
        // A version-1 Start (pre-widening wire) refuses loudly as a version
        // mismatch, never as silent truncation.
        let mut v1 = bytes.clone();
        v1[2] = LOBBY_VERSION - 1;
        assert_eq!(
            Msg::decode(&v1),
            Err(LobbyError::BadVersion {
                got: LOBBY_VERSION - 1
            })
        );
        // The 16-slot beacon crosses real loopback UDP and decodes.
        let tx = UdpSocket::bind("127.0.0.1:0").expect("tx");
        let rx = UdpSocket::bind("127.0.0.1:0").expect("rx");
        rx.set_nonblocking(true).expect("nonblocking");
        let target = rx.local_addr().expect("rx addr");
        let body = Beacon {
            port: tx.local_addr().expect("tx addr").port(),
            fighters: MAX_PARTICIPANTS,
            tick: 3,
            hash: 0xBEEF,
        };
        body.announce(&tx, target).expect("announce");
        let mut seen = None;
        for _ in 0..200 {
            if let Some(got) = Beacon::listen_once(&rx).expect("listen") {
                seen = Some(got);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(seen.map(|(beacon, _)| beacon), Some(body));
    }

    #[test]
    fn beacon_uses_canonical_mp1_layout() {
        let body = beacon(4242);
        let bytes = body.encode().expect("encode");
        assert_eq!(bytes.len(), BEACON_LEN);
        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), BEACON_MAGIC);
        assert_eq!(bytes[2], BEACON_VERSION);
        assert_eq!(Beacon::decode(&bytes), Ok(body));
        // One owner, one wire layout: the dedicated authority encodes
        // byte-identical datagrams, so either side decodes the other's.
        let canonical = crate::dedicated::Beacon {
            port: 4242,
            fighters: 2,
            tick: 9,
            hash: 0xABCD,
        }
        .encode()
        .expect("canonical encode");
        assert_eq!(bytes, canonical);
        // Wrong magic, future version, truncation and bad fields reject
        // through the single owner as one loud bad-input error.
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
        assert_eq!(
            Beacon { port: 0, ..body }.encode(),
            Err(ServerError::BadInput)
        );
    }

    #[test]
    fn typed_messages_roundtrip_fixed_sizes() {
        let request = Msg::JoinRequest {
            token: 11,
            build: BUILD,
        };
        let grant = Msg::JoinGrant {
            token: 11,
            slot: 1,
            fighters: 4,
            build: BUILD,
        };
        let refuse = Msg::JoinRefuse {
            token: 11,
            reason: RefuseReason::VersionMismatch,
            want: BUILD,
            got: BuildId {
                version: 8,
                rules: 1,
            },
        };
        let ready = Msg::Ready {
            token: 11,
            ready: true,
        };
        let start = Msg::Start {
            fighters: 4,
            seed: 99,
            build: BUILD,
            ai_mask: 0b1100,
        };
        for msg in [request, grant, refuse, ready, start] {
            let bytes = msg.encode().expect("encode");
            assert_eq!(bytes.len(), msg.wire_len());
            assert_eq!(Msg::decode(&bytes), Ok(msg));
        }
        assert_eq!(request.wire_len(), JOIN_REQUEST_LEN);
        assert_eq!(grant.wire_len(), JOIN_GRANT_LEN);
        assert_eq!(refuse.wire_len(), JOIN_REFUSE_LEN);
        assert_eq!(ready.wire_len(), READY_LEN);
        assert_eq!(start.wire_len(), START_LEN);
        // No strings on the wire: every message has one fixed length and a
        // trailing byte refuses.
        let bytes = request.encode().expect("encode");
        let mut trailed = bytes.clone();
        trailed.push(0);
        assert_eq!(Msg::decode(&trailed), Err(LobbyError::Truncated));
        assert_eq!(
            Msg::decode(&bytes[..bytes.len() - 1]),
            Err(LobbyError::Truncated)
        );
        // Future version, bad magic, unknown kind, bad token and bad ready.
        let mut future = bytes.clone();
        future[2] = LOBBY_VERSION + 1;
        assert_eq!(
            Msg::decode(&future),
            Err(LobbyError::BadVersion {
                got: LOBBY_VERSION + 1
            })
        );
        let mut bad_magic = bytes.clone();
        bad_magic[0] ^= 0xFF;
        assert_eq!(Msg::decode(&bad_magic), Err(LobbyError::BadMagic));
        let mut bad_kind = bytes.clone();
        bad_kind[3] = 0xFF;
        assert_eq!(Msg::decode(&bad_kind), Err(LobbyError::BadKind(0xFF)));
        let zero_token = Msg::Ready {
            token: 0,
            ready: true,
        };
        assert_eq!(zero_token.encode(), Err(LobbyError::BadInput));
        let mut bad_ready = ready.encode().expect("encode");
        bad_ready[READY_LEN - 1] = 7;
        assert_eq!(Msg::decode(&bad_ready), Err(LobbyError::BadInput));
        let bad_reason = Msg::JoinRefuse {
            token: 5,
            reason: RefuseReason::SlotFull,
            want: BUILD,
            got: BUILD,
        }
        .encode()
        .expect("encode");
        let mut tampered = bad_reason;
        tampered[12] = 9;
        assert_eq!(Msg::decode(&tampered), Err(LobbyError::BadInput));
        // Start mask bits beyond the fighter count refuse.
        let bad_mask = Msg::Start {
            fighters: 2,
            seed: 1,
            build: BUILD,
            ai_mask: 0b0100,
        };
        assert_eq!(bad_mask.encode(), Err(LobbyError::BadInput));
    }

    #[test]
    fn version_rules_mismatch_refused_loudly_with_digest_pair() {
        let mut lobby = Lobby::new(BUILD, 2, 99).expect("lobby");
        let bad_version = BuildId {
            version: 8,
            rules: BUILD.rules,
        };
        let bad_rules = BuildId {
            version: BUILD.version,
            rules: BUILD.rules ^ 1,
        };
        for bad in [bad_version, bad_rules] {
            let err = lobby.join(11, bad).expect_err("must refuse");
            assert_eq!(
                err,
                LobbyError::VersionMismatch {
                    want_version: BUILD.version,
                    got_version: bad.version,
                    want_rules: BUILD.rules,
                    got_rules: bad.rules,
                }
            );
            // The refusal names both digest pairs in hex.
            let text = std::format!("{err}");
            assert!(text.contains("mismatch"));
            assert!(std::format!("{:016x}", BUILD.rules).len() == 16);
            assert!(text.contains(&std::format!("{:016x}", bad.rules)));
        }
        assert_eq!(lobby.human_count(), 0);
        // Zero versions never open or join.
        assert_eq!(
            Lobby::new(
                BuildId {
                    version: 0,
                    rules: 1
                },
                2,
                1
            )
            .map(|_| ()),
            Err(LobbyError::BadInput)
        );
    }

    #[test]
    fn refresh_pacing_holds_4hz() {
        assert_eq!(REFRESH_HZ, 4);
        assert_eq!(REFRESH_PERIOD_MS, 250);
        assert!(!refresh_due(1000, 1000));
        assert!(!refresh_due(1000, 1249));
        assert!(refresh_due(1000, 1250));
        assert!(refresh_due(1000, 1500));
        let mut browser = Browser::new();
        assert!(browser.poll_due(0));
        assert!(browser.mark_refreshed(0));
        assert!(!browser.poll_due(249));
        assert!(!browser.mark_refreshed(249));
        assert!(browser.poll_due(250));
        assert!(browser.mark_refreshed(250));
        assert_eq!(browser.last_refresh_ms(), Some(250));
    }

    #[test]
    fn browser_dedups_prunes_and_caps() {
        let mut browser = Browser::new();
        let a: SocketAddr = "127.0.0.1:4001".parse().expect("addr");
        let b: SocketAddr = "127.0.0.1:4002".parse().expect("addr");
        browser.observe(beacon(4001), a, 0).expect("observe");
        assert_eq!(browser.len(), 1);
        // Same sender refreshes in place, newest tick wins.
        let newer = Beacon {
            tick: 30,
            ..beacon(4001)
        };
        browser.observe(newer, a, 100).expect("observe");
        assert_eq!(browser.len(), 1);
        assert_eq!(browser.best().expect("best").beacon.tick, 30);
        // Stale tick from the same sender keeps the newer payload.
        let older = Beacon {
            tick: 3,
            ..beacon(4001)
        };
        browser.observe(older, a, 150).expect("observe");
        assert_eq!(browser.best().expect("best").beacon.tick, 30);
        browser.observe(beacon(4002), b, 100).expect("observe");
        assert_eq!(browser.len(), 2);
        // Stale entries prune by whole-ms age (a refreshed at 150, b at 100).
        browser.prune(150 + STALE_BEACON_MS + 1, STALE_BEACON_MS);
        assert!(browser.is_empty());
        // Junk beacons never enter the list.
        assert_eq!(
            browser.observe(
                Beacon {
                    port: 0,
                    ..beacon(1)
                },
                a,
                0
            ),
            Err(LobbyError::BadInput)
        );
        assert!(browser.is_empty());
    }

    #[test]
    fn ready_up_ai_fill_and_start() {
        let mut lobby = Lobby::new(BUILD, 4, 4242).expect("lobby");
        assert_eq!(lobby.join(11, BUILD), Ok(0));
        assert_eq!(lobby.join(22, BUILD), Ok(1));
        // Duplicate join is idempotent, wrong builds still refuse loudly.
        assert_eq!(lobby.join(11, BUILD), Ok(0));
        assert_eq!(lobby.slot_of(22), Some(1));
        assert_eq!(lobby.try_start(), Err(LobbyError::NotReady));
        lobby.set_ready(11, true).expect("ready");
        assert_eq!(lobby.try_start(), Err(LobbyError::NotReady));
        // AI fills the two open slots as ordinary fighters, low index first.
        assert_eq!(lobby.ai_fill(), 2);
        assert_eq!(lobby.ai_mask(), 0b1100);
        assert_eq!((lobby.human_count(), lobby.ai_count()), (2, 2));
        assert_eq!(lobby.try_start(), Err(LobbyError::NotReady));
        lobby.set_ready(22, true).expect("ready");
        assert!(lobby.all_ready());
        let start = lobby.try_start().expect("start");
        assert_eq!(
            start,
            StartParams {
                fighters: 4,
                seed: 4242,
                build: BUILD,
                ai_mask: 0b1100,
            }
        );
        assert_eq!(lobby.phase(), Phase::Starting);
        assert_eq!(start.as_msg().encode().expect("encode").len(), START_LEN);
        // After start, joins and ready posts refuse and the mask roundtrips.
        assert_eq!(lobby.join(33, BUILD), Err(LobbyError::AlreadyStarted));
        assert_eq!(lobby.set_ready(11, true), Err(LobbyError::AlreadyStarted));
        assert_eq!(lobby.try_start(), Err(LobbyError::AlreadyStarted));
        let decoded = Msg::decode(&start.as_msg().encode().expect("encode")).expect("decode");
        assert_eq!(decoded, start.as_msg());
        // Unknown tokens and full lobbies fail without moving state.
        let mut pair = Lobby::new(BUILD, 2, 1).expect("lobby");
        pair.join(1, BUILD).expect("join");
        pair.join(2, BUILD).expect("join");
        assert_eq!(pair.join(3, BUILD), Err(LobbyError::SlotFull));
        assert_eq!(
            pair.set_ready(9, true),
            Err(LobbyError::UnknownToken { token: 9 })
        );
        assert!(!pair.leave(9));
        assert!(pair.leave(1));
        assert_eq!(pair.join(3, BUILD), Ok(0));
    }

    #[test]
    fn party_of_three_queues_into_one_match() {
        let mut seed_state = 0x0A11_CE00_1234_5678;
        let invite = InviteCode::mint(&mut seed_state);
        let mut party = PartyForm::new(BUILD, invite).expect("party");
        assert_eq!(party.invite(11), Ok(0));
        assert_eq!(party.invite(22), Ok(1));
        assert_eq!(party.invite(11), Ok(0));
        assert_eq!(party.invite(33), Ok(2));
        let start = party
            .queue(4, 4242, PARTY_BOARD_X_MM, PARTY_BOARD_Z_MM)
            .expect("queue");
        assert_eq!(start.fighters, 4);
        assert_eq!(start.seed, 4242);
        assert_eq!(start.build, BUILD);
        assert_eq!(start.ai_mask, 0b1000);
        assert_eq!(
            Msg::decode(&start.as_msg().encode().expect("encode")),
            Ok(start.as_msg())
        );
    }

    #[test]
    fn party_queue_refuses_bad_board_fourth_and_mismatch() {
        let mut seed_state = 0x0A11_CE00_1234_5678;
        let invite = InviteCode::mint(&mut seed_state);
        let mut party = PartyForm::new(BUILD, invite).expect("party");
        assert_eq!(party.invite(0), Err(LobbyError::BadInput));
        party.invite(11).expect("join");
        party.invite(22).expect("join");
        assert_eq!(
            party.queue(4, 1, 0, 0),
            Err(LobbyError::BadInput),
            "wrong board refuses"
        );
        assert_eq!(
            party.queue(4, 1, PARTY_BOARD_X_MM, PARTY_BOARD_Z_MM),
            Err(LobbyError::NotReady),
            "two of three cannot queue"
        );
        party.invite(33).expect("join");
        assert_eq!(party.invite(44), Err(LobbyError::SlotFull));
        assert_eq!(
            party
                .queue(4, 1, PARTY_BOARD_X_MM, PARTY_BOARD_Z_MM)
                .map(|_| ()),
            Ok(()),
            "three of three queues"
        );
        let bad = BuildId {
            version: BUILD.version,
            rules: BUILD.rules ^ 1,
        };
        let mut lobby = Lobby::new(BUILD, 3, 9).expect("lobby");
        assert_eq!(
            lobby.join(77, bad),
            Err(LobbyError::VersionMismatch {
                want_version: BUILD.version,
                got_version: bad.version,
                want_rules: BUILD.rules,
                got_rules: bad.rules,
            })
        );
    }

    #[test]
    fn loopback_two_peers_form_fight_without_typed_addresses() {
        const HOST_TOKEN: u64 = 0xB0517;
        const CLIENT_TOKEN: u64 = 0xC11E47;
        let host_sock = UdpSocket::bind("127.0.0.1:0").expect("host socket");
        let client_sock = UdpSocket::bind("127.0.0.1:0").expect("client socket");
        host_sock.set_nonblocking(true).expect("nonblocking");
        client_sock.set_nonblocking(true).expect("nonblocking");
        let host_port = host_sock.local_addr().expect("host addr").port();
        let client_addr = client_sock.local_addr().expect("client addr");

        // Host owns a 4-slot lobby and beacons it; the beacon datagram
        // carries the host port, so the client never types an address.
        let mut lobby = Lobby::new(BUILD, 4, 777).expect("lobby");
        assert_eq!(lobby.join(HOST_TOKEN, BUILD), Ok(0));
        let beacon_body = Beacon {
            port: host_port,
            fighters: 4,
            tick: 0,
            hash: 0,
        };
        beacon_body
            .announce(&host_sock, client_addr)
            .expect("announce");

        // Client browsers the beacon and learns the host address from it.
        let (seen, _) = Beacon::listen_once(&client_sock)
            .expect("listen")
            .expect("beacon arrives");
        assert_eq!(seen, beacon_body);
        let mut browser = Browser::new();
        let host_addr: SocketAddr = std::format!("127.0.0.1:{}", seen.port)
            .parse()
            .expect("host addr");
        assert_eq!(host_addr.port(), host_port);
        browser.observe(seen, host_addr, 0).expect("observe");
        assert_eq!(browser.best().expect("best").beacon.port, host_port);

        // Typed join exchange over loopback.
        let request = Msg::JoinRequest {
            token: CLIENT_TOKEN,
            build: BUILD,
        };
        client_sock
            .send_to(&request.encode().expect("encode"), host_addr)
            .expect("send join");
        let mut got_request = None;
        for (bytes, from) in drain_settle(&host_sock) {
            if let Ok(Msg::JoinRequest { token, build }) = Msg::decode(&bytes) {
                got_request = Some((token, build, from));
            }
        }
        let (token, build, from) = got_request.expect("host sees join");
        let slot = lobby.join(token, build).expect("grant");
        assert_eq!((token, slot), (CLIENT_TOKEN, 1));
        let grant = Msg::JoinGrant {
            token,
            slot,
            fighters: 4,
            build: BUILD,
        };
        host_sock
            .send_to(&grant.encode().expect("encode"), from)
            .expect("send grant");
        let mut got_slot = None;
        for (bytes, _) in drain_settle(&client_sock) {
            if let Ok(Msg::JoinGrant { slot, .. }) = Msg::decode(&bytes) {
                got_slot = Some(slot);
            }
        }
        assert_eq!(got_slot, Some(1));

        // Ready-up both humans, AI-fill the rest, start with one typed
        // Start both sides load identically.
        lobby.set_ready(HOST_TOKEN, true).expect("host ready");
        let ready = Msg::Ready {
            token: CLIENT_TOKEN,
            ready: true,
        };
        client_sock
            .send_to(&ready.encode().expect("encode"), host_addr)
            .expect("send ready");
        for (bytes, _) in drain(&host_sock) {
            if let Ok(Msg::Ready { token, ready }) = Msg::decode(&bytes) {
                lobby.set_ready(token, ready).expect("client ready");
            }
        }
        assert_eq!(lobby.ai_fill(), 2);
        let start = lobby.try_start().expect("start");
        assert_eq!(start.ai_mask, 0b1100);
        host_sock
            .send_to(&start.as_msg().encode().expect("encode"), from)
            .expect("send start");
        let mut got_start = None;
        for (bytes, _) in drain(&client_sock) {
            if let Ok(msg @ Msg::Start { .. }) = Msg::decode(&bytes) {
                got_start = Some(msg);
            }
        }
        assert_eq!(got_start, Some(start.as_msg()));
        // Formation receipt: two humans plus two AI in four slots.
        assert_eq!((lobby.human_count(), lobby.ai_count()), (2, 2));
        assert_eq!(lobby.phase(), Phase::Starting);
    }
}
