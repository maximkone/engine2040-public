//! progress: versioned game-progression payload owned by `engine-game`.
//!
//! Consumer: [`App`](super::App) save/load through `engine-save`
//! (multiplayer and packaging build on this file shape). Data owner: this
//! module owns the payload bytes; `engine-save` owns the envelope/storage
//! around them and never imports this crate. Units: counts are plain
//! integers, `match_seed` is the arena seed, `match_ticks` is a fixed-tick
//! cursor. Determinism: fixed 25-byte little-endian layout, no padding, no
//! pointers. Errors: [`ProgressError`] on length or version mismatch; v1
//! loads, anything newer is rejected, anything short/long/corrupt is rejected.

/// Payload version this crate writes and reads.
pub const PROGRESS_VERSION: u8 = 1;
/// Exact payload length in bytes: version (1) + 3 counts (12) + seed (8) +
/// match ticks (4). Fixed so truncation and trailing garbage both fail.
pub const PROGRESS_BYTES: usize = 25;

/// Progression payload failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ProgressError {
    /// Payload length is not [`PROGRESS_BYTES`], length carried.
    BadLength(usize),
    /// Payload version is newer than [`PROGRESS_VERSION`], version carried.
    FutureVersion(u8),
}

impl core::fmt::Display for ProgressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadLength(len) => write!(f, "progression payload length {len}"),
            Self::FutureVersion(version) => {
                write!(f, "progression payload version {version} is newer than v1")
            }
        }
    }
}

impl std::error::Error for ProgressError {}

/// Versioned progression state: arena-ladder counters (progress-lite per
/// `FOCUS.md`) plus the match the counters were saved under. Plain integers
/// only, so the bytes stay stable across builds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct GameProgress {
    /// Matches decided (first knockdown seen) since this payload started.
    pub matches_completed: u32,
    /// Knockdowns scored by the human fighter.
    pub wins: u32,
    /// Knockdowns suffered by the human fighter.
    pub losses: u32,
    /// Arena seed of the live match at save time.
    pub match_seed: u64,
    /// Fixed-tick cursor of the live match at save time.
    pub match_ticks: u32,
}

impl GameProgress {
    /// Fixed-layout little-endian encode: `[version, matches, wins, losses,
    /// seed, ticks]`. Pure.
    #[must_use]
    pub fn encode(self) -> [u8; PROGRESS_BYTES] {
        let mut out = [0u8; PROGRESS_BYTES];
        out[0] = PROGRESS_VERSION;
        out[1..5].copy_from_slice(&self.matches_completed.to_le_bytes());
        out[5..9].copy_from_slice(&self.wins.to_le_bytes());
        out[9..13].copy_from_slice(&self.losses.to_le_bytes());
        out[13..21].copy_from_slice(&self.match_seed.to_le_bytes());
        out[21..25].copy_from_slice(&self.match_ticks.to_le_bytes());
        out
    }

    /// Decode exactly [`PROGRESS_BYTES`] at version [`PROGRESS_VERSION`].
    /// Future versions fail (forward compatibility is explicit migration, not
    /// silent reinterpretation); short, long and empty inputs fail. Pure.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProgressError> {
        if bytes.len() != PROGRESS_BYTES {
            return Err(ProgressError::BadLength(bytes.len()));
        }
        if bytes[0] != PROGRESS_VERSION {
            return Err(ProgressError::FutureVersion(bytes[0]));
        }
        let mut counts = [0u8; 4];
        let mut seed = [0u8; 8];
        counts.copy_from_slice(&bytes[1..5]);
        let matches_completed = u32::from_le_bytes(counts);
        counts.copy_from_slice(&bytes[5..9]);
        let wins = u32::from_le_bytes(counts);
        counts.copy_from_slice(&bytes[9..13]);
        let losses = u32::from_le_bytes(counts);
        seed.copy_from_slice(&bytes[13..21]);
        let match_seed = u64::from_le_bytes(seed);
        counts.copy_from_slice(&bytes[21..25]);
        let match_ticks = u32::from_le_bytes(counts);
        Ok(Self {
            matches_completed,
            wins,
            losses,
            match_seed,
            match_ticks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GameProgress {
        GameProgress {
            matches_completed: 3,
            wins: 2,
            losses: 1,
            match_seed: 2040,
            match_ticks: 600,
        }
    }

    #[test]
    fn v1_roundtrip_is_byte_stable() {
        let first = sample().encode();
        assert_eq!(first.len(), PROGRESS_BYTES);
        assert_eq!(first[0], PROGRESS_VERSION);
        assert_eq!(first, sample().encode());
        assert_eq!(GameProgress::decode(&first).unwrap(), sample());
        assert_eq!(GameProgress::default().encode().len(), PROGRESS_BYTES);
        assert_eq!(
            GameProgress::decode(&GameProgress::default().encode()).unwrap(),
            GameProgress::default()
        );
    }

    #[test]
    fn future_versions_rejected() {
        for version in [PROGRESS_VERSION + 1, u8::MAX] {
            let mut bytes = sample().encode();
            bytes[0] = version;
            assert_eq!(
                GameProgress::decode(&bytes),
                Err(ProgressError::FutureVersion(version))
            );
        }
    }

    #[test]
    fn corrupt_lengths_rejected() {
        let bytes = sample().encode();
        for len in [
            0,
            1,
            PROGRESS_BYTES - 1,
            PROGRESS_BYTES + 1,
            PROGRESS_BYTES + 75,
        ] {
            let mut clipped = bytes.to_vec();
            clipped.resize(len, 0xAB);
            assert_eq!(
                GameProgress::decode(&clipped),
                Err(ProgressError::BadLength(len))
            );
        }
    }
}
