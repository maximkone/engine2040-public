//! Shared strict row plumbing for the game session config vectors.
//!
//! Consumer: the `config_*_vectors` integration tests over
//! [`engine_game::GameConfig`]. Data owner: this module owns the strict row
//! parser; `engine-game` owns [`engine_game::GameConfig`] and its range
//! checks. Units: seed is an opaque `u64`, ticks and fighters are integer
//! counts, extent is meters. Determinism scope: same rows give the same
//! accept/refuse answer, no IO. Errors: [`engine_game::GameError::BadConfig`]
//! names the culprit row. One home for the row keys so a set-but-unread key
//! refuses instead of hiding in a per-file copy. Pure, never panics.

use engine_game::{GameConfig, GameError, MAX_TICKS};
use engine_input::MAX_PARTICIPANTS;

/// Known strict keys: any other key refuses (the `deny_unknown_fields` half).
pub const KNOWN_KEYS: [&str; 4] = ["seed", "ticks", "fighters", "half_extent_m"];

/// True when the text is non-empty ASCII digits only (integer-only gate, so a
/// float tick such as `600.0` never parses). Pure.
fn is_digits(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit())
}

/// Look up one named row; `None` when absent. Pure.
pub fn row_of<'a>(rows: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    rows.iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Strict parse of config rows: unknown keys refuse, every value parses
/// integer-only (extent accepts integer text as meters), then the owned
/// [`GameConfig::validate`] range check runs. Pure, never panics.
pub fn strict_parse(rows: &[(&str, &str)]) -> Result<GameConfig, GameError> {
    for entry in rows {
        if !KNOWN_KEYS.contains(&entry.0) {
            return Err(GameError::BadConfig("unknown"));
        }
    }
    let seed_raw = row_of(rows, "seed").ok_or(GameError::BadConfig("seed"))?;
    let ticks_raw = row_of(rows, "ticks").ok_or(GameError::BadConfig("ticks"))?;
    let fighters_raw = row_of(rows, "fighters").ok_or(GameError::BadConfig("fighters"))?;
    let extent_raw = row_of(rows, "half_extent_m").ok_or(GameError::BadConfig("half_extent_m"))?;
    if !is_digits(seed_raw) {
        return Err(GameError::BadConfig("seed"));
    }
    let seed: u64 = seed_raw.parse().map_err(|_| GameError::BadConfig("seed"))?;
    if !is_digits(ticks_raw) {
        return Err(GameError::BadConfig("ticks"));
    }
    let ticks: u32 = ticks_raw
        .parse()
        .map_err(|_| GameError::BadConfig("ticks"))?;
    if ticks == 0 || ticks > MAX_TICKS {
        return Err(GameError::BadConfig("ticks"));
    }
    if !is_digits(fighters_raw) {
        return Err(GameError::BadConfig("fighters"));
    }
    let fighters: u8 = fighters_raw
        .parse()
        .map_err(|_| GameError::BadConfig("fighters"))?;
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(GameError::BadConfig("fighters"));
    }
    // Extent vectors stay integer-only (`20`, `5`, `100`); parsed as meters.
    if !is_digits(extent_raw) {
        return Err(GameError::BadConfig("half_extent_m"));
    }
    let half_extent_m: f32 = extent_raw
        .parse()
        .map_err(|_| GameError::BadConfig("half_extent_m"))?;
    let config = GameConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
    };
    config.validate()?;
    Ok(config)
}
