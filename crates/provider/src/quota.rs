//! quota: per-player and per-session model budgets, rate windows, and key
//! safety for shipped games.
//!
//! Owner: this module owns quota accounting, the closed key-route contract
//! and the package key scan. Units: token counts and spend in whole minor
//! units; time in caller-supplied simulated milliseconds. Determinism: the
//! same charge sequence at the same timestamps gives the same receipt or
//! refusal, with no clock read. Errors: [`QuotaConfigError`],
//! [`QuotaRefusal`], [`KeySafetyError`] and [`ScanError`] carry labels and
//! integers only; no key value appears in any error, notice or `Debug`.
//!
//! A quota overrun converts to the shared [`ScriptedFallback`] with the
//! quota reason and a ring-ready notice. A shipped build holds no provider
//! key: [`KeyRoute`] has no inline variant, player keys live behind
//! [`OsKeyStore`], and [`scan_package`] fails the release when a key-shaped
//! string is found in the package tree.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use super::fallback::{fallback_for, FallbackErr, ScriptedFallback, MAX_NOTICE_LEN};
use super::policy::AiSessionPolicy;
use super::protocol::{AiError, AiRequest};
use super::stream::truncate_str;
use super::transport::{is_loopback, parse_endpoint, EndpointError};

/// Largest number of players one book tracks before refusing new ones.
pub const MAX_TRACKED_PLAYERS: usize = 256;
/// Largest number of sessions one book tracks before refusing new ones.
pub const MAX_TRACKED_SESSIONS: usize = 1024;

/// Stable player identity: an integer the app assigns, never a name or key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PlayerId(pub u64);

/// Stable session identity: an integer this book assigns at open time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SessionId(pub u64);

/// One request rate window: at most `max_requests` in any `window_ms`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RateLimit {
    /// Requests admitted per window.
    pub max_requests: u32,
    /// Window length in milliseconds; must be non-zero.
    pub window_ms: u64,
}

/// Lifetime limits for one player or one session. Spend `None` means the
/// scope has no spend cap and only tokens and requests bound it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuotaLimits {
    /// Requests admitted over the whole scope.
    pub requests: u32,
    /// Tokens admitted over the whole scope.
    pub tokens: u64,
    /// Spend cap in minor units, or `None` for no spend cap.
    pub spend: Option<u64>,
    /// Optional short-window request rate.
    pub rate: Option<RateLimit>,
}

/// A validated pair of scopes. The player budget spans sessions; the
/// session budget is spent fresh for each opened session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuotaPlan {
    /// Budget for one player across every session in this book.
    pub player: QuotaLimits,
    /// Budget for one opened session.
    pub session: QuotaLimits,
}

/// Why a plan was refused before it could admit anything.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuotaConfigError {
    /// A scope admits zero requests.
    ZeroRequests,
    /// A scope admits zero tokens.
    ZeroTokens,
    /// A spend cap is zero, which would refuse every request.
    ZeroSpend,
    /// A rate window admits zero requests.
    ZeroRate,
    /// A rate window has zero length.
    ZeroWindow,
}

impl fmt::Display for QuotaConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            QuotaConfigError::ZeroRequests => "quota: request limit must be non-zero",
            QuotaConfigError::ZeroTokens => "quota: token limit must be non-zero",
            QuotaConfigError::ZeroSpend => "quota: spend cap must be non-zero",
            QuotaConfigError::ZeroRate => "quota: rate limit must be non-zero",
            QuotaConfigError::ZeroWindow => "quota: rate window must be non-zero",
        };
        f.write_str(text)
    }
}

impl std::error::Error for QuotaConfigError {}

impl QuotaPlan {
    /// Validate both scopes. A zero limit is a misconfiguration, never an
    /// implicit "unlimited".
    pub fn new(player: QuotaLimits, session: QuotaLimits) -> Result<Self, QuotaConfigError> {
        validate_limits(&player)?;
        validate_limits(&session)?;
        Ok(Self { player, session })
    }
}

fn validate_limits(limits: &QuotaLimits) -> Result<(), QuotaConfigError> {
    if limits.requests == 0 {
        return Err(QuotaConfigError::ZeroRequests);
    }
    if limits.tokens == 0 {
        return Err(QuotaConfigError::ZeroTokens);
    }
    if limits.spend == Some(0) {
        return Err(QuotaConfigError::ZeroSpend);
    }
    if let Some(rate) = limits.rate {
        if rate.max_requests == 0 {
            return Err(QuotaConfigError::ZeroRate);
        }
        if rate.window_ms == 0 {
            return Err(QuotaConfigError::ZeroWindow);
        }
    }
    Ok(())
}

/// Counters for one scope since it opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct QuotaUsage {
    /// Requests admitted.
    pub requests: u32,
    /// Tokens admitted.
    pub tokens: u64,
    /// Spend admitted, in minor units.
    pub spend: u64,
}

/// What one admitted request costs. One request always costs one request;
/// token and spend cost are supplied by the caller from its estimate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuotaCharge {
    /// Tokens this request is expected to use.
    pub tokens: u64,
    /// Spend this request is expected to cost, in minor units.
    pub spend: u64,
}

impl QuotaCharge {
    /// Charge for one request with the given token and spend cost.
    pub const fn new(tokens: u64, spend: u64) -> Self {
        Self { tokens, spend }
    }
}

/// Which scope refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuotaScope {
    /// The player-wide budget.
    Player,
    /// The current session budget.
    Session,
}

impl QuotaScope {
    /// Short label used in notices.
    pub const fn label(self) -> &'static str {
        match self {
            QuotaScope::Player => "player",
            QuotaScope::Session => "session",
        }
    }
}

/// Which limit refused. Labels are fixed words, never caller text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuotaLimit {
    /// Lifetime request count.
    Requests,
    /// Lifetime token count.
    Tokens,
    /// Lifetime spend.
    Spend,
    /// Short-window request rate.
    Rate,
    /// Tracked player or session table capacity.
    Sessions,
}

impl QuotaLimit {
    /// Short label used in notices.
    pub const fn label(self) -> &'static str {
        match self {
            QuotaLimit::Requests => "requests",
            QuotaLimit::Tokens => "tokens",
            QuotaLimit::Spend => "spend",
            QuotaLimit::Rate => "rate",
            QuotaLimit::Sessions => "sessions",
        }
    }
}

/// One refused charge: which scope, which limit, what was allowed and what
/// was already used. Carries integers only, never a request body or a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuotaRefusal {
    /// Scope that refused.
    pub scope: QuotaScope,
    /// Limit that refused.
    pub limit: QuotaLimit,
    /// Limit value in its own unit.
    pub allowed: u64,
    /// Usage before the refused charge, in the same unit.
    pub used: u64,
    /// Player or session id the refusal is about; zero before an id exists.
    pub subject: u64,
}

impl QuotaRefusal {
    fn new(scope: QuotaScope, limit: QuotaLimit, allowed: u64, used: u64, subject: u64) -> Self {
        Self {
            scope,
            limit,
            allowed,
            used,
            subject,
        }
    }

    fn unknown(scope: QuotaScope, subject: u64) -> Self {
        Self::new(scope, QuotaLimit::Sessions, 0, 0, subject)
    }

    /// Ring-ready text for the notice path: fixed labels and integers only.
    pub fn message(&self) -> String {
        format!(
            "scope={} subject={} limit={} allowed={} used={}",
            self.scope.label(),
            self.subject,
            self.limit.label(),
            self.allowed,
            self.used
        )
    }

    /// Serve the refused request on the shared scripted path. The notice
    /// names the quota reason and the proposal is the validating no-op the
    /// caller presents as its scripted line; a policy without scripting
    /// refuses loudly through [`FallbackErr`].
    pub fn scripted(
        self,
        request: &AiRequest,
        policy: &AiSessionPolicy,
    ) -> Result<ScriptedFallback, FallbackErr> {
        let mut served = fallback_for(AiError::QuotaAuth, request, policy)?;
        let message = format!(
            "provider.fallback cause=refused error=QuotaAuth budget_ms={} quota {} scripted",
            request.budget.max_ms,
            self.message()
        );
        truncate_str(&message, MAX_NOTICE_LEN).clone_into(&mut served.notice.message);
        served
            .proposal
            .diagnostics
            .push(format!("quota overrun: {}", self.message()));
        Ok(served)
    }
}

/// Usage after one admitted charge, for both scopes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuotaReceipt {
    /// Player scope usage including this charge.
    pub player: QuotaUsage,
    /// Session scope usage including this charge.
    pub session: QuotaUsage,
}

#[derive(Clone, Copy, Debug, Default)]
struct QuotaRow {
    usage: QuotaUsage,
    rate_start_ms: u64,
    rate_count: u32,
}

#[derive(Clone, Copy, Debug)]
struct SessionRow {
    owner: PlayerId,
    row: QuotaRow,
}

/// The per-player and per-session book. A charge checks both scopes before
/// it moves either one, so a refusal never spends part of a request.
#[derive(Debug)]
pub struct QuotaBook {
    plan: QuotaPlan,
    players: HashMap<PlayerId, QuotaRow>,
    sessions: HashMap<SessionId, SessionRow>,
    next_session: u64,
}

impl QuotaBook {
    /// Start an empty book for `plan`.
    pub fn new(plan: QuotaPlan) -> Self {
        Self {
            plan,
            players: HashMap::new(),
            sessions: HashMap::new(),
            next_session: 0,
        }
    }

    /// The plan this book enforces.
    pub fn plan(&self) -> &QuotaPlan {
        &self.plan
    }

    /// Open one session for `player`, creating the player row on first use.
    /// The book is bounded; a full table refuses instead of growing.
    pub fn open_session(&mut self, player: PlayerId) -> Result<SessionId, QuotaRefusal> {
        if self.sessions.len() >= MAX_TRACKED_SESSIONS {
            return Err(QuotaRefusal::new(
                QuotaScope::Session,
                QuotaLimit::Sessions,
                usize_to_u64(MAX_TRACKED_SESSIONS),
                usize_to_u64(self.sessions.len()),
                0,
            ));
        }
        if !self.players.contains_key(&player) {
            if self.players.len() >= MAX_TRACKED_PLAYERS {
                return Err(QuotaRefusal::new(
                    QuotaScope::Player,
                    QuotaLimit::Sessions,
                    usize_to_u64(MAX_TRACKED_PLAYERS),
                    usize_to_u64(self.players.len()),
                    player.0,
                ));
            }
            self.players.insert(player, QuotaRow::default());
        }
        self.next_session = self.next_session.saturating_add(1);
        let session = SessionId(self.next_session);
        self.sessions.insert(
            session,
            SessionRow {
                owner: player,
                row: QuotaRow::default(),
            },
        );
        Ok(session)
    }

    /// Close one session and drop its counters. Player usage stays.
    pub fn close_session(&mut self, session: SessionId) -> bool {
        self.sessions.remove(&session).is_some()
    }

    /// Book one request against both scopes at `now_ms`. All checks pass
    /// before either scope moves; a refusal leaves both counters untouched.
    pub fn charge(
        &mut self,
        now_ms: u64,
        player: PlayerId,
        session: SessionId,
        charge: QuotaCharge,
    ) -> Result<QuotaReceipt, QuotaRefusal> {
        let Some(player_row) = self.players.get(&player) else {
            return Err(QuotaRefusal::unknown(QuotaScope::Player, player.0));
        };
        let Some(session_row) = self.sessions.get(&session) else {
            return Err(QuotaRefusal::unknown(QuotaScope::Session, session.0));
        };
        if session_row.owner != player {
            return Err(QuotaRefusal::unknown(QuotaScope::Session, session.0));
        }
        let player_window = rate_window(player_row, now_ms, self.plan.player.rate);
        let session_window = rate_window(&session_row.row, now_ms, self.plan.session.rate);
        check_limits(
            &self.plan.player,
            &player_row.usage,
            player_window,
            charge,
            QuotaScope::Player,
            player.0,
        )?;
        check_limits(
            &self.plan.session,
            &session_row.row.usage,
            session_window,
            charge,
            QuotaScope::Session,
            session.0,
        )?;

        let player_row = self.players.get_mut(&player);
        let session_row = self.sessions.get_mut(&session);
        let (Some(player_row), Some(session_row)) = (player_row, session_row) else {
            return Err(QuotaRefusal::unknown(QuotaScope::Session, session.0));
        };
        apply_charge(player_row, player_window, charge);
        apply_charge(&mut session_row.row, session_window, charge);
        Ok(QuotaReceipt {
            player: player_row.usage,
            session: session_row.row.usage,
        })
    }

    /// Player usage so far, or `None` when the player has no row.
    pub fn player_usage(&self, player: PlayerId) -> Option<QuotaUsage> {
        self.players.get(&player).map(|row| row.usage)
    }

    /// Session usage so far, or `None` for a closed or unknown session.
    pub fn session_usage(&self, session: SessionId) -> Option<QuotaUsage> {
        self.sessions.get(&session).map(|row| row.row.usage)
    }
}

fn rate_window(row: &QuotaRow, now_ms: u64, rate: Option<RateLimit>) -> Option<(u64, u32)> {
    let rate = rate?;
    if now_ms.saturating_sub(row.rate_start_ms) >= rate.window_ms {
        Some((now_ms, 0))
    } else {
        Some((row.rate_start_ms, row.rate_count))
    }
}

fn check_limits(
    limits: &QuotaLimits,
    usage: &QuotaUsage,
    window: Option<(u64, u32)>,
    charge: QuotaCharge,
    scope: QuotaScope,
    subject: u64,
) -> Result<(), QuotaRefusal> {
    let requests = usage.requests.saturating_add(1);
    if requests > limits.requests {
        return Err(QuotaRefusal::new(
            scope,
            QuotaLimit::Requests,
            u64::from(limits.requests),
            u64::from(usage.requests),
            subject,
        ));
    }
    let tokens = usage.tokens.saturating_add(charge.tokens);
    if tokens > limits.tokens {
        return Err(QuotaRefusal::new(
            scope,
            QuotaLimit::Tokens,
            limits.tokens,
            usage.tokens,
            subject,
        ));
    }
    if let Some(cap) = limits.spend {
        if usage.spend.saturating_add(charge.spend) > cap {
            return Err(QuotaRefusal::new(
                scope,
                QuotaLimit::Spend,
                cap,
                usage.spend,
                subject,
            ));
        }
    }
    if let (Some(rate), Some((_, used))) = (limits.rate, window) {
        if used >= rate.max_requests {
            return Err(QuotaRefusal::new(
                scope,
                QuotaLimit::Rate,
                u64::from(rate.max_requests),
                u64::from(used),
                subject,
            ));
        }
    }
    Ok(())
}

fn apply_charge(row: &mut QuotaRow, window: Option<(u64, u32)>, charge: QuotaCharge) {
    row.usage.requests = row.usage.requests.saturating_add(1);
    row.usage.tokens = row.usage.tokens.saturating_add(charge.tokens);
    row.usage.spend = row.usage.spend.saturating_add(charge.spend);
    if let Some((start_ms, count)) = window {
        row.rate_start_ms = start_ms;
        row.rate_count = count.saturating_add(1);
    }
}

/// Longest accepted player key, in bytes. Bounds the store lookup account
/// label, not the key value.
pub const MAX_ACCOUNT_LEN: usize = 256;
/// Shortest accepted key value, in bytes.
pub const MIN_TOKEN_BYTES: usize = 16;
/// Longest accepted key value, in bytes.
pub const MAX_TOKEN_BYTES: usize = 4096;

/// One provider key value. It never prints: `Debug` redacts the bytes and
/// no `Display` or serialization exists. Only the adapter's header writer
/// should call [`SecretToken::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(Vec<u8>);

impl SecretToken {
    /// Wrap a key value. Refuses short, oversized and non-printable values:
    /// a value with control bytes could split an HTTP header.
    pub fn new(value: impl AsRef<[u8]>) -> Result<Self, KeySafetyError> {
        let value = value.as_ref();
        if value.len() < MIN_TOKEN_BYTES {
            return Err(KeySafetyError::TokenTooShort);
        }
        if value.len() > MAX_TOKEN_BYTES {
            return Err(KeySafetyError::TokenTooLong);
        }
        if !value.iter().all(|byte| (0x21..=0x7e).contains(byte)) {
            return Err(KeySafetyError::TokenNotHeaderSafe);
        }
        Ok(Self(value.to_vec()))
    }

    /// Key bytes for the one header writer. Never format the result.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Key length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always false: a token is never empty by construction.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

/// Why a key value or route was refused. Carries no key payload.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeySafetyError {
    /// The key value is shorter than [`MIN_TOKEN_BYTES`].
    TokenTooShort,
    /// The key value is longer than [`MAX_TOKEN_BYTES`].
    TokenTooLong,
    /// The key value holds bytes outside printable ASCII.
    TokenNotHeaderSafe,
    /// The player account label is empty, too long or not printable.
    BadAccount,
    /// A relay endpoint would carry the player token in plaintext.
    PlaintextRelay,
    /// The relay endpoint did not parse.
    BadEndpoint(EndpointError),
    /// The OS store holds no key for this account.
    MissingPlayerKey,
    /// The OS store could not answer.
    Store(KeyStoreError),
}

impl fmt::Display for KeySafetyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            KeySafetyError::TokenTooShort => "provider key is too short",
            KeySafetyError::TokenTooLong => "provider key is too long",
            KeySafetyError::TokenNotHeaderSafe => "provider key is not header-safe",
            KeySafetyError::BadAccount => "player key account label is unusable",
            KeySafetyError::PlaintextRelay => {
                "relay must be loopback or TLS: a plaintext relay leaks the player token"
            }
            KeySafetyError::BadEndpoint(_) => "relay endpoint is invalid",
            KeySafetyError::MissingPlayerKey => "no player key is stored by the OS",
            KeySafetyError::Store(_) => "OS key store is unavailable",
        };
        f.write_str(text)
    }
}

impl std::error::Error for KeySafetyError {}

/// Why the OS store could not answer. Carries no key payload.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyStoreError {
    /// No credential backend is available on this platform or session.
    Unavailable,
    /// The backend refused the operation (locked or denied).
    Denied,
}

impl fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            KeyStoreError::Unavailable => "OS key store unavailable",
            KeyStoreError::Denied => "OS key store denied the operation",
        };
        f.write_str(text)
    }
}

impl std::error::Error for KeyStoreError {}

/// Player-owned key storage backed by the operating system: Windows
/// Credential Manager, macOS Keychain or the freedesktop Secret Service.
/// The app shell supplies the backend; this crate never writes a plaintext
/// key file and never puts a key in an account label.
pub trait OsKeyStore {
    /// Load the key for `account`, or `None` when the player stored none.
    fn load(&self, account: &str) -> Result<Option<SecretToken>, KeyStoreError>;
    /// Store or replace the key for `account`.
    fn save(&mut self, account: &str, token: &SecretToken) -> Result<(), KeyStoreError>;
    /// Forget the key for `account`.
    fn erase(&mut self, account: &str) -> Result<(), KeyStoreError>;
}

/// In-memory store for tests and headless runs. It is not persistent and
/// never touches the disk, so a key cannot leak through a file.
#[derive(Default)]
pub struct MemoryKeyStore {
    entries: HashMap<String, SecretToken>,
}

impl fmt::Debug for MemoryKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryKeyStore")
            .field("accounts", &self.entries.len())
            .finish()
    }
}

impl MemoryKeyStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of stored accounts.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no account is stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl OsKeyStore for MemoryKeyStore {
    fn load(&self, account: &str) -> Result<Option<SecretToken>, KeyStoreError> {
        Ok(self.entries.get(account).cloned())
    }

    fn save(&mut self, account: &str, token: &SecretToken) -> Result<(), KeyStoreError> {
        if !account_ok(account) {
            return Err(KeyStoreError::Denied);
        }
        self.entries.insert(account.to_owned(), token.clone());
        Ok(())
    }

    fn erase(&mut self, account: &str) -> Result<(), KeyStoreError> {
        self.entries.remove(account);
        Ok(())
    }
}

/// Where a shipped game may get a key. The set is closed: there is no
/// inline or embedded variant, so a key cannot be configured into a build.
/// Direct variant construction is allowed by Rust, so [`resolve_key`]
/// revalidates every route.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum KeyRoute {
    /// The player's own key, held by the OS store under a public label.
    PlayerOsStore {
        /// Public account label; never the key itself.
        account: String,
    },
    /// An owner-run relay: the relay holds the key, the player holds none.
    OwnerRelay {
        /// Relay URL; must be loopback or TLS.
        endpoint: String,
    },
    /// The shipped local model: no key and no network.
    LocalModel,
}

impl KeyRoute {
    /// Route to the player's OS-store key under `account`.
    pub fn player_key(account: &str) -> Result<Self, KeySafetyError> {
        if !account_ok(account) {
            return Err(KeySafetyError::BadAccount);
        }
        Ok(KeyRoute::PlayerOsStore {
            account: account.to_owned(),
        })
    }

    /// Route through an owner-run relay. A plaintext off-loopback endpoint
    /// would send the player token in the clear, so it is refused.
    pub fn owner_relay(endpoint: &str) -> Result<Self, KeySafetyError> {
        let route = KeyRoute::OwnerRelay {
            endpoint: endpoint.to_owned(),
        };
        route.validate()?;
        Ok(route)
    }

    /// Route to the shipped local model, which needs no key.
    pub const fn local_model() -> Self {
        KeyRoute::LocalModel
    }

    /// Re-check a route, including one built by a struct literal.
    pub fn validate(&self) -> Result<(), KeySafetyError> {
        match self {
            KeyRoute::PlayerOsStore { account } => {
                if account_ok(account) {
                    Ok(())
                } else {
                    Err(KeySafetyError::BadAccount)
                }
            }
            KeyRoute::OwnerRelay { endpoint } => {
                let parsed = parse_endpoint(endpoint).map_err(KeySafetyError::BadEndpoint)?;
                if parsed.tls() || is_loopback(parsed.host()) {
                    Ok(())
                } else {
                    Err(KeySafetyError::PlaintextRelay)
                }
            }
            KeyRoute::LocalModel => Ok(()),
        }
    }

    /// Fixed label for logs: names the route kind, never the account or URL.
    pub const fn label(&self) -> &'static str {
        match self {
            KeyRoute::PlayerOsStore { .. } => "player-os-store",
            KeyRoute::OwnerRelay { .. } => "owner-relay",
            KeyRoute::LocalModel => "local-model",
        }
    }
}

fn account_ok(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= MAX_ACCOUNT_LEN
        && !account.chars().any(char::is_control)
}

/// What the adapter should use for one call. The token variant prints redacted.
#[derive(Clone, PartialEq, Eq)]
pub enum KeyUse {
    /// Bearer token from the player's OS store.
    PlayerToken(SecretToken),
    /// Relay endpoint; the token stays on the relay.
    OwnerRelay {
        /// Validated relay URL.
        endpoint: String,
    },
    /// Shipped local model, no credential.
    LocalModel,
}

impl fmt::Debug for KeyUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyUse::PlayerToken(_) => f.write_str("PlayerToken(<redacted>)"),
            KeyUse::OwnerRelay { endpoint } => f
                .debug_struct("OwnerRelay")
                .field("endpoint", endpoint)
                .finish(),
            KeyUse::LocalModel => f.write_str("LocalModel"),
        }
    }
}

/// Resolve one route into what the call needs. Missing keys fail closed;
/// there is no fallback to an embedded key or a plaintext file.
pub fn resolve_key(route: &KeyRoute, store: &dyn OsKeyStore) -> Result<KeyUse, KeySafetyError> {
    route.validate()?;
    match route {
        KeyRoute::PlayerOsStore { account } => {
            let Some(token) = store.load(account).map_err(KeySafetyError::Store)? else {
                return Err(KeySafetyError::MissingPlayerKey);
            };
            Ok(KeyUse::PlayerToken(token))
        }
        KeyRoute::OwnerRelay { endpoint } => Ok(KeyUse::OwnerRelay {
            endpoint: endpoint.clone(),
        }),
        KeyRoute::LocalModel => Ok(KeyUse::LocalModel),
    }
}

/// Key-shaped patterns the package scan refuses.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyKind {
    /// `sk-` provider key (OpenAI, Anthropic and compatible vendors).
    ProviderKey,
    /// AWS access key id.
    AwsAccessKey,
    /// GitHub token.
    GithubToken,
    /// Slack token.
    SlackToken,
    /// Stripe live secret key.
    StripeLiveKey,
    /// Google API key.
    GoogleApiKey,
    /// PEM private-key header.
    PrivateKeyPem,
    /// Long high-entropy token with no known prefix.
    HighEntropyToken,
}

impl KeyKind {
    /// Fixed label for scan notices.
    pub const fn label(self) -> &'static str {
        match self {
            KeyKind::ProviderKey => "provider-key",
            KeyKind::AwsAccessKey => "aws-access-key",
            KeyKind::GithubToken => "github-token",
            KeyKind::SlackToken => "slack-token",
            KeyKind::StripeLiveKey => "stripe-live-key",
            KeyKind::GoogleApiKey => "google-api-key",
            KeyKind::PrivateKeyPem => "private-key",
            KeyKind::HighEntropyToken => "high-entropy-token",
        }
    }
}

/// One finding: where and what, never the matched value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyFinding {
    /// Package-relative path.
    pub path: PathBuf,
    /// One-based line number.
    pub line: u32,
    /// Which pattern matched.
    pub kind: KeyKind,
}

/// Bounds for one package scan. Unscanned entries make the report unclean.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ScanLimits {
    /// Largest file the scan reads; a bigger file is left unscanned.
    pub max_file_bytes: u64,
    /// Largest total bytes the scan reads.
    pub max_total_bytes: u64,
    /// Largest number of files the scan opens.
    pub max_files: usize,
    /// Largest number of findings the scan collects.
    pub max_findings: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_files: 20_000,
            max_findings: 32,
        }
    }
}

/// Outcome of one package scan. `is_clean` requires every file scanned, no
/// findings and no truncation, so a skipped file never reads as a pass.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct KeyScanReport {
    /// Files read.
    pub files: usize,
    /// Bytes read.
    pub bytes_scanned: u64,
    /// Key-shaped findings, capped by [`ScanLimits::max_findings`].
    pub findings: Vec<KeyFinding>,
    /// Files or directories the scan could not or would not read.
    pub refused: Vec<PathBuf>,
    /// True when a bound stopped the scan before the tree was covered.
    pub truncated: bool,
}

impl KeyScanReport {
    /// A release may ship only when nothing was found and nothing was left
    /// unscanned.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.refused.is_empty() && !self.truncated
    }

    /// Ring-ready one-line summary. Names counts and the verdict only.
    pub fn notice(&self) -> String {
        format!(
            "package key scan: files={} bytes={} findings={} unscanned={} {}",
            self.files,
            self.bytes_scanned,
            self.findings.len(),
            self.refused.len(),
            if self.is_clean() { "clean" } else { "refused" }
        )
    }
}

/// Why a scan could not start or finish a directory listing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanError {
    /// The root is not a directory.
    NotADirectory,
    /// The root could not be read.
    RootUnreadable(std::io::ErrorKind),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            ScanError::NotADirectory => "package scan root is not a directory",
            ScanError::RootUnreadable(_) => "package scan root is unreadable",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ScanError {}

/// Scan one package tree for key-shaped strings. VCS metadata and build
/// output directories are skipped; symlinks, special files, oversize files
/// and unreadable entries are recorded in `refused`, which fails the run.
pub fn scan_package(root: &Path, limits: ScanLimits) -> Result<KeyScanReport, ScanError> {
    let metadata = fs::metadata(root).map_err(|error| ScanError::RootUnreadable(error.kind()))?;
    if !metadata.is_dir() {
        return Err(ScanError::NotADirectory);
    }
    let mut report = KeyScanReport::default();
    if limits.max_files == 0 || limits.max_total_bytes == 0 || limits.max_findings == 0 {
        report.truncated = true;
        return Ok(report);
    }
    let mut directories = VecDeque::new();
    directories.push_back(root.to_path_buf());
    'walk: while let Some(directory) = directories.pop_front() {
        if report.truncated {
            break;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => {
                report.refused.push(relative_path(&directory, root));
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => children.push(entry.path()),
                Err(_) => report.refused.push(relative_path(&directory, root)),
            }
        }
        children.sort();
        for path in children {
            if report.truncated {
                break 'walk;
            }
            if report.findings.len() >= limits.max_findings {
                report.truncated = true;
                break 'walk;
            }
            let file_type = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata.file_type(),
                Err(_) => {
                    report.refused.push(relative_path(&path, root));
                    continue;
                }
            };
            if file_type.is_dir() {
                if !is_skipped_dir(&path) {
                    directories.push_back(path);
                }
            } else if file_type.is_file() {
                if report.files >= limits.max_files {
                    report.truncated = true;
                    break 'walk;
                }
                scan_file(&path, root, &limits, &mut report);
            } else {
                // Symlinks and special entries are never followed.
                report.refused.push(relative_path(&path, root));
            }
        }
    }
    Ok(report)
}

fn scan_file(path: &Path, root: &Path, limits: &ScanLimits, report: &mut KeyScanReport) {
    let relative = relative_path(path, root);
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(_) => {
            report.refused.push(relative);
            return;
        }
    };
    if size > limits.max_file_bytes {
        report.refused.push(relative);
        return;
    }
    if report.bytes_scanned.saturating_add(size) > limits.max_total_bytes {
        report.truncated = true;
        return;
    }
    match fs::read(path) {
        Ok(bytes) => {
            report.files += 1;
            let read = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            report.bytes_scanned = report.bytes_scanned.saturating_add(read);
            let room = limits.max_findings.saturating_sub(report.findings.len());
            report.findings.extend(scan_bytes(&relative, &bytes, room));
        }
        Err(_) => report.refused.push(relative),
    }
}

fn relative_path(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn is_skipped_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | ".hg" | ".svn" | "target" | "node_modules")
    )
}

fn scan_bytes(relative: &Path, bytes: &[u8], max_findings: usize) -> Vec<KeyFinding> {
    let text = String::from_utf8_lossy(bytes);
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if findings.len() >= max_findings {
            break;
        }
        if let Some(kind) = line_kind(line) {
            findings.push(KeyFinding {
                path: relative.to_path_buf(),
                line: u32::try_from(index + 1).unwrap_or(u32::MAX),
                kind,
            });
        }
    }
    findings
}

/// PEM armour begins with this text.
const PEM_BEGIN: &str = "-----BEGIN ";
/// PEM armour ends with this text.
const PEM_END: &str = "PRIVATE KEY-----";

fn line_kind(line: &str) -> Option<KeyKind> {
    if line.contains(PEM_BEGIN) && line.contains(PEM_END) {
        return Some(KeyKind::PrivateKeyPem);
    }
    line.split(is_word_break)
        .filter(|word| word.len() >= 8)
        .find_map(word_kind)
}

fn is_word_break(character: char) -> bool {
    !(character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '+' | '/' | '=' | '.'))
}

fn word_kind(word: &str) -> Option<KeyKind> {
    if word.starts_with("AKIA") && word.len() == 20 && entropy(word) > 3.0 {
        return Some(KeyKind::AwsAccessKey);
    }
    if word.starts_with("github_pat_") && word.len() >= 50 && entropy(word) > 3.0 {
        return Some(KeyKind::GithubToken);
    }
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| word.starts_with(*prefix))
        && word.len() >= 40
        && entropy(word) > 3.0
    {
        return Some(KeyKind::GithubToken);
    }
    if word.len() >= 15 && word.starts_with("xox") && entropy(word) > 3.0 {
        let kind = word.as_bytes().get(3).copied();
        if matches!(kind, Some(b'b' | b'a' | b'p' | b'r' | b's'))
            && word.as_bytes().get(4) == Some(&b'-')
        {
            return Some(KeyKind::SlackToken);
        }
    }
    if word.starts_with("sk_live_") && word.len() >= 24 && entropy(word) > 3.0 {
        return Some(KeyKind::StripeLiveKey);
    }
    if word.starts_with("AIza") && word.len() == 39 && entropy(word) > 3.0 {
        return Some(KeyKind::GoogleApiKey);
    }
    if word.starts_with("sk-") && word.len() >= 23 && entropy(word) > 3.0 {
        return Some(KeyKind::ProviderKey);
    }
    if word.len() >= 32 && entropy(word) >= 4.2 && char_classes(word) >= 3 {
        return Some(KeyKind::HighEntropyToken);
    }
    None
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn usize_to_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

fn entropy(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for byte in text.bytes() {
        counts[usize::from(byte)] += 1;
    }
    let length = usize_to_f64(text.len());
    counts
        .iter()
        .filter(|count| **count > 0)
        .map(|count| {
            let probability = f64::from(*count) / length;
            -probability * probability.log2()
        })
        .sum()
}

fn char_classes(text: &str) -> u8 {
    let mut classes = 0;
    if text.bytes().any(|byte| byte.is_ascii_lowercase()) {
        classes += 1;
    }
    if text.bytes().any(|byte| byte.is_ascii_uppercase()) {
        classes += 1;
    }
    if text.bytes().any(|byte| byte.is_ascii_digit()) {
        classes += 1;
    }
    if text
        .bytes()
        .any(|byte| matches!(byte, b'_' | b'-' | b'+' | b'/' | b'='))
    {
        classes += 1;
    }
    classes
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::super::fallback::{FallbackCause, MAX_NOTICE_LEN};
    use super::super::mock::fixtures;
    use super::*;

    static NEXT_TEMP: AtomicU32 = AtomicU32::new(0);

    fn plan() -> QuotaPlan {
        QuotaPlan::new(
            QuotaLimits {
                requests: 4,
                tokens: 100,
                spend: Some(50),
                rate: None,
            },
            QuotaLimits {
                requests: 2,
                tokens: 60,
                spend: None,
                rate: None,
            },
        )
        .expect("test plan validates")
    }

    fn rate_plan(player_rate: Option<RateLimit>, session_rate: Option<RateLimit>) -> QuotaPlan {
        QuotaPlan::new(
            QuotaLimits {
                requests: 100,
                tokens: 1_000,
                spend: None,
                rate: player_rate,
            },
            QuotaLimits {
                requests: 100,
                tokens: 1_000,
                spend: None,
                rate: session_rate,
            },
        )
        .expect("rate plan validates")
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "e2040-quota-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temp package root");
        root
    }

    fn seeded_key() -> String {
        // Prefix and body are split into short literals on purpose: no
        // complete key and no long high-entropy run sits in this file.
        format!(
            "{}-{}{}{}",
            "sk", "e2040QuotaCanary", "9f2cA1b2C3d4", "E5f6G7h8"
        )
    }

    #[test]
    fn plan_rejects_zero_limits() {
        let zero = QuotaLimits {
            requests: 0,
            tokens: 1,
            spend: None,
            rate: None,
        };
        let good = QuotaLimits {
            requests: 1,
            tokens: 1,
            spend: None,
            rate: None,
        };
        assert_eq!(
            QuotaPlan::new(zero, good),
            Err(QuotaConfigError::ZeroRequests)
        );
        let zero_tokens = QuotaLimits { tokens: 0, ..good };
        assert_eq!(
            QuotaPlan::new(zero_tokens, good),
            Err(QuotaConfigError::ZeroTokens)
        );
        let zero_spend = QuotaLimits {
            spend: Some(0),
            ..good
        };
        assert_eq!(
            QuotaPlan::new(zero_spend, good),
            Err(QuotaConfigError::ZeroSpend)
        );
        let zero_rate = QuotaLimits {
            rate: Some(RateLimit {
                max_requests: 0,
                window_ms: 10,
            }),
            ..good
        };
        assert_eq!(
            QuotaPlan::new(zero_rate, good),
            Err(QuotaConfigError::ZeroRate)
        );
        assert!(QuotaConfigError::ZeroRate.to_string().contains("rate"));
    }

    #[test]
    fn charge_spends_both_scopes_and_reports_usage() {
        let mut book = QuotaBook::new(plan());
        let player = PlayerId(7);
        let session = book.open_session(player).expect("session opens");
        let receipt = book
            .charge(0, player, session, QuotaCharge::new(10, 5))
            .expect("first charge fits");
        assert_eq!(
            receipt.player,
            QuotaUsage {
                requests: 1,
                tokens: 10,
                spend: 5
            }
        );
        assert_eq!(receipt.session.requests, 1);
        assert_eq!(book.player_usage(player), Some(receipt.player));
        assert_eq!(book.session_usage(session), Some(receipt.session));
        book.close_session(session);
        assert_eq!(book.session_usage(session), None);
        assert_eq!(book.player_usage(player), Some(receipt.player));
    }

    #[test]
    fn player_token_and_spend_overruns_refuse() {
        let mut book = QuotaBook::new(plan());
        let player = PlayerId(1);
        let session = book.open_session(player).expect("session opens");
        // Session token cap is 60, player token cap is 100.
        book.charge(0, player, session, QuotaCharge::new(50, 5))
            .expect("first charge fits");
        let token_refusal = book
            .charge(1, player, session, QuotaCharge::new(11, 5))
            .expect_err("session tokens refuse");
        assert_eq!(token_refusal.scope, QuotaScope::Session);
        assert_eq!(token_refusal.limit, QuotaLimit::Tokens);
        assert_eq!(token_refusal.allowed, 60);
        assert_eq!(token_refusal.used, 50);

        let spend_session = book.open_session(PlayerId(2)).expect("second player");
        let spend_refusal = book
            .charge(2, PlayerId(2), spend_session, QuotaCharge::new(1, 51))
            .expect_err("player spend refuses");
        assert_eq!(spend_refusal.scope, QuotaScope::Player);
        assert_eq!(spend_refusal.limit, QuotaLimit::Spend);
        assert_eq!(spend_refusal.allowed, 50);
        assert_eq!(spend_refusal.used, 0);
    }

    #[test]
    fn session_request_overrun_refuses_after_two() {
        let mut book = QuotaBook::new(plan());
        let player = PlayerId(3);
        let session = book.open_session(player).expect("session opens");
        book.charge(0, player, session, QuotaCharge::new(1, 1))
            .expect("first");
        book.charge(1, player, session, QuotaCharge::new(1, 1))
            .expect("second");
        let refusal = book
            .charge(2, player, session, QuotaCharge::new(1, 1))
            .expect_err("session request cap refuses");
        assert_eq!(refusal.scope, QuotaScope::Session);
        assert_eq!(refusal.limit, QuotaLimit::Requests);
        assert_eq!(refusal.allowed, 2);
        assert_eq!(refusal.used, 2);
        // The refused charge moved neither scope.
        assert_eq!(book.player_usage(player).expect("player row").requests, 2);
        assert_eq!(
            book.session_usage(session).expect("session row").requests,
            2
        );
    }

    #[test]
    fn rate_window_rolls_on_the_caller_clock() {
        let rate = RateLimit {
            max_requests: 2,
            window_ms: 1_000,
        };
        let mut book = QuotaBook::new(rate_plan(None, Some(rate)));
        let player = PlayerId(4);
        let session = book.open_session(player).expect("session opens");
        book.charge(0, player, session, QuotaCharge::new(1, 0))
            .expect("first in window");
        book.charge(100, player, session, QuotaCharge::new(1, 0))
            .expect("second in window");
        let refusal = book
            .charge(999, player, session, QuotaCharge::new(1, 0))
            .expect_err("third in window refuses");
        assert_eq!(refusal.limit, QuotaLimit::Rate);
        assert_eq!(refusal.allowed, 2);
        assert_eq!(refusal.used, 2);
        // A backwards clock never opens a fresh window.
        assert!(book
            .charge(50, player, session, QuotaCharge::new(1, 0))
            .is_err());
        book.charge(1_000, player, session, QuotaCharge::new(1, 0))
            .expect("window rolls at its boundary");
    }

    #[test]
    fn session_belongs_to_one_player() {
        let mut book = QuotaBook::new(plan());
        let owner = PlayerId(5);
        let other = PlayerId(6);
        let owned = book.open_session(owner).expect("owner session");
        book.open_session(other).expect("other session");
        let refusal = book
            .charge(0, owner, owned, QuotaCharge::new(1, 1))
            .expect("owner charges");
        assert_eq!(refusal.session.requests, 1);
        // Charging another player's session id is refused, not re-owned.
        let foreign = SessionId(1);
        let refused = book
            .charge(0, other, foreign, QuotaCharge::new(1, 1))
            .expect_err("foreign session refuses");
        assert_eq!(refused.scope, QuotaScope::Session);
    }

    #[test]
    fn overrun_falls_back_to_scripted_with_a_notice() {
        let mut book = QuotaBook::new(rate_plan(
            None,
            Some(RateLimit {
                max_requests: 1,
                window_ms: 1_000,
            }),
        ));
        let player = PlayerId(9);
        let session = book.open_session(player).expect("session opens");
        book.charge(0, player, session, QuotaCharge::new(1, 0))
            .expect("first");
        let refusal = book
            .charge(10, player, session, QuotaCharge::new(1, 0))
            .expect_err("rate refuses");
        let request = fixtures::request(21, 3);
        let policy = fixtures::policy();
        let served = refusal
            .scripted(&request, &policy)
            .expect("scripted path is open");
        assert_eq!(served.cause, FallbackCause::Refused);
        assert_eq!(served.notice.error, AiError::QuotaAuth);
        assert!(
            served.notice.message.contains("quota"),
            "notice names quota: {}",
            served.notice.message
        );
        assert!(
            served.notice.message.contains("scripted"),
            "notice names the scripted path: {}",
            served.notice.message
        );
        assert!(served.notice.message.len() <= MAX_NOTICE_LEN);
        assert!(
            served
                .proposal
                .diagnostics
                .iter()
                .any(|line| line.contains("quota overrun")),
            "proposal records the overrun"
        );
        assert_eq!(served.proposal.request, request.id);
    }

    #[test]
    fn secret_store_roundtrip_never_prints_the_key() {
        let mut store = MemoryKeyStore::new();
        let key = "e2040-test-token-0123456789";
        let token = SecretToken::new(key).expect("test token is header-safe");
        store.save("player/42", &token).expect("memory store saves");
        assert_eq!(store.len(), 1);
        let route = KeyRoute::player_key("player/42").expect("account validates");
        let used = resolve_key(&route, &store).expect("stored key resolves");
        match &used {
            KeyUse::PlayerToken(found) => assert_eq!(found.expose(), token.expose()),
            other => panic!("expected a player token, got {other:?}"),
        }
        let printed = format!("{token:?} {route:?} {used:?} {store:?}");
        assert!(!printed.contains(key), "key leaked into debug output");
        store.erase("player/42").expect("erase");
        assert!(store.is_empty());
        assert_eq!(
            resolve_key(&route, &store),
            Err(KeySafetyError::MissingPlayerKey)
        );
    }

    #[test]
    fn keys_must_be_header_safe() {
        assert_eq!(
            SecretToken::new("short"),
            Err(KeySafetyError::TokenTooShort)
        );
        let with_newline = "e2040-token\r\nX-Injected: yes";
        assert_eq!(
            SecretToken::new(with_newline),
            Err(KeySafetyError::TokenNotHeaderSafe)
        );
        let long = "a".repeat(MAX_TOKEN_BYTES + 1);
        assert_eq!(SecretToken::new(long), Err(KeySafetyError::TokenTooLong));
        let good = SecretToken::new("e2040-token-0123456789").expect("safe token");
        assert_eq!(good.len(), 22);
        assert!(!good.is_empty());
    }

    #[test]
    fn relay_route_refuses_plaintext_off_loopback() {
        assert!(KeyRoute::owner_relay("http://127.0.0.1:8080/v1").is_ok());
        assert!(KeyRoute::owner_relay("http://localhost:8080").is_ok());
        assert_eq!(
            KeyRoute::owner_relay("http://relay.example.com/v1"),
            Err(KeySafetyError::PlaintextRelay)
        );
        // The variant is constructible, but resolve revalidates it.
        let direct = KeyRoute::OwnerRelay {
            endpoint: String::from("http://relay.example.com/v1"),
        };
        assert_eq!(
            resolve_key(&direct, &MemoryKeyStore::new()),
            Err(KeySafetyError::PlaintextRelay)
        );
        #[cfg(feature = "tls")]
        assert!(KeyRoute::owner_relay("https://relay.example.com/v1").is_ok());
        #[cfg(not(feature = "tls"))]
        assert!(matches!(
            KeyRoute::owner_relay("https://relay.example.com/v1"),
            Err(KeySafetyError::BadEndpoint(EndpointError::Scheme))
        ));
        assert_eq!(KeyRoute::player_key(""), Err(KeySafetyError::BadAccount));
        assert_eq!(KeyRoute::local_model().label(), "local-model");
        assert_eq!(
            resolve_key(&KeyRoute::local_model(), &MemoryKeyStore::new()),
            Ok(KeyUse::LocalModel)
        );
    }

    #[test]
    fn seeded_key_in_the_package_fails_the_scan() {
        let root = temp_root("seeded");
        fs::create_dir_all(root.join("assets")).expect("assets dir");
        fs::write(root.join("assets/readme.txt"), "no keys here\n").expect("clean file");
        let key = seeded_key();
        fs::write(
            root.join("assets/leak.txt"),
            format!("line one\nline two\nconst KEY: &str = \"{key}\";\n"),
        )
        .expect("seeded file");
        let report = scan_package(&root, ScanLimits::default()).expect("scan runs");
        assert!(!report.is_clean(), "seeded key must fail the scan");
        assert_eq!(report.findings.len(), 1, "one finding: {report:?}");
        let finding = &report.findings[0];
        assert_eq!(
            finding.path.to_string_lossy().replace('\\', "/"),
            "assets/leak.txt"
        );
        assert_eq!(finding.line, 3);
        assert_eq!(finding.kind, KeyKind::ProviderKey);
        let printed = format!("{report:?} {}", report.notice());
        assert!(!printed.contains(&key), "report leaked the key value");
        assert!(report.notice().contains("refused"));
        fs::remove_dir_all(&root).expect("temp cleanup");
    }

    #[test]
    fn clean_package_passes_and_digests_do_not_false_positive() {
        let root = temp_root("clean");
        fs::create_dir_all(root.join(".git")).expect(".git dir");
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join(".git/config"), seeded_key()).expect("vcs file is skipped");
        fs::write(
            root.join("README.md"),
            format!("digest {}\nplain words\n", "0123456789abcdef".repeat(4)),
        )
        .expect("readme");
        fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("main");
        let report = scan_package(&root, ScanLimits::default()).expect("scan runs");
        assert!(report.is_clean(), "clean tree: {report:?}");
        assert_eq!(report.files, 2, "vcs metadata is skipped");
        assert!(report.findings.is_empty());
        assert!(report.notice().contains("clean"));
        fs::remove_dir_all(&root).expect("temp cleanup");
    }

    #[test]
    fn unscanned_files_fail_closed() {
        let root = temp_root("oversize");
        fs::write(root.join("small.txt"), "plain\n").expect("small file");
        fs::write(root.join("big.txt"), "0123456789abcdef\n").expect("big file");
        let limits = ScanLimits {
            max_file_bytes: 8,
            ..ScanLimits::default()
        };
        let report = scan_package(&root, limits).expect("scan runs");
        assert!(!report.is_clean(), "oversize file is unscanned");
        assert_eq!(report.files, 1);
        assert_eq!(report.refused.len(), 1);
        assert!(report
            .refused
            .iter()
            .any(|path| path.to_string_lossy().contains("big.txt")));
        fs::remove_dir_all(&root).expect("temp cleanup");
    }

    #[test]
    fn private_key_pem_is_found() {
        let root = temp_root("pem");
        let begin = "-----BEGIN RSA ";
        let end = "PRIVATE KEY-----";
        fs::write(root.join("key.pem"), format!("{begin}{end}")).expect("pem file");
        let report = scan_package(&root, ScanLimits::default()).expect("scan runs");
        assert!(!report.is_clean());
        assert_eq!(report.findings[0].kind, KeyKind::PrivateKeyPem);
        fs::remove_dir_all(&root).expect("temp cleanup");
    }
}
