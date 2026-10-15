//! Strict-config refusal vectors for the game session config.
//!
//! Consumer: packaging and quality review over [`engine_game::GameConfig`].
//! Data owner: this test owns its strict row parser; `engine-game` owns
//! [`engine_game::GameConfig`] and [`engine_game::GameError`]. Units: seed is
//! an opaque `u64`, ticks and fighters are integer counts, extent is meters.
//! Determinism scope: same rows give the same accept/refuse answer, no IO.
//! Errors: [`engine_game::GameError::BadConfig`] names the culprit row.
//!
//! Port shape, fresh vectors (lap 4 STEAL-PORT2, in-repo, no licence risk).
//! Donor: `crates/scene/src/component.rs:285` `deny_unknown_fields` (unknown
//! rows refuse) + `crates/game/src/lib.rs:150-162` `GameConfig::validate`
//! (range checks). Shape only, reimplemented here for rows; no bytes copied.
//! Vectors are integer-only; ticks never use float text.

use engine_game::{GameConfig, GameError, MAX_TICKS};
use engine_input::MAX_PARTICIPANTS;

/// Known strict keys: any other key refuses (the `deny_unknown_fields` half).
const KNOWN_KEYS: [&str; 4] = ["seed", "ticks", "fighters", "half_extent_m"];

/// True when the text is non-empty ASCII digits only (integer-only gate, so a
/// float tick such as `600.0` never parses). Pure.
fn is_digits(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit())
}

/// Look up one named row; `None` when absent. Pure.
fn row_of<'a>(rows: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    rows.iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Strict parse of config rows: unknown keys refuse, every value parses
/// integer-only (extent accepts integer text as meters), then the owned
/// [`GameConfig::validate`] range check runs. Pure, never panics.
fn strict_parse(rows: &[(&str, &str)]) -> Result<GameConfig, GameError> {
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
    if fighters < 2 || fighters > MAX_PARTICIPANTS {
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

#[test]
fn strict_bad_seed_refused() {
    let rows = [
        ("seed", "abc"),
        ("ticks", "600"),
        ("fighters", "2"),
        ("half_extent_m", "20"),
    ];
    assert_eq!(strict_parse(&rows), Err(GameError::BadConfig("seed")));
}

#[test]
fn strict_bad_ticks_refused() {
    let rows = [
        ("seed", "2040"),
        ("ticks", "0"),
        ("fighters", "2"),
        ("half_extent_m", "20"),
    ];
    assert_eq!(strict_parse(&rows), Err(GameError::BadConfig("ticks")));
}

#[test]
fn strict_bad_fighters_refused() {
    let rows = [
        ("seed", "2040"),
        ("ticks", "600"),
        ("fighters", "1"),
        ("half_extent_m", "20"),
    ];
    assert_eq!(strict_parse(&rows), Err(GameError::BadConfig("fighters")));
}

#[test]
fn strict_bad_extent_refused() {
    let rows = [
        ("seed", "2040"),
        ("ticks", "600"),
        ("fighters", "2"),
        ("half_extent_m", "4"),
    ];
    assert_eq!(
        strict_parse(&rows),
        Err(GameError::BadConfig("half_extent_m"))
    );
}

#[test]
fn strict_unknown_key_refused() {
    let rows = [
        ("seed", "2040"),
        ("ticks", "600"),
        ("fighters", "2"),
        ("half_extent_m", "20"),
        ("mode", "arena"),
    ];
    assert_eq!(strict_parse(&rows), Err(GameError::BadConfig("unknown")));
}

#[test]
fn strict_happy_defaults_accepted() {
    let rows = [
        ("seed", "2040"),
        ("ticks", "600"),
        ("fighters", "2"),
        ("half_extent_m", "20"),
    ];
    assert_eq!(
        strict_parse(&rows).expect("defaults accept"),
        GameConfig {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        }
    );
}

#[test]
fn strict_happy_min_edge_accepted() {
    let rows = [
        ("seed", "1"),
        ("ticks", "1"),
        ("fighters", "2"),
        ("half_extent_m", "5"),
    ];
    assert_eq!(
        strict_parse(&rows).expect("min edge accepts"),
        GameConfig {
            seed: 1,
            ticks: 1,
            fighters: 2,
            half_extent_m: 5.0,
        }
    );
}

#[test]
fn strict_happy_max_edge_accepted() {
    let rows = [
        ("seed", "777"),
        ("ticks", "100000"),
        ("fighters", "16"),
        ("half_extent_m", "100"),
    ];
    assert_eq!(
        strict_parse(&rows).expect("max edge accepts"),
        GameConfig {
            seed: 777,
            ticks: 100_000,
            fighters: 16,
            half_extent_m: 100.0,
        }
    );
}
