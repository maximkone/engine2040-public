//! Steamworks backend: achievements, stats, versioned cloud generations and
//! rich presence behind the platform contract, with a null fallback.
//!
//! Owner: the game packaging layer. Units: Steam app ids, `0..8` cloud slots,
//! payload bytes capped at 1 MiB. Determinism: discovery reads env paths and
//! files at probe time only; the fixed tick never waits on Steam and hashes
//! never include backend bytes. Errors: [`SteamError`] per call; without
//! Steam running (or without the `steam` feature) every entry reports
//! [`SteamError::NoBackend`] so callers run the null path.
//!
//! The `steamworks-rs` SDK binding behind a `steam` cargo feature is an
//! orchestrator step (manifest + `deny.toml` row); until it lands this file
//! is the only place that may name Steam types, and the file-backed cloud
//! dir below stands in for the SDK cloud calls.

use std::collections::{BTreeMap, BTreeSet};

/// Env switch marking Steam as running (test hook; the real backend checks
/// the Steam client instead).
pub const STEAM_RUNNING_ENV: &str = "E2040_STEAM_RUNNING";
/// Env dir standing in for the SDK cloud store (tests and SDK-less runs).
pub const STEAM_CLOUD_DIR_ENV: &str = "E2040_STEAM_CLOUD_DIR";
/// Public Valve test app used for achievement smoke tests.
pub const STEAM_TEST_APP_ID: u32 = 480;
/// Largest cloud payload in bytes (mirrors the platform contract cap).
pub const STEAM_MAX_CLOUD_BYTES: usize = 1_048_576;
/// Cloud slots addressable per title.
pub const STEAM_CLOUD_SLOTS: u8 = 8;
/// Longest achievement id or stat name.
pub const STEAM_MAX_ID_LEN: usize = 64;

/// Title achievement catalog; unlocks outside it refuse loudly.
pub const KNOWN_ACHIEVEMENTS: &[&str] = &[
    "first_blood",
    "gladiator_10",
    "untouched",
    "comeback",
    "parry_riposte",
];

/// Tracked stat names; records outside them refuse loudly.
pub const KNOWN_STATS: &[&str] = &["wins", "losses", "matches", "best_streak"];

/// Steam backend failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SteamError {
    /// Steam is not running or the `steam` feature is off; run the null path.
    NoBackend,
    /// Id or name is empty, too long or outside `[A-Za-z0-9._-]`.
    BadId,
    /// Cloud slot is at or above [`STEAM_CLOUD_SLOTS`].
    BadSlot(u8),
    /// Payload exceeds [`STEAM_MAX_CLOUD_BYTES`].
    TooLarge,
    /// Achievement id is well-formed but not in [`KNOWN_ACHIEVEMENTS`].
    UnknownAchievement,
    /// Stat name is well-formed but not in [`KNOWN_STATS`].
    UnknownStat,
    /// Cloud dir IO failure, kind carried (paths stay in the caller).
    Io(std::io::ErrorKind),
    /// Cloud bytes on disk are empty or fail the length check.
    CloudCorrupt,
}

impl core::fmt::Display for SteamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::NoBackend => write!(f, "steam backend unavailable"),
            Self::BadId => write!(f, "bad steam id"),
            Self::BadSlot(slot) => write!(f, "bad steam cloud slot ({slot})"),
            Self::TooLarge => write!(f, "steam payload exceeds size bound"),
            Self::UnknownAchievement => write!(f, "unknown steam achievement"),
            Self::UnknownStat => write!(f, "unknown steam stat"),
            Self::Io(kind) => write!(f, "steam cloud io error ({kind:?})"),
            Self::CloudCorrupt => write!(f, "steam cloud bytes corrupt"),
        }
    }
}

impl std::error::Error for SteamError {}

/// Id rows: `1..=STEAM_MAX_ID_LEN` bytes of `[A-Za-z0-9._-]`.
fn check_id(id: &str) -> Result<(), SteamError> {
    if id.is_empty()
        || id.len() > STEAM_MAX_ID_LEN
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(SteamError::BadId);
    }
    Ok(())
}

/// Cloud file name for a slot (`save00.e2040` style, matching `engine-save`).
fn cloud_name(slot: u8) -> Result<String, SteamError> {
    if slot >= STEAM_CLOUD_SLOTS {
        return Err(SteamError::BadSlot(slot));
    }
    Ok(std::format!("save{slot:02}.e2040"))
}

/// Steamworks backend over a file-backed cloud dir. The SDK client behind
/// the `steam` feature replaces `probe` and the cloud file calls; every
/// other method keeps its shape so callers never branch on the feature.
#[derive(Debug)]
pub struct SteamBackend {
    /// Steam app id (the test app unless overridden by env at probe time).
    pub app_id: u32,
    /// Cloud store dir (the SDK cloud when the feature lands).
    pub cloud_dir: std::path::PathBuf,
    /// Achievements unlocked this session, in catalog order of first unlock.
    pub unlocked: BTreeSet<String>,
    /// Tracked stat values.
    pub stats: BTreeMap<String, i64>,
    /// Presence lines staged for the overlay.
    pub presence_state: String,
    /// Presence details staged for the overlay.
    pub presence_details: String,
}

impl SteamBackend {
    /// Probe for a running Steam client. Without `E2040_STEAM_RUNNING=1`
    /// (the real backend checks the client instead) this reports
    /// [`SteamError::NoBackend`] and the caller runs the null path.
    pub fn probe() -> Result<Self, SteamError> {
        let running = std::env::var(STEAM_RUNNING_ENV).is_ok_and(|v| v == "1");
        if !running {
            return Err(SteamError::NoBackend);
        }
        let cloud_dir = std::env::var(STEAM_CLOUD_DIR_ENV).map_or_else(
            |_| std::env::temp_dir().join("e2040-steam-cloud"),
            std::path::PathBuf::from,
        );
        Ok(Self {
            app_id: STEAM_TEST_APP_ID,
            cloud_dir,
            unlocked: BTreeSet::new(),
            stats: BTreeMap::new(),
            presence_state: String::new(),
            presence_details: String::new(),
        })
    }

    /// Backend name for receipts.
    #[must_use]
    pub fn name(&self) -> &'static str {
        "steam"
    }

    /// Unlock a catalog achievement; `Ok(true)` is newly unlocked.
    pub fn unlock_achievement(&mut self, id: &str) -> Result<bool, SteamError> {
        check_id(id)?;
        if !KNOWN_ACHIEVEMENTS.contains(&id) {
            return Err(SteamError::UnknownAchievement);
        }
        Ok(self.unlocked.insert(id.to_string()))
    }

    /// Record a tracked stat value.
    pub fn record_stat(&mut self, name: &str, value: i64) -> Result<(), SteamError> {
        check_id(name)?;
        if !KNOWN_STATS.contains(&name) {
            return Err(SteamError::UnknownStat);
        }
        self.stats.insert(name.to_string(), value);
        Ok(())
    }

    /// Last recorded stat value; untracked names fail loudly.
    pub fn stat(&self, name: &str) -> Result<i64, SteamError> {
        check_id(name)?;
        self.stats.get(name).copied().ok_or(SteamError::UnknownStat)
    }

    /// Store one versioned generation payload in the cloud store.
    pub fn store_generation(&self, slot: u8, payload: &[u8]) -> Result<(), SteamError> {
        if payload.len() > STEAM_MAX_CLOUD_BYTES {
            return Err(SteamError::TooLarge);
        }
        let name = cloud_name(slot)?;
        std::fs::create_dir_all(&self.cloud_dir).map_err(|err| SteamError::Io(err.kind()))?;
        std::fs::write(self.cloud_dir.join(name), payload)
            .map_err(|err| SteamError::Io(err.kind()))?;
        Ok(())
    }

    /// Load one versioned generation payload from the cloud store.
    pub fn load_generation(&self, slot: u8) -> Result<Vec<u8>, SteamError> {
        let name = cloud_name(slot)?;
        let bytes =
            std::fs::read(self.cloud_dir.join(name)).map_err(|err| SteamError::Io(err.kind()))?;
        if bytes.is_empty() || bytes.len() > STEAM_MAX_CLOUD_BYTES {
            return Err(SteamError::CloudCorrupt);
        }
        Ok(bytes)
    }

    /// Stage rich-presence lines for the overlay.
    pub fn set_presence(&mut self, state: &str, details: &str) {
        self.presence_state = state.to_string();
        self.presence_details = details.to_string();
    }
}

/// Cloud-mirror outcome for [`mirror_generation`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MirrorOutcome {
    /// Steam absent; the local save stands alone.
    Skipped,
    /// Generation mirrored to the cloud store.
    Stored,
}

/// Mirror one saved generation to the Steam cloud when a backend is up,
/// else skip silently: without Steam (or without the feature) the null
/// backend runs and the local save is the whole story.
#[must_use]
pub fn mirror_generation(slot: u8, payload: &[u8]) -> MirrorOutcome {
    let Ok(backend) = SteamBackend::probe() else {
        return MirrorOutcome::Skipped;
    };
    if backend.store_generation(slot, payload).is_ok() {
        MirrorOutcome::Stored
    } else {
        MirrorOutcome::Skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env vars are process-global; serialize every test that touches them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn cloud_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(std::format!("e2040-steam-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cloud dir");
        dir
    }

    #[test]
    fn null_path_without_steam() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var(STEAM_RUNNING_ENV);
        assert!(matches!(SteamBackend::probe(), Err(SteamError::NoBackend)));
        assert_eq!(mirror_generation(3, b"gen"), MirrorOutcome::Skipped);
    }

    #[test]
    fn backend_unlocks_stats_cloud_and_presence() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = cloud_dir("full");
        std::env::set_var(STEAM_RUNNING_ENV, "1");
        std::env::set_var(STEAM_CLOUD_DIR_ENV, &dir);
        let mut backend = SteamBackend::probe().expect("backend");
        assert_eq!(backend.name(), "steam");
        assert_eq!(backend.app_id, STEAM_TEST_APP_ID);
        assert!(backend.unlock_achievement("first_blood").expect("unlock"));
        assert!(!backend.unlock_achievement("first_blood").expect("repeat"));
        assert_eq!(
            backend.unlock_achievement("not.a.trophy"),
            Err(SteamError::UnknownAchievement)
        );
        assert_eq!(
            backend.unlock_achievement("has space"),
            Err(SteamError::BadId)
        );
        backend.record_stat("wins", 4).expect("record");
        assert_eq!(backend.stat("wins").expect("read"), 4);
        assert_eq!(
            backend.record_stat("kills", 1),
            Err(SteamError::UnknownStat)
        );
        let payload = b"sv1-generation-bytes";
        backend.store_generation(3, payload).expect("store");
        assert_eq!(backend.load_generation(3).expect("load"), payload);
        assert_eq!(
            mirror_generation(3, payload),
            MirrorOutcome::Stored,
            "mirror stores when the backend is up"
        );
        backend.set_presence("in arena", "final vs rook");
        assert_eq!(backend.presence_state, "in arena");
        std::env::remove_var(STEAM_RUNNING_ENV);
        std::env::remove_var(STEAM_CLOUD_DIR_ENV);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn cloud_bounds_and_corruption_refuse() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let dir = cloud_dir("bounds");
        std::env::set_var(STEAM_RUNNING_ENV, "1");
        std::env::set_var(STEAM_CLOUD_DIR_ENV, &dir);
        let backend = SteamBackend::probe().expect("backend");
        assert_eq!(
            backend.store_generation(STEAM_CLOUD_SLOTS, b"x"),
            Err(SteamError::BadSlot(STEAM_CLOUD_SLOTS))
        );
        assert_eq!(
            backend.store_generation(0, &vec![0u8; STEAM_MAX_CLOUD_BYTES + 1]),
            Err(SteamError::TooLarge)
        );
        assert!(backend.load_generation(1).is_err(), "missing slot fails");
        std::fs::write(dir.join("save02.e2040"), b"").expect("empty cloud file");
        assert_eq!(
            backend.load_generation(2),
            Err(SteamError::CloudCorrupt),
            "empty cloud bytes refuse"
        );
        std::env::remove_var(STEAM_RUNNING_ENV);
        std::env::remove_var(STEAM_CLOUD_DIR_ENV);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
