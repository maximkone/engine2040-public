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

use crate::config_rows::strict_parse;
use engine_game::{GameConfig, GameError};

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
