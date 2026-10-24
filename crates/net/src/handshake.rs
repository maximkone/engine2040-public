//! handshake: the session connect handshake, modeled on the netcode 1.02
//! protocol spec's *shape* (the spec is reference only; no donor bytes).
//!
//! Steps, integer-only and deterministic (no wall clock: the caller passes
//! `now`):
//!
//! 1. a [`TokenGenerator`](crate::handshake::TokenGenerator) mints a [`ConnectToken`](crate::handshake::ConnectToken) (protocol id, timestamps,
//!    a session nonce, the two directional keys) and [`seal_token`](crate::handshake::seal_token)s it;
//! 2. the client sends a connection request carrying the sealed token;
//! 3. the server verifies the seal, refuses a token that is expired, not yet
//!    valid, minted for another protocol id or replayed, and answers a
//!    challenge sealed under the token's server-to-client key;
//! 4. the client answers a response sealed under a key derived from the
//!    token's client-to-server key and the challenge nonce; the server
//!    verifies it. Both ends then hold the same [`SessionKeys`](crate::handshake::SessionKeys).
//!
//! Authentication: with the `crypto` feature on, every seal is HMAC-SHA256
//! (`hmac` + `sha2`, never hand-rolled) and a wrong key or a tampered byte
//! refuses as [`HandshakeError::BadToken`](crate::handshake::HandshakeError::BadToken). With `crypto` off the seal carries
//! a documented 8-byte FNV fold instead: deterministic and integer-only, but
//! NOT authentication, and the sealed token is a development-only plain shape.
//! The feature is off by default, so the wasm bundle `engine-web` builds does
//! not grow. This layer runs at join time, never inside the fixed tick.

use std::collections::BTreeSet;

/// Protocol id of an engine2040 arena join: the ASCII text `e2040` read as a
/// big-endian `u64`, so a log shows it as text.
pub const PROTOCOL_ID: u64 = 0x6532_3034_3000_0000;
/// Version byte of the plaintext connect-token shape.
pub const CONNECT_TOKEN_VERSION: u8 = 1;
/// Version byte of the request/challenge/response messages.
pub const HANDSHAKE_VERSION: u8 = 1;
/// Session key length in bytes.
pub const KEY_LEN: usize = 32;
/// Nonce length in bytes.
pub const NONCE_LEN: usize = 24;
/// Most server addresses one connect token carries.
pub const MAX_SERVERS: usize = 16;
/// Most private-data bytes one connect token carries.
pub const MAX_PRIVATE_DATA: usize = 1024;
/// Most sessions one server holds between a request and its response.
pub const MAX_PENDING: usize = 64;

/// A 32-byte directional session key.
pub type SessionKey = [u8; KEY_LEN];
/// A 24-byte nonce (session nonce or seal nonce).
pub type Nonce = [u8; NONCE_LEN];

/// Key-derivation labels, so the two directions cannot derive the same key.
const LABEL_C2S: &[u8] = b"e2040-c2s";
/// Key-derivation label of the server-to-client direction.
const LABEL_S2C: &[u8] = b"e2040-s2c";

/// Seal backend with the `crypto` feature: HMAC-SHA256 over `nonce || message`
/// under the seal key, and an HMAC-based key derivation. Never hand-rolled.
#[cfg(feature = "crypto")]
mod auth {
    use super::{HandshakeError, Nonce, SessionKey, KEY_LEN};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    /// Tag length in bytes: one SHA-256 block of HMAC output.
    pub(super) const TAG_LEN: usize = 32;

    /// HMAC-SHA256 of `nonce || message` under `key`.
    pub(super) fn tag(
        key: &[u8],
        nonce: &Nonce,
        message: &[u8],
    ) -> Result<Vec<u8>, HandshakeError> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| HandshakeError::BadToken)?;
        mac.update(nonce);
        mac.update(message);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    /// Constant-time check of `tag` under `key`; a key the backend rejects
    /// verifies nothing (fails closed).
    pub(super) fn verify(key: &[u8], nonce: &Nonce, message: &[u8], tag: &[u8]) -> bool {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
            return false;
        };
        mac.update(nonce);
        mac.update(message);
        mac.verify_slice(tag).is_ok()
    }

    /// HMAC-SHA256 key derivation: the token key is the HMAC key, the label
    /// and the challenge nonce are the message, and 32 bytes come back.
    pub(super) fn derive(
        token_key: &SessionKey,
        label: &[u8],
        challenge: &Nonce,
    ) -> Result<SessionKey, HandshakeError> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(token_key).map_err(|_| HandshakeError::BadToken)?;
        mac.update(label);
        mac.update(challenge);
        let mut out = [0u8; KEY_LEN];
        out.copy_from_slice(mac.finalize().into_bytes().as_slice());
        Ok(out)
    }
}

/// Seal backend without `crypto`: an 8-byte FNV-1a fold of
/// `nonce || message || key`. Deterministic and integer-only, but NOT
/// authentication: it is a development shape, and a deployment that must
/// refuse a forged token turns the `crypto` feature on.
#[cfg(not(feature = "crypto"))]
mod auth {
    use super::{HandshakeError, Nonce, SessionKey, KEY_LEN};
    use engine_core::Fnv64;

    /// Tag length in bytes: one folded `u64`.
    pub(super) const TAG_LEN: usize = 8;

    /// FNV-1a 64 over `nonce || message || key`, little-endian.
    fn fold(key: &[u8], nonce: &Nonce, message: &[u8]) -> [u8; TAG_LEN] {
        let mut hasher = Fnv64::new();
        hasher.write(nonce);
        hasher.write(message);
        hasher.write(key);
        hasher.finish().to_le_bytes()
    }

    /// The development-only fold as a tag.
    pub(super) fn tag(
        key: &[u8],
        nonce: &Nonce,
        message: &[u8],
    ) -> Result<Vec<u8>, HandshakeError> {
        Ok(fold(key, nonce, message).to_vec())
    }

    /// Compare the fold; equal bytes pass.
    pub(super) fn verify(key: &[u8], nonce: &Nonce, message: &[u8], tag: &[u8]) -> bool {
        tag == fold(key, nonce, message).as_slice()
    }

    /// Development-only key derivation: four folds, one per 8-byte lane, over
    /// `label || challenge || lane`. Not a KDF; see the module docs.
    pub(super) fn derive(
        token_key: &SessionKey,
        label: &[u8],
        challenge: &Nonce,
    ) -> Result<SessionKey, HandshakeError> {
        let mut out = [0u8; KEY_LEN];
        for (lane, chunk) in out.chunks_mut(8).enumerate() {
            let mut hasher = Fnv64::new();
            hasher.write(label);
            hasher.write(challenge);
            hasher.write(token_key);
            let lane = u64::try_from(lane).map_err(|_| HandshakeError::BadToken)?;
            hasher.write_u64(lane);
            chunk.copy_from_slice(&hasher.finish().to_le_bytes());
        }
        Ok(out)
    }
}

/// Tag length in bytes this build writes: 32 with `crypto` (HMAC-SHA256), 8
/// without it (the development-only fold). A token sealed by one build does
/// not parse in the other.
pub const TAG_LEN: usize = auth::TAG_LEN;

/// Handshake refusals: typed, bounded and loud. A refusal never changes state
/// and never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HandshakeError {
    /// A seal did not verify: a wrong key or a tampered byte. A verifier
    /// cannot tell those apart, and must not.
    BadToken,
    /// The token's protocol id is not this server's, its nonce was already
    /// used, or a response names an unknown or mismatched session.
    WrongSession,
    /// The token expired: `now` is at or past `expire`.
    Expired {
        /// Caller's time when the token was presented.
        now: i64,
        /// The token's expire timestamp.
        expire: i64,
    },
    /// The token is not valid yet: `now` is before `create`.
    NotYetValid {
        /// Caller's time when the token was presented.
        now: i64,
        /// The token's create timestamp.
        create: i64,
    },
    /// A datagram ends mid-field, carries a bad version byte or an
    /// impossible length.
    Malformed,
    /// A count exceeds its bound ([`MAX_SERVERS`], [`MAX_PRIVATE_DATA`],
    /// [`MAX_PENDING`]).
    TooLong,
    /// A message arrived out of order for the handshake state.
    OutOfOrder,
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadToken => write!(f, "handshake seal did not verify"),
            Self::WrongSession => write!(f, "token or response is for another session"),
            Self::Expired { now, expire } => {
                write!(f, "connect token expired at {expire}, presented at {now}")
            }
            Self::NotYetValid { now, create } => {
                write!(f, "connect token valid from {create}, presented at {now}")
            }
            Self::Malformed => write!(f, "malformed handshake message"),
            Self::TooLong => write!(f, "handshake count exceeds its bound"),
            Self::OutOfOrder => write!(f, "handshake message out of order"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// One server address a connect token offers: four IPv4 bytes and a port.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ServerAddress {
    /// IPv4 address bytes, network order.
    pub ip: [u8; 4],
    /// UDP port, host order.
    pub port: u16,
}

/// The connect token a session is opened from: what the token generator mints
/// and the server validates. The two directional keys travel inside it, so
/// both ends reach the same session keys once the token is authenticated.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConnectToken {
    /// Protocol id the token was minted for.
    pub protocol_id: u64,
    /// Unix seconds the token was minted at.
    pub create_timestamp: i64,
    /// Unix seconds at or after which the server refuses the token.
    pub expire_timestamp: i64,
    /// Session nonce, unique per token; a second use refuses.
    pub nonce: Nonce,
    /// Server addresses the client may try, in order.
    pub servers: Vec<ServerAddress>,
    /// Opaque bytes the server hands back to the game; never read here.
    pub private_data: Vec<u8>,
    /// Seconds a session stays alive after the token is accepted.
    pub timeout_seconds: i32,
    /// Key the client seals its connection traffic with.
    pub client_to_server_key: SessionKey,
    /// Key the server seals its connection traffic with.
    pub server_to_client_key: SessionKey,
}

impl ConnectToken {
    /// Encode the plaintext shape: little-endian and integer-only, with a
    /// length before the server list and the private data. Counts above
    /// `u32::MAX` saturate, which the generator's bounds make unreachable.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(CONNECT_TOKEN_VERSION);
        out.extend_from_slice(&self.protocol_id.to_le_bytes());
        out.extend_from_slice(&self.create_timestamp.to_le_bytes());
        out.extend_from_slice(&self.expire_timestamp.to_le_bytes());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.timeout_seconds.to_le_bytes());
        put_len32(&mut out, self.servers.len());
        for server in &self.servers {
            out.extend_from_slice(&server.ip);
            out.extend_from_slice(&server.port.to_le_bytes());
        }
        put_len32(&mut out, self.private_data.len());
        out.extend_from_slice(&self.private_data);
        out.extend_from_slice(&self.client_to_server_key);
        out.extend_from_slice(&self.server_to_client_key);
        out
    }

    /// Decode a plaintext token. Bounds are enforced ([`MAX_SERVERS`],
    /// [`MAX_PRIVATE_DATA`]) and trailing bytes refuse; nothing is truncated.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HandshakeError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.u8()? != CONNECT_TOKEN_VERSION {
            return Err(HandshakeError::Malformed);
        }
        let protocol_id = cursor.u64()?;
        let create_timestamp = cursor.i64()?;
        let expire_timestamp = cursor.i64()?;
        let nonce = cursor.arr::<NONCE_LEN>()?;
        let timeout_seconds = cursor.i32()?;
        let server_count = count32(&mut cursor, MAX_SERVERS)?;
        let mut servers = Vec::with_capacity(server_count);
        for _ in 0..server_count {
            let ip = cursor.arr::<4>()?;
            servers.push(ServerAddress {
                ip,
                port: cursor.u16()?,
            });
        }
        let private_len = count32(&mut cursor, MAX_PRIVATE_DATA)?;
        let private_data = cursor.take(private_len)?.to_vec();
        let client_to_server_key = cursor.arr::<KEY_LEN>()?;
        let server_to_client_key = cursor.arr::<KEY_LEN>()?;
        if !cursor.is_empty() {
            return Err(HandshakeError::Malformed);
        }
        Ok(Self {
            protocol_id,
            create_timestamp,
            expire_timestamp,
            nonce,
            servers,
            private_data,
            timeout_seconds,
            client_to_server_key,
            server_to_client_key,
        })
    }

    /// True when `now` is at or past [`Self::expire_timestamp`].
    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expire_timestamp
    }
}

/// A sealed message: the 24-byte seal nonce, the payload and the tag. Used
/// for the connect token in flight, the challenge and the response.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sealed {
    /// Seal nonce, mixed into the tag with the payload.
    pub nonce: Nonce,
    /// The message bytes.
    pub payload: Vec<u8>,
    /// The authentication tag, [`TAG_LEN`] bytes.
    pub tag: Vec<u8>,
}

impl Sealed {
    /// Seal `payload` under `key` with `nonce`.
    pub fn seal(key: &[u8], nonce: &Nonce, payload: &[u8]) -> Result<Self, HandshakeError> {
        Ok(Self {
            nonce: *nonce,
            payload: payload.to_vec(),
            tag: auth::tag(key, nonce, payload)?,
        })
    }

    /// Verify the tag under `key` and lend the payload. The tag is checked
    /// before the payload is trusted, so a tampered byte never gets past here.
    pub fn open(&self, key: &[u8]) -> Result<&[u8], HandshakeError> {
        if auth::verify(key, &self.nonce, &self.payload, &self.tag) {
            Ok(&self.payload)
        } else {
            Err(HandshakeError::BadToken)
        }
    }

    /// Wire bytes: `nonce || len(payload) || payload || tag`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(NONCE_LEN + 4 + self.payload.len() + self.tag.len());
        out.extend_from_slice(&self.nonce);
        put_len32(&mut out, self.payload.len());
        out.extend_from_slice(&self.payload);
        out.extend_from_slice(&self.tag);
        out
    }

    /// Parse wire bytes. The tag is carried, not checked: [`Self::open`]
    /// gates trust. A tag of another length refuses as
    /// [`HandshakeError::Malformed`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HandshakeError> {
        let mut cursor = Cursor::new(bytes);
        let nonce = cursor.arr::<NONCE_LEN>()?;
        let tag_len = TAG_LEN;
        let payload_len = payload_len32(&mut cursor, tag_len)?;
        let payload = cursor.take(payload_len)?.to_vec();
        let tag = cursor.rest().to_vec();
        if tag.len() != tag_len {
            return Err(HandshakeError::Malformed);
        }
        Ok(Self {
            nonce,
            payload,
            tag,
        })
    }
}

/// Seal a connect token for the wire under the server key. The seal nonce is
/// the token's own nonce, so one session nonce appears once.
pub fn seal_token(server_key: &[u8], token: &ConnectToken) -> Result<Sealed, HandshakeError> {
    Sealed::seal(server_key, &token.nonce, &token.to_bytes())
}

/// Verify a sealed connect token under the server key and decode it. The tag
/// is checked before a single field is believed.
pub fn open_token(server_key: &[u8], sealed: &Sealed) -> Result<ConnectToken, HandshakeError> {
    let payload = sealed.open(server_key)?;
    ConnectToken::from_bytes(payload)
}

/// The agreed session keys: one per direction, plus the index and the connect
/// token nonce that name the session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SessionKeys {
    /// Server-assigned session index.
    pub session_index: u32,
    /// Key the client seals to the server with.
    pub client_to_server_key: SessionKey,
    /// Key the server seals to the client with.
    pub server_to_client_key: SessionKey,
    /// The connect token nonce this session was opened from.
    pub connect_token_nonce: Nonce,
}

/// Derive the session keys both ends agree on: the token's directional keys,
/// derived with the challenge nonce so a new challenge moves the keys.
fn session_keys(
    token: &ConnectToken,
    session_index: u32,
    challenge: &Nonce,
) -> Result<SessionKeys, HandshakeError> {
    Ok(SessionKeys {
        session_index,
        client_to_server_key: auth::derive(&token.client_to_server_key, LABEL_C2S, challenge)?,
        server_to_client_key: auth::derive(&token.server_to_client_key, LABEL_S2C, challenge)?,
        connect_token_nonce: token.nonce,
    })
}

/// What the token generator hands out: the plaintext token the client keeps
/// (its session keys live inside) and the sealed bytes it presents.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GeneratedToken {
    /// Plaintext token: the client reads its keys here.
    pub token: ConnectToken,
    /// Sealed bytes the server authenticates.
    pub sealed: Sealed,
}

/// Mints connect tokens for one protocol id under one server key. The key and
/// nonce stream is seeded, so a run is reproducible; a deployment seeds from
/// OS randomness outside this module.
#[derive(Clone, Debug)]
pub struct TokenGenerator {
    protocol_id: u64,
    server_key: SessionKey,
    stream: u64,
}

impl TokenGenerator {
    /// New generator for `protocol_id` sealing under `server_key`, with
    /// `seed` driving its nonce and key stream.
    pub fn new(protocol_id: u64, server_key: SessionKey, seed: u64) -> Self {
        Self {
            protocol_id,
            server_key,
            stream: seed,
        }
    }

    /// The protocol id every token this generator mints carries.
    pub fn protocol_id(&self) -> u64 {
        self.protocol_id
    }

    /// Mint one token valid from `now` for `timeout_seconds`, offering
    /// `servers` (one to [`MAX_SERVERS`] addresses).
    pub fn generate(
        &mut self,
        now: i64,
        timeout_seconds: i32,
        servers: &[ServerAddress],
    ) -> Result<GeneratedToken, HandshakeError> {
        if servers.is_empty() || servers.len() > MAX_SERVERS {
            return Err(HandshakeError::TooLong);
        }
        let mut nonce = [0u8; NONCE_LEN];
        fill_stream(&mut self.stream, &mut nonce);
        let mut client_to_server_key = [0u8; KEY_LEN];
        fill_stream(&mut self.stream, &mut client_to_server_key);
        let mut server_to_client_key = [0u8; KEY_LEN];
        fill_stream(&mut self.stream, &mut server_to_client_key);
        let token = ConnectToken {
            protocol_id: self.protocol_id,
            create_timestamp: now,
            expire_timestamp: now.saturating_add(i64::from(timeout_seconds)),
            nonce,
            servers: servers.to_vec(),
            private_data: Vec::new(),
            timeout_seconds,
            client_to_server_key,
            server_to_client_key,
        };
        let sealed = seal_token(&self.server_key, &token)?;
        Ok(GeneratedToken { token, sealed })
    }
}

/// Where a client handshake stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClientState {
    /// Minted; no request sent yet.
    Request,
    /// A challenge was accepted and the session keys are derived.
    Challenge,
    /// The response was built; the handshake is done from this end.
    Connected,
}

/// Client side of the connect handshake: connection request, challenge,
/// response.
#[derive(Clone, Debug)]
pub struct Client {
    protocol_id: u64,
    token: ConnectToken,
    sealed: Sealed,
    session_index: u32,
    challenge: Nonce,
    keys: Option<SessionKeys>,
    state: ClientState,
}

impl Client {
    /// Open a client handshake from the token the join backend handed out
    /// (its keys are read here) and the sealed form it presents.
    pub fn new(protocol_id: u64, token: ConnectToken, sealed: Sealed) -> Self {
        Self {
            protocol_id,
            token,
            sealed,
            session_index: 0,
            challenge: [0u8; NONCE_LEN],
            keys: None,
            state: ClientState::Request,
        }
    }

    /// Where this handshake stands.
    pub fn state(&self) -> ClientState {
        self.state
    }

    /// The connection request to send to the server.
    pub fn request(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(HANDSHAKE_VERSION);
        out.extend_from_slice(&self.protocol_id.to_le_bytes());
        out.extend_from_slice(&self.sealed.to_bytes());
        out
    }

    /// Accept the server's challenge: verify it under the token's
    /// server-to-client key and derive the session keys. Refuses when no
    /// request is outstanding or the seal does not verify.
    pub fn on_challenge(&mut self, bytes: &[u8]) -> Result<(), HandshakeError> {
        if self.state != ClientState::Request {
            return Err(HandshakeError::OutOfOrder);
        }
        let sealed = Sealed::from_bytes(bytes)?;
        let payload = sealed.open(&self.token.server_to_client_key)?;
        let (session_index, challenge) = decode_index_nonce(payload)?;
        self.keys = Some(session_keys(&self.token, session_index, &challenge)?);
        self.session_index = session_index;
        self.challenge = challenge;
        self.state = ClientState::Challenge;
        Ok(())
    }

    /// The response to send after a challenge was accepted: sealed under the
    /// derived client-to-server key, over the index and the challenge nonce.
    pub fn response(&mut self) -> Result<Vec<u8>, HandshakeError> {
        if self.state != ClientState::Challenge {
            return Err(HandshakeError::OutOfOrder);
        }
        let keys = self.keys.ok_or(HandshakeError::OutOfOrder)?;
        let payload = encode_index_nonce(self.session_index, &self.challenge);
        let sealed = Sealed::seal(&keys.client_to_server_key, &self.challenge, &payload)?;
        self.state = ClientState::Connected;
        Ok(sealed.to_bytes())
    }

    /// The agreed session keys, once a challenge was accepted.
    pub fn keys(&self) -> Option<SessionKeys> {
        self.keys
    }
}

/// One session between its request and its response.
#[derive(Clone, Debug)]
struct Pending {
    index: u32,
    challenge: Nonce,
    keys: SessionKeys,
}

/// Server side of the connect handshake: verifies the sealed token, answers a
/// challenge and completes on the response. Keeps no wall clock.
#[derive(Clone, Debug)]
pub struct Server {
    protocol_id: u64,
    server_key: SessionKey,
    stream: u64,
    next_index: u32,
    seen: BTreeSet<Nonce>,
    pending: Vec<Pending>,
    connected: u64,
}

impl Server {
    /// New server for `protocol_id` that authenticates tokens sealed under
    /// `server_key`; `seed` drives the deterministic challenge-nonce stream.
    pub fn new(protocol_id: u64, server_key: SessionKey, seed: u64) -> Self {
        Self {
            protocol_id,
            server_key,
            stream: seed,
            next_index: 0,
            seen: BTreeSet::new(),
            pending: Vec::new(),
            connected: 0,
        }
    }

    /// Sessions waiting for a response.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Sessions that completed a handshake.
    pub fn connected_count(&self) -> u64 {
        self.connected
    }

    /// Handle a connection request and return the challenge to send back.
    /// Refuses a bad seal, another protocol id, an expired or not-yet-valid
    /// token, a replayed nonce, or a full pending table.
    pub fn on_request(&mut self, bytes: &[u8], now: i64) -> Result<Vec<u8>, HandshakeError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.u8()? != HANDSHAKE_VERSION {
            return Err(HandshakeError::Malformed);
        }
        if cursor.u64()? != self.protocol_id {
            return Err(HandshakeError::WrongSession);
        }
        let sealed = Sealed::from_bytes(cursor.rest())?;
        let token = open_token(&self.server_key, &sealed)?;
        if token.protocol_id != self.protocol_id {
            return Err(HandshakeError::WrongSession);
        }
        if token.is_expired(now) {
            return Err(HandshakeError::Expired {
                now,
                expire: token.expire_timestamp,
            });
        }
        if now < token.create_timestamp {
            return Err(HandshakeError::NotYetValid {
                now,
                create: token.create_timestamp,
            });
        }
        if self.seen.contains(&token.nonce) {
            return Err(HandshakeError::WrongSession);
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(HandshakeError::TooLong);
        }
        let index = self.next_index;
        self.next_index = self.next_index.wrapping_add(1);
        let mut challenge = [0u8; NONCE_LEN];
        fill_stream(&mut self.stream, &mut challenge);
        let keys = session_keys(&token, index, &challenge)?;
        let payload = encode_index_nonce(index, &challenge);
        let sealed = Sealed::seal(&token.server_to_client_key, &challenge, &payload)?;
        self.seen.insert(token.nonce);
        self.pending.push(Pending {
            index,
            challenge,
            keys,
        });
        Ok(sealed.to_bytes())
    }

    /// Complete a handshake: verify the response under the session key the
    /// pending challenge derived. The session index routes to a key; nothing
    /// else is believed before the tag verifies.
    pub fn on_response(&mut self, bytes: &[u8]) -> Result<SessionKeys, HandshakeError> {
        let sealed = Sealed::from_bytes(bytes)?;
        let (index, _) = decode_index_nonce(&sealed.payload)?;
        let at = self
            .pending
            .iter()
            .position(|p| p.index == index)
            .ok_or(HandshakeError::WrongSession)?;
        let keys = self.pending[at].keys;
        let payload = sealed.open(&keys.client_to_server_key)?;
        let (again, challenge) = decode_index_nonce(payload)?;
        if again != index || challenge != self.pending[at].challenge {
            return Err(HandshakeError::WrongSession);
        }
        self.pending.swap_remove(at);
        self.connected = self.connected.saturating_add(1);
        Ok(keys)
    }
}

/// Wire shape of a challenge and a response payload: the session index then
/// the challenge nonce.
fn encode_index_nonce(index: u32, nonce: &Nonce) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + NONCE_LEN);
    out.extend_from_slice(&index.to_le_bytes());
    out.extend_from_slice(nonce);
    out
}

/// Decode a challenge or response payload; trailing bytes refuse.
fn decode_index_nonce(bytes: &[u8]) -> Result<(u32, Nonce), HandshakeError> {
    let mut cursor = Cursor::new(bytes);
    let index = cursor.u32()?;
    let nonce = cursor.arr::<NONCE_LEN>()?;
    if !cursor.is_empty() {
        return Err(HandshakeError::Malformed);
    }
    Ok((index, nonce))
}

/// Append a length as a `u32`, saturating above `u32::MAX`.
fn put_len32(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
}

/// Read a bounded `u32` count and refuse one above `max`.
fn count32(cursor: &mut Cursor<'_>, max: usize) -> Result<usize, HandshakeError> {
    let count = usize::try_from(cursor.u32()?).map_err(|_| HandshakeError::Malformed)?;
    if count > max {
        return Err(HandshakeError::TooLong);
    }
    Ok(count)
}

/// Read a payload length that must leave exactly `trailer` bytes behind.
fn payload_len32(cursor: &mut Cursor<'_>, trailer: usize) -> Result<usize, HandshakeError> {
    let len = usize::try_from(cursor.u32()?).map_err(|_| HandshakeError::Malformed)?;
    if len.saturating_add(trailer) > cursor.remaining() {
        return Err(HandshakeError::Malformed);
    }
    Ok(len)
}

/// Fill `out` from a SplitMix64 stream: deterministic keys and nonces, whole
/// little-endian words.
fn fill_stream(state: &mut u64, out: &mut [u8]) {
    for chunk in out.chunks_mut(8) {
        *state = engine_core::splitmix64(*state);
        chunk.copy_from_slice(&state.to_le_bytes());
    }
}

/// Bounds-checked byte reader: every field is taken or the read refuses.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    /// Reader over `bytes` at offset zero.
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    /// Bytes left to read.
    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    /// True when every byte was read.
    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Take `n` bytes, or refuse a field that runs past the end.
    fn take(&mut self, n: usize) -> Result<&'a [u8], HandshakeError> {
        let end = self.at.checked_add(n).ok_or(HandshakeError::Malformed)?;
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(HandshakeError::Malformed)?;
        self.at = end;
        Ok(slice)
    }

    /// Take a fixed-size array.
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], HandshakeError> {
        let slice = self.take(N)?;
        <[u8; N]>::try_from(slice).map_err(|_| HandshakeError::Malformed)
    }

    /// Everything left, unconsumed.
    fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.at..).unwrap_or(&[])
    }

    /// A `u8`, little-endian by definition.
    fn u8(&mut self) -> Result<u8, HandshakeError> {
        Ok(self.arr::<1>()?[0])
    }

    /// A `u16` as two little-endian bytes.
    fn u16(&mut self) -> Result<u16, HandshakeError> {
        Ok(u16::from_le_bytes(self.arr::<2>()?))
    }

    /// A `u32` as four little-endian bytes.
    fn u32(&mut self) -> Result<u32, HandshakeError> {
        Ok(u32::from_le_bytes(self.arr::<4>()?))
    }

    /// A `u64` as eight little-endian bytes.
    fn u64(&mut self) -> Result<u64, HandshakeError> {
        Ok(u64::from_le_bytes(self.arr::<8>()?))
    }

    /// An `i32` as four little-endian bytes.
    fn i32(&mut self) -> Result<i32, HandshakeError> {
        Ok(i32::from_le_bytes(self.arr::<4>()?))
    }

    /// An `i64` as eight little-endian bytes.
    fn i64(&mut self) -> Result<i64, HandshakeError> {
        Ok(i64::from_le_bytes(self.arr::<8>()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one address every test token offers.
    fn servers() -> Vec<ServerAddress> {
        vec![ServerAddress {
            ip: [127, 0, 0, 1],
            port: 40_000,
        }]
    }

    /// A fixed server key so a run is reproducible.
    fn server_key() -> SessionKey {
        let mut key = [0u8; KEY_LEN];
        for (at, byte) in key.iter_mut().enumerate() {
            *byte = u8::try_from(at).expect("lane fits") ^ 0x5a;
        }
        key
    }

    /// Mint a token for `key` with `seed`.
    fn mint(key: SessionKey, seed: u64, now: i64) -> GeneratedToken {
        TokenGenerator::new(PROTOCOL_ID, key, seed)
            .generate(now, 30, &servers())
            .expect("a bounded token mints")
    }

    /// Run the whole four-step exchange; returns every message plus both ends'
    /// keys.
    fn roundtrip(
        key: SessionKey,
        seed: u64,
        now: i64,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, SessionKeys, SessionKeys) {
        let minted = mint(key, seed, now);
        let mut client = Client::new(PROTOCOL_ID, minted.token.clone(), minted.sealed.clone());
        let mut server = Server::new(PROTOCOL_ID, key, seed ^ 0x9e37_79b9_7f4a_7c15);
        let request = client.request();
        let challenge = server.on_request(&request, now).expect("request accepted");
        client.on_challenge(&challenge).expect("challenge accepted");
        let response = client.response().expect("response built");
        let server_keys = server.on_response(&response).expect("response accepted");
        let client_keys = client.keys().expect("client holds keys");
        (request, challenge, response, client_keys, server_keys)
    }

    #[test]
    fn handshake_roundtrip_x2() {
        let key = server_key();
        let first = roundtrip(key, 21, 1_000);
        let second = roundtrip(key, 21, 1_000);
        // The same seed replays every message byte for byte.
        assert_eq!(first, second, "roundtrip x2 is byte-identical");
        assert!(first.0.len() > NONCE_LEN && first.1.len() > NONCE_LEN);
        assert!(first.2.len() > NONCE_LEN);
        // Both ends hold the same session keys.
        assert_eq!(first.3, first.4, "client and server agree on the keys");
        assert_eq!(first.3.session_index, 0);
        assert_eq!(
            first.3.connect_token_nonce,
            mint(key, 21, 1_000).token.nonce
        );
        assert_ne!(
            first.3.client_to_server_key, first.3.server_to_client_key,
            "the two directions derive different keys"
        );
        // A different seed mints different keys and different bytes.
        let other = roundtrip(key, 22, 1_000);
        assert_ne!(other.3, first.3, "a new seed moves the session keys");
        assert_ne!(other.1, first.1);
        // A fresh client starts at the request state with no keys yet.
        let minted = mint(key, 21, 1_000);
        let client = Client::new(PROTOCOL_ID, minted.token, minted.sealed);
        assert_eq!(client.state(), ClientState::Request);
        assert_eq!(client.keys(), None);
    }

    #[test]
    fn bad_keys_refuse() {
        let key = server_key();
        let now = 1_000;
        let minted = mint(key, 7, now);
        let request =
            Client::new(PROTOCOL_ID, minted.token.clone(), minted.sealed.clone()).request();

        // 1. a token sealed under the wrong key refuses.
        let other_key = [9u8; KEY_LEN];
        assert_eq!(
            open_token(&other_key, &minted.sealed),
            Err(HandshakeError::BadToken)
        );
        let mut wrong_key_server = Server::new(PROTOCOL_ID, other_key, 3);
        assert_eq!(
            wrong_key_server.on_request(&request, now),
            Err(HandshakeError::BadToken)
        );

        // 2. a tampered token refuses (one flipped payload byte).
        let mut tampered = minted.sealed.clone();
        let at = tampered.payload.len() / 2;
        tampered.payload[at] ^= 0x01;
        assert_eq!(open_token(&key, &tampered), Err(HandshakeError::BadToken));

        // 3. an expired token refuses.
        let mut server = Server::new(PROTOCOL_ID, key, 3);
        assert_eq!(
            server.on_request(&request, now + 30),
            Err(HandshakeError::Expired {
                now: now + 30,
                expire: now + 30,
            })
        );

        // 4. another session refuses: another protocol id, then a replay.
        let elsewhere = mint(key, 8, now);
        let elsewhere = Client::new(PROTOCOL_ID + 1, elsewhere.token, elsewhere.sealed).request();
        assert_eq!(
            server.on_request(&elsewhere, now),
            Err(HandshakeError::WrongSession)
        );
        assert!(
            server.on_request(&request, now).is_ok(),
            "first use is fine"
        );
        assert_eq!(
            server.on_request(&request, now),
            Err(HandshakeError::WrongSession),
            "the same token cannot open two sessions"
        );

        // 5. a response sealed under a key the server did not derive refuses,
        //    even with the right session index and challenge nonce.
        let fresh = mint(key, 9, now);
        let mut client = Client::new(PROTOCOL_ID, fresh.token.clone(), fresh.sealed.clone());
        let mut server = Server::new(PROTOCOL_ID, key, 4);
        let challenge = server
            .on_request(&client.request(), now)
            .expect("request accepted");
        let opened = Sealed::from_bytes(&challenge).expect("challenge parses");
        let payload = opened
            .open(&fresh.token.server_to_client_key)
            .expect("challenge verifies");
        let forged = Sealed::seal(&[1u8; KEY_LEN], &[2u8; NONCE_LEN], payload)
            .expect("a seal always builds");
        assert_eq!(
            server.on_response(&forged.to_bytes()),
            Err(HandshakeError::BadToken)
        );
        client
            .on_challenge(&challenge)
            .expect("the real challenge verifies");
        let response = client.response().expect("response builds");
        assert!(
            server.on_response(&response).is_ok(),
            "the real response still completes the session"
        );
        assert_eq!(server.connected_count(), 1);
        assert_eq!(server.pending_count(), 0);
    }

    #[test]
    fn token_shape_roundtrips_and_bounds_refuse() {
        let key = server_key();
        let minted = mint(key, 31, 500);
        let bytes = minted.token.to_bytes();
        assert_eq!(
            ConnectToken::from_bytes(&bytes),
            Ok(minted.token.clone()),
            "the plaintext shape round-trips"
        );
        assert!(!minted.token.is_expired(500));
        assert!(minted.token.is_expired(530));
        assert_eq!(minted.token.timeout_seconds, 30);

        // Truncation and trailing bytes refuse, never panic.
        assert_eq!(
            ConnectToken::from_bytes(&bytes[..bytes.len() - 1]),
            Err(HandshakeError::Malformed)
        );
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(
            ConnectToken::from_bytes(&long),
            Err(HandshakeError::Malformed)
        );
        assert_eq!(
            ConnectToken::from_bytes(&[]),
            Err(HandshakeError::Malformed)
        );

        // Counts above their bound refuse.
        let mut too_many = bytes.clone();
        let servers_at = 1 + 8 + 8 + 8 + NONCE_LEN + 4;
        let big = u32::try_from(MAX_SERVERS + 1).expect("fits").to_le_bytes();
        too_many[servers_at..servers_at + 4].copy_from_slice(&big);
        assert_eq!(
            ConnectToken::from_bytes(&too_many),
            Err(HandshakeError::TooLong)
        );

        // A generator refuses an empty or oversize server list.
        let mut generator = TokenGenerator::new(PROTOCOL_ID, key, 2);
        assert_eq!(generator.generate(0, 5, &[]), Err(HandshakeError::TooLong));
        let many = vec![
            ServerAddress {
                ip: [0, 0, 0, 0],
                port: 1
            };
            MAX_SERVERS + 1
        ];
        assert_eq!(
            generator.generate(0, 5, &many),
            Err(HandshakeError::TooLong)
        );
        assert_eq!(generator.protocol_id(), PROTOCOL_ID);
    }

    #[test]
    fn client_out_of_order_refuses() {
        let key = server_key();
        let minted = mint(key, 41, 10);
        let mut client = Client::new(PROTOCOL_ID, minted.token, minted.sealed);
        assert_eq!(client.response(), Err(HandshakeError::OutOfOrder));
        assert_eq!(
            client.on_challenge(&[0u8; 8]),
            Err(HandshakeError::Malformed)
        );
        let good = roundtrip(key, 41, 10).1;
        client.on_challenge(&good).expect("challenge accepted");
        assert_eq!(client.state(), ClientState::Challenge);
        assert_eq!(
            client.on_challenge(&good),
            Err(HandshakeError::OutOfOrder),
            "a second challenge refuses"
        );
        let response = client.response().expect("response builds");
        assert_eq!(client.state(), ClientState::Connected);
        assert!(response.len() > NONCE_LEN);
        assert_eq!(client.response(), Err(HandshakeError::OutOfOrder));
    }

    #[test]
    fn error_messages_name_the_refusal() {
        assert!(HandshakeError::BadToken.to_string().contains("seal"));
        assert!(HandshakeError::Expired { now: 9, expire: 9 }
            .to_string()
            .contains("expired"));
        assert!(HandshakeError::WrongSession
            .to_string()
            .contains("another session"));
        assert!(HandshakeError::Malformed.to_string().contains("malformed"));
        assert!(HandshakeError::TooLong.to_string().contains("bound"));
        assert!(HandshakeError::OutOfOrder
            .to_string()
            .contains("out of order"));
        assert!(HandshakeError::NotYetValid { now: 1, create: 2 }
            .to_string()
            .contains("valid from"));
    }

    /// Lowercase hex, for the golden-vector cross-check.
    #[cfg(feature = "crypto")]
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[cfg(feature = "crypto")]
    #[test]
    fn crypto_seals_are_hmac_sha256() {
        // Golden vector: HMAC-SHA256 over `nonce || message` under a 32-byte
        // key, computed independently with node's `crypto` HMAC. It pins the
        // seal to the real primitive, so a change of tag or message order
        // fails here instead of silently in a session.
        assert_eq!(TAG_LEN, 32);
        let key: SessionKey = std::array::from_fn(|at| u8::try_from(at).expect("lane fits"));
        let nonce: Nonce = std::array::from_fn(|at| u8::try_from(at).expect("byte fits"));
        let tag = auth::tag(&key, &nonce, b"handshake-selftest").expect("tag builds");
        assert_eq!(
            hex(&tag),
            "a4b80d8749d70b2d83f1c32caf8e400262a91c7dad2092b8f70f4475cd68f254"
        );
        assert!(auth::verify(&key, &nonce, b"handshake-selftest", &tag));
        assert!(!auth::verify(&key, &nonce, b"handshake-selftesu", &tag));
        assert!(!auth::verify(
            &[0u8; KEY_LEN],
            &nonce,
            b"handshake-selftest",
            &tag
        ));
    }

    #[cfg(not(feature = "crypto"))]
    #[test]
    fn dev_seal_is_not_authentication() {
        // The default build carries the documented 8-byte fold: a development
        // shape, pinned here so it cannot pass for HMAC by accident.
        assert_eq!(TAG_LEN, 8);
        let sealed = mint(server_key(), 51, 0).sealed;
        assert_eq!(sealed.tag.len(), 8);
        assert_eq!(Sealed::from_bytes(&sealed.to_bytes()), Ok(sealed));
    }
}
