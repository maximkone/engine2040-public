//! GameConfig unknown-key refusal vectors (verify shape).
//!
//! Consumer: packaging and quality review over [`engine_game::GameConfig`].
//! Data owner: this test owns its strict row parser; `engine-game` owns
//! [`engine_game::GameConfig`] and [`engine_game::GameError`]. Units: seed is
//! an opaque `u64`, ticks and fighters are counts, extent is meters.
//! Determinism scope: same seed gives the same 50 rows and verdicts, no IO.
//! Errors: [`engine_game::GameError::BadConfig`] names the culprit row.
//!
//! Donor idea: s-panferov/valico (MIT) `ScopedSchema::validate` plus the
//! `ban_unknown_keywords` idea: unknown keys refuse before value checks.
//! Fresh std-only reimplementation for game rows here; no bytes copied and
//! no new dependency. HEAD freshness: upstream HEAD was not fetched live in
//! this packet (offline lane), so re-check upstream before extending shape.

use engine_core::splitmix64;
use engine_game::{GameConfig, GameError, MAX_TICKS};
use engine_input::MAX_PARTICIPANTS;

/// Known strict keys: any other key refuses (the ban-unknown half).
const KNOWN_KEYS: [&str; 4] = ["seed", "ticks", "fighters", "half_extent_m"];
/// Twelve unknown keys that must all refuse with `"unknown"`.
const UNKNOWN_KEYS: [&str; 12] = [
    "mode",
    "arena",
    "difficulty",
    "seed_hex",
    "TICKS",
    "ticks ",
    "half-extent-m",
    "fighters_count",
    "half_extent",
    "seed2",
    "extra",
    "debug",
];
/// Seeded fixture size: 38 pass rows plus 12 unknown-key rows.
const PASS_ROWS: usize = 38;
/// Full seeded fixture size.
const TOTAL_ROWS: usize = 50;
/// Fixture seed: same input gives the same rows.
const FIXTURE_SEED: u64 = 2040;

/// One fixture row: valid base fields plus an optional unknown key.
struct TestVector {
    seed: u64,
    ticks: u32,
    fighters: u8,
    extent_m: u8,
    unknown: Option<&'static str>,
}

/// True when the text is non-empty ASCII digits only, so float text such as
/// `600.0` never parses. Pure.
fn is_digits(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit())
}

/// Look up one named row; `None` when absent. Pure.
fn row_of<'a>(rows: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    rows.iter()
        .find(|entry| entry.0 == key)
        .map(|entry| entry.1)
}

/// Strict parse of config rows: unknown keys refuse first (the
/// `ban_unknown_keywords` half), then known values parse integer-only and
/// the owned [`GameConfig::validate`] range check runs. Pure.
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
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(GameError::BadConfig("fighters"));
    }
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

/// Next draw from a chained `splitmix64` stream. Pure.
fn next_draw(state: &mut u64) -> u64 {
    let out = splitmix64(*state);
    *state = out;
    out
}

/// Build the 50 seeded rows: the first 38 are valid, the last 12 each carry
/// one unknown key. Same seed gives the same rows. Pure.
fn seeded_vectors() -> Vec<TestVector> {
    let mut state = FIXTURE_SEED;
    let mut out = Vec::with_capacity(TOTAL_ROWS);
    for index in 0..TOTAL_ROWS {
        let seed = next_draw(&mut state);
        let ticks = u32::try_from(next_draw(&mut state) % u64::from(MAX_TICKS) + 1)
            .unwrap_or(1)
            .max(1);
        let fighters = u8::try_from(next_draw(&mut state) % u64::from(MAX_PARTICIPANTS - 1) + 2u64)
            .unwrap_or(2)
            .max(2);
        let extent_m = u8::try_from(next_draw(&mut state) % 96 + 5)
            .unwrap_or(5)
            .max(5);
        let unknown = if index < PASS_ROWS {
            None
        } else {
            Some(UNKNOWN_KEYS[index - PASS_ROWS])
        };
        out.push(TestVector {
            seed,
            ticks,
            fighters,
            extent_m,
            unknown,
        });
    }
    out
}

/// Render one vector into owned texts.
fn render_rows(vector: &TestVector) -> (String, String, String, String) {
    (
        vector.seed.to_string(),
        vector.ticks.to_string(),
        vector.fighters.to_string(),
        vector.extent_m.to_string(),
    )
}

#[test]
fn unknown_keys_all_flagged_12_of_12() {
    let vectors = seeded_vectors();
    let mut flagged = 0;
    for vector in vectors.iter().filter(|entry| entry.unknown.is_some()) {
        let rendered = render_rows(vector);
        let mut rows: Vec<(&str, &str)> = Vec::with_capacity(KNOWN_KEYS.len() + 1);
        rows.push(("seed", rendered.0.as_str()));
        rows.push(("ticks", rendered.1.as_str()));
        rows.push(("fighters", rendered.2.as_str()));
        rows.push(("half_extent_m", rendered.3.as_str()));
        let unknown = vector.unknown.unwrap_or("extra");
        rows.push((unknown, "1"));
        assert_eq!(
            strict_parse(&rows),
            Err(GameError::BadConfig("unknown")),
            "unknown key must refuse"
        );
        flagged += 1;
    }
    assert_eq!(flagged, 12, "all twelve unknown keys refuse");
}

#[test]
fn seeded_pass_vectors_all_accept_38_of_38() {
    let vectors = seeded_vectors();
    assert_eq!(vectors.len(), TOTAL_ROWS, "fixture holds fifty rows");
    let mut passed = 0;
    for vector in vectors.iter().filter(|entry| entry.unknown.is_none()) {
        let rendered = render_rows(vector);
        let rows: [(&str, &str); 4] = [
            ("seed", rendered.0.as_str()),
            ("ticks", rendered.1.as_str()),
            ("fighters", rendered.2.as_str()),
            ("half_extent_m", rendered.3.as_str()),
        ];
        let parsed = strict_parse(&rows);
        assert_eq!(
            parsed,
            Ok(GameConfig {
                seed: vector.seed,
                ticks: vector.ticks,
                fighters: vector.fighters,
                half_extent_m: f32::from(vector.extent_m),
            }),
            "seeded pass row must accept"
        );
        passed += 1;
    }
    assert_eq!(passed, PASS_ROWS, "all thirty-eight pass rows accept");
}

#[test]
fn seeded_fixture_is_deterministic() {
    let first = seeded_vectors();
    let second = seeded_vectors();
    assert_eq!(first.len(), second.len(), "same seed gives same length");
    for (left, right) in first.iter().zip(second.iter()) {
        assert_eq!(left.seed, right.seed, "seed stream repeats");
        assert_eq!(left.ticks, right.ticks, "ticks stream repeats");
        assert_eq!(left.fighters, right.fighters, "fighters stream repeats");
        assert_eq!(left.extent_m, right.extent_m, "extent stream repeats");
        assert_eq!(left.unknown, right.unknown, "unknown slot repeats");
    }
}
