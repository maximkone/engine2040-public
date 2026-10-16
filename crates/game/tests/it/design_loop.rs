//! Design loop drift: plan round numbers plus ladder tuning as checks.
//!
//! Consumer: the systems designer edits `docs/plan/rome-spec.json` and
//! `content/design/loop.rows`; this test fails when code drifts from them.
//! Data owner: the plan file owns round numbers; `engine-game` owns session
//! defaults; `ladder` owns glory numbers; `loop_text` owns the rows parse.
//! Units: ticks at 60 Hz, round wins, glory counts. Determinism: pure string
//! scan over included text, no IO at test time. Errors: panics name the
//! culprit key; a moved number fails by name.

use engine_core::config::Budgets;
use engine_game::ladder::{
    Ladder, Outcome, GLORY_PER_LOOP, GLORY_PER_WIN, GLORY_RANKS, GLORY_RUNG_BONUS, MAX_RUNG,
};
use engine_game::loop_text::parse_loop;
use engine_game::GameConfig;

/// Included plan spec (compile-time, so a missing file fails the build).
const PLAN_SPEC_SRC: &str = include_str!("../../../../docs/plan/rome-spec.json");
/// Included loop rows (compile-time, so a missing file fails the build).
const LOOP_SRC: &str = include_str!("../../../../content/design/loop.rows");

/// Number token after `"key":` (stops at `,`/`}`/`]`/whitespace). Panics
/// with the key name when absent, so the culprit is named.
fn token_after<'a>(text: &'a str, key: &str) -> &'a str {
    let needle = std::format!("\"{key}\"");
    let start = text
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("plan spec missing key {key}"));
    let colon = start.saturating_add(
        text[start..]
            .find(':')
            .unwrap_or_else(|| panic!("plan spec key {key} has no value")),
    );
    let rest = text[colon.saturating_add(1)..].trim_start();
    let end = rest.find([',', '}', ']', '\n', ' ']).unwrap_or(rest.len());
    rest[..end].trim()
}

/// Parse a `u64` plan number, naming the key on failure. Pure.
fn spec_u64(key: &str) -> u64 {
    token_after(PLAN_SPEC_SRC, key)
        .parse::<u64>()
        .unwrap_or_else(|_| panic!("plan spec {key} must be a whole number"))
}

#[test]
fn plan_round_is_600_best_of_5_first_to_3() {
    assert!(
        PLAN_SPEC_SRC.contains("best-of-5 rounds, first to 3 round wins"),
        "plan match must stay best of 5, first to 3"
    );
    assert!(
        PLAN_SPEC_SRC
            .contains("600 ticks; higher total HP wins on timeout; exact tie replays the round"),
        "plan round must stay 600 ticks with hp win and tie replay"
    );
    assert_eq!(spec_u64("max_ticks"), 600, "plan max_ticks is 600");
    assert_eq!(spec_u64("tick_hz"), 60, "plan tick_hz is 60");
    assert_eq!(
        u64::from(engine_game::GameConfig::default().ticks),
        spec_u64("max_ticks"),
        "session default ticks track the plan round"
    );
    assert_eq!(
        u64::from(GameConfig::default().ticks),
        600,
        "a round is 600 ticks"
    );
    assert_eq!(
        u64::from(Budgets::default().max_ticks),
        spec_u64("max_ticks"),
        "core budget tracks the plan round"
    );
    assert_eq!(engine_core::TICK_HZ, spec_u64("tick_hz"));
}

#[test]
fn ladder_constants_match_plan_and_rows() {
    assert_eq!(MAX_RUNG, 7, "rungs run 0..=7");
    assert_eq!(GLORY_PER_WIN, 100, "win pays 100");
    assert_eq!(GLORY_RUNG_BONUS, 25, "rung bonus is 25");
    assert_eq!(GLORY_PER_LOOP, 500, "top-rung loop pays 500");
    assert_eq!(
        GLORY_RANKS,
        [0, 300, 900, 1800, 3600],
        "rank floors stay put"
    );
    let set = parse_loop(LOOP_SRC).expect("shipped loop.rows parses");
    assert_eq!(set.version, 1, "shipped loop version is 1");
    assert_eq!(set.tables.glory_per_win, GLORY_PER_WIN);
    assert_eq!(set.tables.glory_rung_bonus, GLORY_RUNG_BONUS);
    assert_eq!(set.tables.glory_per_loop, GLORY_PER_LOOP);
    assert_eq!(set.tables.glory_ranks, GLORY_RANKS);
    let script = [Outcome::Win, Outcome::Win, Outcome::Loss, Outcome::Win];
    let mut compiled = Ladder::new();
    let mut from_rows = Ladder::new();
    for (index, outcome) in script.iter().enumerate() {
        let tick = index as u32;
        compiled.apply(*outcome, tick);
        from_rows.apply_with_tables(*outcome, tick, &set.tables);
    }
    assert_eq!(from_rows, compiled, "rows fold like compiled");
    assert_eq!(from_rows.rank_with_tables(&set.tables), compiled.rank());
}

#[test]
fn loop_rows_loader_refuses_bad() {
    let bad_version = LOOP_SRC.replace("table loop version 1", "table loop version 0");
    assert!(parse_loop(&bad_version).is_err(), "version 0 must refuse");
    let bad_int = LOOP_SRC.replace("glory_per_win=100", "glory_per_win=10.0");
    assert!(parse_loop(&bad_int).is_err(), "float must refuse");
    let missing = LOOP_SRC.replace(" glory_per_loop=500", "");
    assert!(parse_loop(&missing).is_err(), "missing key must refuse");
}
