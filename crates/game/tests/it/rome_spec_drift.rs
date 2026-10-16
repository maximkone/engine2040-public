//! Rome numbers drift: one shared file both builds read.
//!
//! Consumer: the design seat edits `content/arena/rome-spec.json`; this test
//! fails when an arena constant drifts from it. Data owner: the spec file
//! owns the numbers; `engine-core` owns tick/seed budgets, `engine-combat`
//! owns damage/range bounds, `engine-game` owns the session defaults.
//! Units: meters, Hz, ticks, HP, mm, seeds. Determinism: pure string scan
//! over included text, no IO at test time. Errors: panics name the culprit
//! key; a moved wall fails the wall test by name.

use engine_combat::fight::TABLES_V1;
use engine_core::config::{Budgets, Config};

/// Included Rome spec (compile-time, so a missing file fails the build).
const SPEC_SRC: &str = include_str!("../../../../content/arena/rome-spec.json");
/// Included arena placement (compile-time, walls ride along).
const PLACEMENT_SRC: &str = include_str!("../../../../content/arena/placement.json");

/// Number token after `"key":` (stops at `,`/`}`/`]`/whitespace). Panics
/// with the key name when absent, so the culprit is named.
fn token_after<'a>(text: &'a str, key: &str) -> &'a str {
    let needle = std::format!("\"{key}\"");
    let start = text
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("rome spec missing key {key}"));
    let colon = start.saturating_add(
        text[start..]
            .find(':')
            .unwrap_or_else(|| panic!("rome spec key {key} has no value")),
    );
    let rest = text[colon.saturating_add(1)..].trim_start();
    let end = rest.find([',', '}', ']', '\n', ' ']).unwrap_or(rest.len());
    rest[..end].trim()
}

/// Parse a `u64` spec number, naming the key on failure. Pure.
fn spec_u64(key: &str) -> u64 {
    token_after(SPEC_SRC, key)
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("rome spec {key} must be a whole number"))
}

/// Parse an `f32` spec number, naming the key on failure. Pure.
fn spec_f32(key: &str) -> f32 {
    token_after(SPEC_SRC, key)
        .parse::<f32>()
        .unwrap_or_else(|_| panic!("rome spec {key} must be a number"))
}

/// Parse a 3-tuple `[a, b, c]` after `"key":`. Panics with the key. Pure.
fn spec_triple(key: &str) -> [f32; 3] {
    let needle = std::format!("\"{key}\"");
    let start = SPEC_SRC
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("rome spec missing key {key}"));
    let open = start.saturating_add(
        SPEC_SRC[start..]
            .find('[')
            .unwrap_or_else(|| panic!("rome spec {key} has no list")),
    );
    let close = open.saturating_add(
        SPEC_SRC[open..]
            .find(']')
            .unwrap_or_else(|| panic!("rome spec {key} list never closes")),
    );
    let body = &SPEC_SRC[open.saturating_add(1)..close];
    let mut parts = body.split(',');
    let mut out = [0.0_f32; 3];
    for slot in &mut out {
        *slot = parts
            .next()
            .unwrap_or_else(|| panic!("rome spec {key} wants 3 numbers"))
            .trim()
            .parse::<f32>()
            .unwrap_or_else(|_| panic!("rome spec {key} must hold numbers"));
    }
    out
}

/// Wall `z` after `"wall_name"` in placement text. Panics with the wall
/// name, so a moved wall names itself. Pure.
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
    assert_eq!(spec_f32("half_extent_m"), 10.0, "half extent is 10 m");
    assert_eq!(spec_u64("tick_hz"), 60, "tick rate is 60 Hz");
    assert_eq!(spec_u64("max_ticks"), 600, "a round is 600 ticks");
    assert_eq!(spec_u64("damage_hp"), 25, "sword damage is 25 HP");
    assert_eq!(spec_u64("cooldown_ticks"), 30, "cooldown is 30 ticks");
    assert_eq!(spec_u64("range_mm"), 2000, "sword range is 2000 mm");
    assert_eq!(
        spec_triple("spawn_a_m"),
        [-4.0, 0.0, 0.0],
        "duel spawn A is -4 m"
    );
    assert_eq!(
        spec_triple("spawn_b_m"),
        [4.0, 0.0, 0.0],
        "duel spawn B is +4 m"
    );
    assert_eq!(spec_u64("seed"), 2040, "map seed is 2040");
    // Code constants track the same numbers; a drift on either side fails.
    assert_eq!(engine_core::TICK_HZ, spec_u64("tick_hz"));
    assert_eq!(Config::default().tick_hz, spec_u64("tick_hz"));
    assert_eq!(Config::default().seed, spec_u64("seed"));
    assert_eq!(Budgets::default().max_ticks, spec_u64("max_ticks"));
    assert_eq!(
        u64::from(engine_game::GameConfig::default().ticks),
        spec_u64("max_ticks")
    );
    assert_eq!(
        u64::from(engine_combat::sweep::HURT_RADIUS_MAX_MM),
        spec_u64("range_mm")
    );
    // The shipped V1 row stays frozen beside the Rome preset, so the proof
    // scene and the golden never move with this file.
    assert_eq!(TABLES_V1.attack_damage, 120);
}

#[test]
fn walls_match_spec_half_extent() {
    let half = spec_f32("half_extent_m");
    assert_eq!(wall_z("wall_north"), -half, "north wall sits at -half");
    assert_eq!(wall_z("wall_south"), half, "south wall sits at +half");
}
