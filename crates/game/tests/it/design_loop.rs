//! Design loop drift: plan round numbers, ladder tuning and the matchup
//! table as checks.
//!
//! Consumer: the systems designer edits `docs/plan/rome-spec.json`,
//! `content/design/loop.rows` and `docs/design/matchups.csv`; this test fails
//! when code drifts from them. Data owner: the plan file owns round numbers
//! and the sword base; `engine-game` owns session defaults; `ladder` owns
//! glory numbers; `loop_text` owns the rows parse; this file owns the matchup
//! parse plus the odd-cycle check. Units: ticks at 60 Hz, round wins, glory
//! counts, seconds as centiseconds. Determinism: pure string scan over
//! included text, no IO at test time. Errors: panics name the culprit key or
//! line; a moved number fails by name.

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
/// Included matchup table (compile-time, so a missing file fails the build).
const MATCHUPS_SRC: &str = include_str!("../../../../docs/design/matchups.csv");

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
        Budgets::default().max_ticks,
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

/// Header the matchup table must carry, in column order: a renamed or moved
/// column fails loudly instead of being read off by position.
const MATCHUPS_HEADER: &str = "attacker,defender,hp_defender,damage_hp,cooldown_s,hit_rate_permille,hits_to_kill,ttk_s,winner";

/// One parsed matchup row: attacker races the defender's hp down.
#[derive(Clone, PartialEq, Eq, Debug)]
struct MatchupRow {
    /// Source line number (1-based) for refusal messages.
    line: usize,
    /// Attacker type name.
    attacker: String,
    /// Defender type name.
    defender: String,
    /// Defender hp in display points.
    hp_defender: u32,
    /// Sword damage in display points per landed hit.
    damage_hp: u32,
    /// Swing cooldown in centiseconds (`0.5` s is `50`).
    cooldown_centis: u32,
    /// Landed-hit rate in permille of swings.
    hit_rate_permille: u32,
    /// Hits needed, `ceil(hp / damage)`.
    hits_to_kill: u32,
    /// Time to kill in centiseconds, analytic from the four cells before it.
    ttk_centis: u32,
    /// Raw winner word: `attacker`, `defender` or `draw`.
    winner: String,
}

/// The parsed matchup table: rows plus every type name seen, in file order.
#[derive(Clone, PartialEq, Eq, Debug)]
struct MatchupSet {
    rows: Vec<MatchupRow>,
    types: Vec<String>,
}

/// Parse a decimal seconds cell into centiseconds (`0.5` -> `50`). Pure:
/// integer math only, so a float never enters the check.
fn centis(text: &str, line: usize, key: &str) -> Result<u32, String> {
    let (whole, frac) = text
        .split_once('.')
        .ok_or_else(|| format!("line {line}: {key} `{text}` must be decimals like 0.5"))?;
    if !is_whole_number(whole) || frac.is_empty() || frac.len() > 2 || !is_whole_number(frac) {
        return Err(format!(
            "line {line}: {key} `{text}` must be whole numbers with up to two decimals"
        ));
    }
    let whole: u32 = whole
        .parse()
        .map_err(|_| format!("line {line}: {key} `{text}` out of range"))?;
    let scale = if frac.len() == 1 { 10 } else { 1 };
    let frac: u32 = frac
        .parse()
        .map_err(|_| format!("line {line}: {key} `{text}` out of range"))?;
    Ok(whole * 100 + frac * scale)
}

/// True when the text is all ASCII digits with at least one digit. Pure.
fn is_whole_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Parse one whole-number cell, naming the line and key on refusal. Pure.
fn u32_cell(value: &str, line: usize, key: &str) -> Result<u32, String> {
    if !is_whole_number(value) {
        return Err(format!(
            "line {line}: {key} `{value}` must be a whole number"
        ));
    }
    value
        .parse()
        .map_err(|_| format!("line {line}: {key} `{value}` out of range"))
}

/// Cells per matchup row (comma separated, the header's own count).
const MATCHUP_CELLS: usize = 9;

/// Parse one data line into a row: hp, damage, cooldown and hit rate, then
/// the two analytic cells recomputed from them. Refuses with the line number,
/// so a hand-edited number fails where it is written. Pure.
fn parse_matchup_row(text: &str, line: usize) -> Result<MatchupRow, String> {
    let cells: Vec<&str> = text.split(',').collect();
    if cells.len() != MATCHUP_CELLS {
        return Err(format!(
            "line {line}: want {MATCHUP_CELLS} comma cells, got {}",
            cells.len()
        ));
    }
    let damage = u32_cell(cells[3], line, "damage_hp")?;
    if damage == 0 {
        return Err(format!("line {line}: damage_hp never 0"));
    }
    let hp = u32_cell(cells[2], line, "hp_defender")?;
    if hp == 0 {
        return Err(format!("line {line}: hp_defender never 0"));
    }
    let cooldown = centis(cells[4], line, "cooldown_s")?;
    let hit_rate = u32_cell(cells[5], line, "hit_rate_permille")?;
    if !(1..=1000).contains(&hit_rate) {
        return Err(format!(
            "line {line}: hit_rate_permille {hit_rate} must fit 1..=1000"
        ));
    }
    let declared_hits = u32_cell(cells[6], line, "hits_to_kill")?;
    let declared_ttk = centis(cells[7], line, "ttk_s")?;
    let hits = hp.div_ceil(damage);
    if declared_hits != hits {
        return Err(format!(
            "line {line}: hits_to_kill {declared_hits} must be ceil({hp}/{damage}) = {hits}"
        ));
    }
    let ttk = (hits * cooldown * 1000 + hit_rate / 2) / hit_rate;
    if declared_ttk != ttk {
        return Err(format!(
            "line {line}: ttk_s {} must be {}.{:02} from {hits} hits x {} centis at {hit_rate} permille",
            cells[7],
            ttk / 100,
            ttk % 100,
            cooldown
        ));
    }
    let winner = cells[8].to_string();
    if !["attacker", "defender", "draw"].contains(&winner.as_str()) {
        return Err(format!(
            "line {line}: winner `{winner}` must be attacker, defender or draw"
        ));
    }
    let attacker = cells[0].to_string();
    let defender = cells[1].to_string();
    if attacker == defender {
        if winner != "draw" {
            return Err(format!("line {line}: a self row must draw, got `{winner}`"));
        }
    } else if winner == "draw" {
        return Err(format!(
            "line {line}: distinct types must decide, `{attacker}` vs `{defender}` drew"
        ));
    }
    Ok(MatchupRow {
        line,
        attacker,
        defender,
        hp_defender: hp,
        damage_hp: damage,
        cooldown_centis: cooldown,
        hit_rate_permille: hit_rate,
        hits_to_kill: hits,
        ttk_centis: ttk,
        winner,
    })
}

/// Parse the header plus every data row of the table. A missing header, a
/// renamed column or an empty table refuses with its line. Pure.
fn parse_matchup_rows(src: &str) -> Result<Vec<MatchupRow>, String> {
    let mut rows: Vec<MatchupRow> = Vec::new();
    let mut header = false;
    for (index, raw) in src.lines().enumerate() {
        let line = index + 1;
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if !header {
            if text != MATCHUPS_HEADER {
                return Err(format!(
                    "line {line}: header must be `{MATCHUPS_HEADER}`, got `{text}`"
                ));
            }
            header = true;
            continue;
        }
        rows.push(parse_matchup_row(text, line)?);
    }
    if !header {
        return Err("matchups table has no header row".to_string());
    }
    if rows.is_empty() {
        return Err("matchups table has no data rows".to_string());
    }
    Ok(rows)
}

/// Every type name in file order, first sighting wins. Pure.
fn matchup_types(rows: &[MatchupRow]) -> Vec<String> {
    let mut types: Vec<String> = Vec::new();
    for row in rows {
        for name in [&row.attacker, &row.defender] {
            if !types.contains(name) {
                types.push(name.clone());
            }
        }
    }
    types
}

/// The cycle shape: an odd type count, one row per direction with both
/// directions agreeing on the winner, and every type one win and one loss.
/// Refuses with the line or the type name, never a panic. Pure.
fn check_matchup_cycle(rows: &[MatchupRow], types: &[String]) -> Result<(), String> {
    if types.len().is_multiple_of(2) {
        return Err(format!(
            "want an odd number of types (an odd cycle), got {}",
            types.len()
        ));
    }
    // Every decided pair carries one row per direction and the two rows must
    // agree on the winner: a table where both directions win is not a cycle.
    let mut beats: Vec<(String, u32)> = types.iter().map(|t| (t.clone(), 0)).collect();
    let mut loses: Vec<(String, u32)> = types.iter().map(|t| (t.clone(), 0)).collect();
    let mut decided = 0usize;
    let mut selfs = 0usize;
    for row in rows {
        if row.attacker == row.defender {
            selfs += 1;
            continue;
        }
        let mirror = rows
            .iter()
            .filter(|other| other.attacker == row.defender && other.defender == row.attacker)
            .collect::<Vec<_>>();
        if mirror.len() != 1 {
            return Err(format!(
                "line {}: `{}` vs `{}` needs exactly one reverse row, got {}",
                row.line,
                row.attacker,
                row.defender,
                mirror.len()
            ));
        }
        // Count each unordered pair once, from its low name first.
        if row.attacker > row.defender {
            continue;
        }
        let reverse = mirror[0];
        let agrees = (row.winner == "attacker" && reverse.winner == "defender")
            || (row.winner == "defender" && reverse.winner == "attacker");
        if !agrees {
            return Err(format!(
                "lines {} and {}: `{}` vs `{}` disagree on the winner (`{}` vs `{}`)",
                row.line, reverse.line, row.attacker, row.defender, row.winner, reverse.winner
            ));
        }
        let (winner, loser) = if row.winner == "attacker" {
            (&row.attacker, &row.defender)
        } else {
            (&row.defender, &row.attacker)
        };
        for slot in beats.iter_mut() {
            if &slot.0 == winner {
                slot.1 += 1;
            }
        }
        for slot in loses.iter_mut() {
            if &slot.0 == loser {
                slot.1 += 1;
            }
        }
        decided += 1;
    }
    let want_pairs = types.len() * (types.len() - 1) / 2;
    if decided != want_pairs {
        return Err(format!(
            "want {want_pairs} decided pairs over {} types, got {decided}",
            types.len()
        ));
    }
    if selfs != types.len() {
        return Err(format!(
            "want one draw row per type ({}), got {selfs}",
            types.len()
        ));
    }
    for (name, won) in &beats {
        let lost = loses
            .iter()
            .find(|slot| &slot.0 == name)
            .map_or(0, |slot| slot.1);
        if *won != 1 || lost != 1 {
            return Err(format!(
                "type `{name}` beats {won} and loses {lost}, want one each"
            ));
        }
    }
    Ok(())
}

/// Parse the matchup table: rows, type names, then the shape checks a designer
/// breaks by hand. Refuses with the line or the type name, never a panic, so
/// the negative cases below can run on a mutated copy of the shipped text.
fn parse_matchups(src: &str) -> Result<MatchupSet, String> {
    let rows = parse_matchup_rows(src)?;
    let types = matchup_types(&rows);
    check_matchup_cycle(&rows, &types)?;
    Ok(MatchupSet { rows, types })
}

/// Read a decimal plan-spec number as centiseconds, naming the key on a miss.
fn spec_centis(key: &str) -> u32 {
    centis(token_after(PLAN_SPEC_SRC, key), 0, key)
        .unwrap_or_else(|err| panic!("plan spec {key}: {err}"))
}

#[test]
fn matchups_hold_an_odd_cycle_over_the_plan_sword() {
    let set = parse_matchups(MATCHUPS_SRC).expect("shipped matchups parse");
    assert_eq!(
        set.types,
        ["hero", "recruit_T1", "veteran_T2"],
        "the cycle is hero, recruit_T1, veteran_T2"
    );
    assert_eq!(
        set.types.len() % 2,
        1,
        "an odd number of types keeps every type one win and one loss"
    );
    let sword_damage = spec_u64("damage_hp");
    let sword_cooldown = spec_centis("cooldown_s");
    let hero_hp = spec_u64("hp");
    for row in &set.rows {
        assert_eq!(
            u64::from(row.damage_hp),
            sword_damage,
            "line {}: every row fights with the plan sword",
            row.line
        );
        assert_eq!(
            row.cooldown_centis, sword_cooldown,
            "line {}: every row swings on the plan cooldown",
            row.line
        );
        assert!(
            (1..=1000).contains(&row.hit_rate_permille),
            "line {}: hit rate {} must fit 1..=1000",
            row.line,
            row.hit_rate_permille
        );
        assert!(
            row.damage_hp > 0 && row.hits_to_kill > 0 && row.ttk_centis > 0,
            "line {}: a bloodless row is not a matchup",
            row.line
        );
        if row.defender == "hero" {
            assert_eq!(
                u64::from(row.hp_defender),
                hero_hp,
                "line {}: the hero keeps the plan hp",
                row.line
            );
        }
    }
    // The cycle named in the file's own header comment, row by row.
    for (attacker, defender, wins) in [
        ("hero", "recruit_T1", true),
        ("hero", "veteran_T2", false),
        ("recruit_T1", "veteran_T2", true),
    ] {
        let row = set
            .rows
            .iter()
            .find(|row| row.attacker == attacker && row.defender == defender)
            .unwrap_or_else(|| panic!("missing pair {attacker} vs {defender}"));
        assert_eq!(
            row.winner == "attacker",
            wins,
            "line {}: {attacker} vs {defender} moved the cycle",
            row.line
        );
    }
}

#[test]
fn matchups_parser_refuses_a_broken_table() {
    // The bar itself: the shipped table passes...
    assert!(
        parse_matchups(MATCHUPS_SRC).is_ok(),
        "shipped table must pass"
    );
    // ...and each hand-broken copy below trips it, so the bar can fail.
    let drifted_ttk = MATCHUPS_SRC.replace(
        "hero,veteran_T2,110,25,0.5,400,5,6.25,defender",
        "hero,veteran_T2,110,25,0.5,600,5,6.25,defender",
    );
    assert!(
        parse_matchups(&drifted_ttk).is_err(),
        "a ttk that ignores the hit rate must refuse"
    );
    let drifted_hits = MATCHUPS_SRC.replace(
        "recruit_T1,recruit_T1,90,25,0.5,500,4,4.00,draw",
        "recruit_T1,recruit_T1,90,25,0.5,500,5,4.00,draw",
    );
    assert!(
        parse_matchups(&drifted_hits).is_err(),
        "hits_to_kill must follow hp and damage"
    );
    let broken_cycle = MATCHUPS_SRC.replace(
        "recruit_T1,hero,100,25,0.5,400,4,5.00,defender",
        "recruit_T1,hero,100,25,0.5,400,4,5.00,attacker",
    );
    assert!(
        parse_matchups(&broken_cycle).is_err(),
        "a pair whose two directions disagree must refuse"
    );
    let renamed =
        MATCHUPS_SRC.replace("cooldown_s,hit_rate_permille", "cooldown,hit_rate_permille");
    assert!(
        parse_matchups(&renamed).is_err(),
        "a renamed column must refuse"
    );
    let fourth = std::format!("{MATCHUPS_SRC}\nhero,rookie_T9,95,25,0.5,500,4,4.00,attacker");
    assert!(
        parse_matchups(&fourth).is_err(),
        "a fourth type with no cycle must refuse"
    );
}

/// The four sources that state the kill-share ceiling, each included at compile
/// time so a moved file fails the build instead of passing unread: the live
/// `pq2-balance-tournament` gate, the feel survey, this crate's mirror and the
/// endpoint's copy. `engine-game` can import neither `engine-playtest` nor
/// `engine-endpoint`, so the two invisible copies are pinned by their text.
const TOURNAMENT_BAR_SRC: &str = include_str!("../../../playtest/src/tournament.rs");
/// Feel direct-hit survey copy of the bar (same crate dependency as `bench`).
const FEEL_BAR_SRC: &str = include_str!("../../../combat/src/feel.rs");
/// This crate's own mirror of the bar (the const the bench checks).
const BENCH_BAR_SRC: &str = include_str!("../../src/bench.rs");
/// Endpoint copy of the bar (the AB3 fork-check refusal bound).
const BUILD_ENCOUNTER_BAR_SRC: &str = include_str!("../../../endpoint/src/build_encounter.rs");

/// The whole-number value of `pub const <name>: u16 = <value>;` in one source
/// text, naming the miss. Pure: integer math, no crate import, so a bar that
/// lives in a crate this test cannot depend on is still pinned. Refuses, never
/// panics, so the mutated copies below can run on the shipped text.
fn const_u16_in(src: &str, name: &str) -> Result<u16, String> {
    let needle = std::format!("pub const {name}: u16 =");
    let start = src
        .find(needle.as_str())
        .ok_or_else(|| std::format!("no `{needle}` in this source"))?;
    let rest = &src[start.saturating_add(needle.len())..];
    let end = rest
        .find(';')
        .ok_or_else(|| std::format!("`{name}` has no `;`"))?;
    let value = rest[..end].trim();
    if !is_whole_number(value) {
        return Err(std::format!(
            "`{name}` value `{value}` is not a whole number"
        ));
    }
    value
        .parse()
        .map_err(|_| std::format!("`{name}` value `{value}` out of range"))
}

#[test]
fn kill_share_bar_is_one_number_across_crates() {
    // One bar, no second number: every copy of the 350 kill-share ceiling reads
    // back the same value as this crate's mirror, whatever the crate.
    let bar = engine_game::bench::PQ2_KILL_SHARE_MAX_PERMILLE;
    assert_eq!(
        bar,
        engine_combat::feel::KILL_SHARE_MAX_PERMILLE,
        "combat's kill-share bar drifted from the bench mirror"
    );
    for (file, name, src) in [
        (
            "crates/playtest/src/tournament.rs",
            "KILL_SHARE_MAX_PERMILLE",
            TOURNAMENT_BAR_SRC,
        ),
        (
            "crates/combat/src/feel.rs",
            "KILL_SHARE_MAX_PERMILLE",
            FEEL_BAR_SRC,
        ),
        (
            "crates/endpoint/src/build_encounter.rs",
            "KILL_SHARE_MAX_PERMILLE",
            BUILD_ENCOUNTER_BAR_SRC,
        ),
        (
            "crates/game/src/bench.rs",
            "PQ2_KILL_SHARE_MAX_PERMILLE",
            BENCH_BAR_SRC,
        ),
    ] {
        assert_eq!(
            const_u16_in(src, name),
            Ok(bar),
            "{file}: `{name}` is not the one kill-share bar"
        );
    }
    // The bar can fail: move the gate's number and the scan reads the new one.
    let drifted = TOURNAMENT_BAR_SRC.replace(
        "pub const KILL_SHARE_MAX_PERMILLE: u16 = 350;",
        "pub const KILL_SHARE_MAX_PERMILLE: u16 = 1000;",
    );
    assert_ne!(drifted, TOURNAMENT_BAR_SRC, "the drift mutation must hit");
    assert_eq!(
        const_u16_in(&drifted, "KILL_SHARE_MAX_PERMILLE"),
        Ok(1000u16),
        "a moved gate bar must be read as moved, or the guard is blind"
    );
    // A float bar or a missing item refuses instead of passing.
    let floaty = TOURNAMENT_BAR_SRC.replace(
        "pub const KILL_SHARE_MAX_PERMILLE: u16 = 350;",
        "pub const KILL_SHARE_MAX_PERMILLE: u16 = 35.0;",
    );
    assert!(
        const_u16_in(&floaty, "KILL_SHARE_MAX_PERMILLE").is_err(),
        "a float bar must refuse"
    );
    assert!(const_u16_in("fn nope() {}", "KILL_SHARE_MAX_PERMILLE").is_err());
}
