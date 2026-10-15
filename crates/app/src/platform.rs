//! Platform contract: store achievements, stats, cloud saves, presence,
//! overlay hooks, lobbies and Workshop mods behind one trait.
//!
//! Owner: the app composition layer. Units: achievement ids and stat names
//! are short ASCII rows, cloud slots are `0..MAX_CLOUD_SLOTS`, payloads are
//! bytes capped at [`MAX_CLOUD_BYTES`]. Determinism: callbacks pump once per
//! frame outside the fixed tick and never enter sim hashes; the mock stores
//! in `BTreeMap`/`BTreeSet` order only. Errors: [`PlatformError`] per call.

use std::collections::{BTreeMap, BTreeSet};

/// Longest accepted achievement id, stat name, lobby user or mod name.
pub const MAX_ID_LEN: usize = 64;
/// Largest cloud payload or Workshop payload in bytes (1 MiB).
pub const MAX_CLOUD_BYTES: usize = 1_048_576;
/// Cloud slots addressable per title.
pub const MAX_CLOUD_SLOTS: u8 = 8;
/// Longest rich-presence state or details line.
pub const MAX_PRESENCE_LEN: usize = 128;
/// Largest lobby roster or Workshop subscription list tracked locally.
pub const MAX_LOBBY_USERS: usize = 64;

/// Platform call failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformError {
    /// Id or name is empty, too long or outside `[A-Za-z0-9._-]`.
    BadId,
    /// Cloud slot is at or above [`MAX_CLOUD_SLOTS`].
    BadSlot(u8),
    /// Payload exceeds [`MAX_CLOUD_BYTES`].
    TooLarge,
    /// Achievement id is well-formed but not in the title catalog.
    UnknownAchievement,
    /// Stat name is well-formed but not tracked by this title.
    UnknownStat,
    /// The backend is present but the cloud store is unreachable.
    CloudUnavailable,
    /// No backend is configured (the null platform, or Steam absent).
    NotConfigured,
    /// Invite or publish raced a full roster or subscription cap.
    SlotFull,
}

impl core::fmt::Display for PlatformError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::BadId => write!(f, "bad platform id"),
            Self::BadSlot(slot) => write!(f, "bad cloud slot ({slot})"),
            Self::TooLarge => write!(f, "platform payload exceeds size bound"),
            Self::UnknownAchievement => write!(f, "unknown achievement"),
            Self::UnknownStat => write!(f, "unknown stat"),
            Self::CloudUnavailable => write!(f, "cloud store unreachable"),
            Self::NotConfigured => write!(f, "no platform backend configured"),
            Self::SlotFull => write!(f, "platform lobby or mod list full"),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Opaque lobby handle issued by [`Platform::create_lobby`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LobbyId(pub u64);

/// Opaque Workshop item handle from [`Platform::publish_workshop_item`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WorkshopId(pub u64);

/// Rich-presence lines shown on the overlay; plain data, never hashed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Presence {
    /// Short status line (`<= MAX_PRESENCE_LEN` bytes).
    pub state: String,
    /// Longer details line (`<= MAX_PRESENCE_LEN` bytes).
    pub details: String,
}

impl Presence {
    /// Build presence lines; over-long lines fail with the culprit named.
    pub fn new(state: &str, details: &str) -> Result<Self, PlatformError> {
        if state.len() > MAX_PRESENCE_LEN || details.len() > MAX_PRESENCE_LEN {
            return Err(PlatformError::TooLarge);
        }
        Ok(Self {
            state: state.to_string(),
            details: details.to_string(),
        })
    }
}

/// One recorded backend call, in call order (mock only).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlatformCall {
    /// Achievement unlock attempt, id carried.
    UnlockAchievement(String),
    /// Stat record, name and value carried.
    RecordStat(String, i64),
    /// Cloud store, slot and byte length carried.
    StoreCloud(u8, usize),
    /// Cloud load, slot carried.
    LoadCloud(u8),
    /// Presence update, lines carried.
    SetPresence(Presence),
    /// Callback pump for one frame.
    Pump,
    /// Lobby created, handle carried.
    CreateLobby(LobbyId),
    /// Overlay invite sent, lobby and user carried.
    InviteToLobby(LobbyId, String),
    /// Workshop item published, handle and byte length carried.
    PublishWorkshop(WorkshopId, usize),
}

/// Store/overlay contract: achievements, stats, cloud saves, rich presence,
/// overlay hooks, lobbies and Workshop mods. All methods are cold
/// presentation-path calls; `pump` runs once per frame outside the tick.
pub trait Platform {
    /// Backend name for receipts (`"null"`, `"mock"`, `"steam"`).
    fn name(&self) -> &'static str;
    /// True when the overlay hook can show invites and presence.
    fn overlay_available(&self) -> bool {
        false
    }
    /// Unlock an achievement; `Ok(true)` is newly unlocked, `Ok(false)` is
    /// a repeat. Unknown or malformed ids fail loudly.
    fn unlock_achievement(&mut self, id: &str) -> Result<bool, PlatformError>;
    /// Record a tracked stat value; unknown or malformed names fail loudly.
    fn record_stat(&mut self, name: &str, value: i64) -> Result<(), PlatformError>;
    /// Last recorded stat value; unknown names fail loudly.
    fn stat(&self, name: &str) -> Result<i64, PlatformError>;
    /// Store a cloud payload; oversize payloads and bad slots fail loudly.
    fn store_cloud(&mut self, slot: u8, data: &[u8]) -> Result<(), PlatformError>;
    /// Load a cloud payload; missing slots fail loudly.
    fn load_cloud(&self, slot: u8) -> Result<Vec<u8>, PlatformError>;
    /// Stage rich-presence lines for the next pump.
    fn set_presence(&mut self, presence: &Presence);
    /// Last staged presence lines.
    fn presence(&self) -> &Presence;
    /// Pump backend callbacks once per frame, outside the fixed tick.
    fn pump(&mut self);
    /// Open a lobby for overlay invites.
    fn create_lobby(&mut self) -> Result<LobbyId, PlatformError>;
    /// Invite a user to a lobby through the overlay hook.
    fn invite_to_lobby(&mut self, lobby: LobbyId, user: &str) -> Result<(), PlatformError>;
    /// Publish a data-mod payload; code mods stay out by policy.
    fn publish_workshop_item(
        &mut self,
        name: &str,
        payload: &[u8],
    ) -> Result<WorkshopId, PlatformError>;
    /// Locally tracked Workshop subscriptions, in id order.
    fn subscribed_workshop_items(&self) -> Vec<WorkshopId>;
}

/// Id rows: `1..=MAX_ID_LEN` bytes of `[A-Za-z0-9._-]`.
pub fn check_id(id: &str) -> Result<(), PlatformError> {
    if id.is_empty()
        || id.len() > MAX_ID_LEN
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(PlatformError::BadId);
    }
    Ok(())
}

/// Cloud slot rows: below [`MAX_CLOUD_SLOTS`].
pub fn check_slot(slot: u8) -> Result<(), PlatformError> {
    if slot >= MAX_CLOUD_SLOTS {
        return Err(PlatformError::BadSlot(slot));
    }
    Ok(())
}

/// Null backend: every call succeeds and does nothing, so the arena runs
/// identically with no store present. Cloud reads report [`NotConfigured`].
#[derive(Debug, Default)]
pub struct NullPlatform {
    /// Last staged presence lines.
    presence: Presence,
}

impl NullPlatform {
    /// Default null backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Platform for NullPlatform {
    fn name(&self) -> &'static str {
        "null"
    }
    fn unlock_achievement(&mut self, id: &str) -> Result<bool, PlatformError> {
        check_id(id)?;
        Ok(false)
    }
    fn record_stat(&mut self, name: &str, _value: i64) -> Result<(), PlatformError> {
        check_id(name)?;
        Ok(())
    }
    fn stat(&self, _name: &str) -> Result<i64, PlatformError> {
        Err(PlatformError::NotConfigured)
    }
    fn store_cloud(&mut self, slot: u8, data: &[u8]) -> Result<(), PlatformError> {
        check_slot(slot)?;
        if data.len() > MAX_CLOUD_BYTES {
            return Err(PlatformError::TooLarge);
        }
        Ok(())
    }
    fn load_cloud(&self, _slot: u8) -> Result<Vec<u8>, PlatformError> {
        Err(PlatformError::NotConfigured)
    }
    fn set_presence(&mut self, presence: &Presence) {
        self.presence = presence.clone();
    }
    fn presence(&self) -> &Presence {
        &self.presence
    }
    fn pump(&mut self) {}
    fn create_lobby(&mut self) -> Result<LobbyId, PlatformError> {
        Err(PlatformError::NotConfigured)
    }
    fn invite_to_lobby(&mut self, _lobby: LobbyId, _user: &str) -> Result<(), PlatformError> {
        Err(PlatformError::NotConfigured)
    }
    fn publish_workshop_item(
        &mut self,
        _name: &str,
        _payload: &[u8],
    ) -> Result<WorkshopId, PlatformError> {
        Err(PlatformError::NotConfigured)
    }
    fn subscribed_workshop_items(&self) -> Vec<WorkshopId> {
        Vec::new()
    }
}

/// Mock backend: records every call in order over `BTreeMap`/`BTreeSet`
/// stores, so tests assert exact call sequences and replayed sessions agree.
#[derive(Debug, Default)]
pub struct MockPlatform {
    /// Every call so far, in call order.
    pub calls: Vec<PlatformCall>,
    /// Unlocked achievement ids.
    pub achievements: BTreeSet<String>,
    /// Tracked stat values.
    pub stats: BTreeMap<String, i64>,
    /// Cloud payloads by slot.
    pub cloud: BTreeMap<u8, Vec<u8>>,
    /// Lobby rosters by handle.
    pub lobbies: BTreeMap<LobbyId, Vec<String>>,
    /// Workshop payloads by handle.
    pub workshop: BTreeMap<WorkshopId, Vec<u8>>,
    /// Last staged presence lines.
    pub presence: Presence,
    /// Next lobby handle.
    pub next_lobby: u64,
    /// Next Workshop handle.
    pub next_workshop: u64,
}

impl MockPlatform {
    /// Empty mock backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Platform for MockPlatform {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn overlay_available(&self) -> bool {
        true
    }
    fn unlock_achievement(&mut self, id: &str) -> Result<bool, PlatformError> {
        check_id(id)?;
        self.calls
            .push(PlatformCall::UnlockAchievement(id.to_string()));
        Ok(self.achievements.insert(id.to_string()))
    }
    fn record_stat(&mut self, name: &str, value: i64) -> Result<(), PlatformError> {
        check_id(name)?;
        self.calls
            .push(PlatformCall::RecordStat(name.to_string(), value));
        self.stats.insert(name.to_string(), value);
        Ok(())
    }
    fn stat(&self, name: &str) -> Result<i64, PlatformError> {
        check_id(name)?;
        self.stats
            .get(name)
            .copied()
            .ok_or(PlatformError::UnknownStat)
    }
    fn store_cloud(&mut self, slot: u8, data: &[u8]) -> Result<(), PlatformError> {
        check_slot(slot)?;
        if data.len() > MAX_CLOUD_BYTES {
            return Err(PlatformError::TooLarge);
        }
        self.calls.push(PlatformCall::StoreCloud(slot, data.len()));
        self.cloud.insert(slot, data.to_vec());
        Ok(())
    }
    fn load_cloud(&self, slot: u8) -> Result<Vec<u8>, PlatformError> {
        check_slot(slot)?;
        self.cloud
            .get(&slot)
            .cloned()
            .ok_or(PlatformError::CloudUnavailable)
    }
    fn set_presence(&mut self, presence: &Presence) {
        self.calls.push(PlatformCall::SetPresence(presence.clone()));
        self.presence = presence.clone();
    }
    fn presence(&self) -> &Presence {
        &self.presence
    }
    fn pump(&mut self) {
        self.calls.push(PlatformCall::Pump);
    }
    fn create_lobby(&mut self) -> Result<LobbyId, PlatformError> {
        let id = LobbyId(self.next_lobby);
        self.next_lobby = self.next_lobby.saturating_add(1);
        self.calls.push(PlatformCall::CreateLobby(id));
        self.lobbies.insert(id, Vec::new());
        Ok(id)
    }
    fn invite_to_lobby(&mut self, lobby: LobbyId, user: &str) -> Result<(), PlatformError> {
        check_id(user)?;
        let roster = self.lobbies.get_mut(&lobby).ok_or(PlatformError::BadId)?;
        if roster.len() >= MAX_LOBBY_USERS {
            return Err(PlatformError::SlotFull);
        }
        self.calls
            .push(PlatformCall::InviteToLobby(lobby, user.to_string()));
        roster.push(user.to_string());
        Ok(())
    }
    fn publish_workshop_item(
        &mut self,
        name: &str,
        payload: &[u8],
    ) -> Result<WorkshopId, PlatformError> {
        check_id(name)?;
        if payload.len() > MAX_CLOUD_BYTES {
            return Err(PlatformError::TooLarge);
        }
        let id = WorkshopId(self.next_workshop);
        self.next_workshop = self.next_workshop.saturating_add(1);
        self.calls
            .push(PlatformCall::PublishWorkshop(id, payload.len()));
        self.workshop.insert(id, payload.to_vec());
        Ok(id)
    }
    fn subscribed_workshop_items(&self) -> Vec<WorkshopId> {
        self.workshop.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_records_every_call_in_order() {
        let mut mock = MockPlatform::new();
        let presence = Presence::new("in arena", "final vs rook").expect("presence");
        assert!(mock.unlock_achievement("first_blood").expect("unlock"));
        assert!(!mock.unlock_achievement("first_blood").expect("repeat"));
        mock.record_stat("wins", 3).expect("stat");
        mock.store_cloud(2, b"gen").expect("store");
        mock.set_presence(&presence);
        mock.pump();
        let lobby = mock.create_lobby().expect("lobby");
        mock.invite_to_lobby(lobby, "rook").expect("invite");
        let item = mock.publish_workshop_item("duel", b"data").expect("mod");
        assert_eq!(
            mock.calls,
            vec![
                PlatformCall::UnlockAchievement("first_blood".to_string()),
                PlatformCall::UnlockAchievement("first_blood".to_string()),
                PlatformCall::RecordStat("wins".to_string(), 3),
                PlatformCall::StoreCloud(2, 3),
                PlatformCall::SetPresence(presence.clone()),
                PlatformCall::Pump,
                PlatformCall::CreateLobby(lobby),
                PlatformCall::InviteToLobby(lobby, "rook".to_string()),
                PlatformCall::PublishWorkshop(item, 4),
            ]
        );
        assert_eq!(mock.stat("wins").expect("read"), 3);
        assert_eq!(mock.load_cloud(2).expect("load"), b"gen");
        assert_eq!(mock.presence(), &presence);
        assert_eq!(mock.subscribed_workshop_items(), vec![item]);
    }

    #[test]
    fn malformed_rows_and_bounds_refuse_loudly() {
        let mut mock = MockPlatform::new();
        assert_eq!(
            mock.unlock_achievement("no spaces!"),
            Err(PlatformError::BadId)
        );
        assert_eq!(mock.unlock_achievement(""), Err(PlatformError::BadId));
        assert_eq!(mock.record_stat("", 1), Err(PlatformError::BadId));
        assert_eq!(mock.stat("never.tracked"), Err(PlatformError::UnknownStat));
        assert_eq!(
            mock.store_cloud(MAX_CLOUD_SLOTS, b"x"),
            Err(PlatformError::BadSlot(MAX_CLOUD_SLOTS))
        );
        assert_eq!(
            mock.store_cloud(0, &vec![0u8; MAX_CLOUD_BYTES + 1]),
            Err(PlatformError::TooLarge)
        );
        assert_eq!(mock.load_cloud(7), Err(PlatformError::CloudUnavailable));
        assert_eq!(
            Presence::new(&"s".repeat(MAX_PRESENCE_LEN + 1), ""),
            Err(PlatformError::TooLarge)
        );
        let mut null = NullPlatform::new();
        assert_eq!(null.load_cloud(0), Err(PlatformError::NotConfigured));
        assert_eq!(null.create_lobby(), Err(PlatformError::NotConfigured));
        assert!(!null.overlay_available());
        assert!(mock.overlay_available());
    }

    #[test]
    fn null_backend_leaves_arena_hash_untouched() {
        let config = engine_arena::ArenaConfig {
            seed: 2040,
            ticks: 64,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let tape = engine_arena::FighterTape::default();
        let (_, first) = engine_arena::run_prefix(&config, &tape, 64).expect("run");
        let mut null = NullPlatform::new();
        null.set_presence(&Presence::new("arena", "verify").expect("presence"));
        null.pump();
        let (_, second) = engine_arena::run_prefix(&config, &tape, 64).expect("rerun");
        assert_eq!(first, second);
    }
}
