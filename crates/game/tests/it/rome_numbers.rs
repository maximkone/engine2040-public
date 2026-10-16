//! Rome numbers drift check for the shared numbers file.
//!
//! One file both builds read. Test fails on drift.
//! Units: meters, Hz, ticks, HP, mm, seeds.

use engine_combat::fight::TABLES_V1;
use engine_core::config::{Budgets, Config};

/// Included numbers file. Missing file fails the build.
const NUMBERS_SRC: &str = include_str!("../../../../content/arena/rome-numbers.json");
/// Included placement. Walls ride along.
const PLACEMENT_SRC: &str = include_str!("../../../../content/arena/placement.json");

/// Number token after `"key":`. Panics with the key.
fn token_after<'a>(text: &'a str, key: &str) -> &'a str {
    let needle = std::format!("\"{key}\"");
    let start = text
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("rome numbers missing key {key}"));
    let colon = start.saturating_add(
        text[start..]
            .find(':')
            .unwrap_or_else(|| panic!("rome numbers key {key} has no value")),
    );
    let rest = text[colon.saturating_add(1)..].trim_start();
    let end = rest.find([',', '}', ']', '\n', ' ']).unwrap_or(rest.len());
    rest[..end].trim()
}

/// Parse a whole number. Names the key on failure.
fn num_u64(key: &str) -> u64 {
    token_after(NUMBERS_SRC, key)
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("rome numbers {key} must be a whole number"))
}

/// Parse a decimal number. Names the key on failure.
fn num_f32(key: &str) -> f32 {
    token_after(NUMBERS_SRC, key)
        .parse::<f32>()
        .unwrap_or_else(|_| panic!("rome numbers {key} must be a number"))
}

/// Parse `[a, b, c]` after `"key":`. Panics with the key.
fn num_triple(key: &str) -> [f32; 3] {
    let needle = std::format!("\"{key}\"");
    let start = NUMBERS_SRC
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("rome numbers missing key {key}"));
    let open = start.saturating_add(
        NUMBERS_SRC[start..]
            .find('[')
            .unwrap_or_else(|| panic!("rome numbers {key} has no list")),
    );
    let close = open.saturating_add(
        NUMBERS_SRC[open..]
            .find(']')
            .unwrap_or_else(|| panic!("rome numbers {key} list never closes")),
    );
    let body = &NUMBERS_SRC[open.saturating_add(1)..close];
    let mut parts = body.split(',');
    let mut out = [0.0_f32; 3];
    for slot in &mut out {
        *slot = parts
            .next()
            .unwrap_or_else(|| panic!("rome numbers {key} wants 3 numbers"))
            .trim()
            .parse::<f32>()
            .unwrap_or_else(|_| panic!("rome numbers {key} must hold numbers"));
    }
    out
}

/// Wall `z` after `"wall_name"` in placement text. Names the wall.
fn wall_z(wall: &str) -> f32 {
    let needle = std::format!("\"{wall}\"");
    let start = PLACEMENT_SRC
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("placement missing {wall}"));
    let tail = &PLACEMENT_SRC[start..];
    let z_at = tail
        .find("\"z\"")
        .unwrap_or_else(|| panic!("placement {wall} has no z"));
    let after = &tail[z_at.saturating_add(3)..];
    let colon = after
        .find(':')
        .unwrap_or_else(|| panic!("placement {wall} z has no value"));
    let rest = after[colon.saturating_add(1)..].trim_start();
    let end = rest.find([',', '\n']).unwrap_or(rest.len());
    rest[..end]
        .trim()
        .parse::<f32>()
        .unwrap_or_else(|_| panic!("placement {wall} z must be a number"))
}

#[test]
fn spec_matches_arena_constants() {
    assert_eq!(num_f32("half_extent_m"), 10.0, "half extent is 10 m");
    assert_eq!(num_u64("tick_hz"), 60, "tick rate is 60 Hz");
    assert_eq!(num_u64("max_ticks"), 600, "a round is 600 ticks");
    assert_eq!(num_u64("damage_hp"), 25, "sword damage is 25 HP");
    assert_eq!(num_u64("cooldown_ticks"), 30, "cooldown is 30 ticks");
    assert_eq!(num_u64("range_mm"), 2000, "sword range is 2000 mm");
    assert_eq!(num_triple("spawn_a_m"), [-4.0, 0.0, 0.0], "spawn A is -4 m");
    assert_eq!(num_triple("spawn_b_m"), [4.0, 0.0, 0.0], "spawn B is +4 m");
    assert_eq!(num_u64("seed"), 2040, "map seed is 2040");
    // Code constants track the same numbers.
    assert_eq!(engine_core::TICK_HZ, num_u64("tick_hz"));
    assert_eq!(Config::default().tick_hz, num_u64("tick_hz"));
    assert_eq!(Config::default().seed, num_u64("seed"));
    assert_eq!(Budgets::default().max_ticks, num_u64("max_ticks"));
    assert_eq!(
        u64::from(engine_game::GameConfig::default().ticks),
        num_u64("max_ticks")
    );
    assert_eq!(
        u64::from(engine_combat::sweep::HURT_RADIUS_MAX_MM),
        num_u64("range_mm")
    );
    // V1 row stays frozen, so the proof scene never moves with this file.
    assert_eq!(TABLES_V1.attack_damage, 120);
}

#[test]
fn walls_match_numbers_half_extent() {
    let half = num_f32("half_extent_m");
    assert_eq!(wall_z("wall_north"), -half, "north wall sits at -half");
    assert_eq!(wall_z("wall_south"), half, "south wall sits at +half");
}
