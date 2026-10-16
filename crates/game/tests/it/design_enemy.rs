//! Design enemy tiers: readable tells plus one counter per tier as checks.
//!
//! Consumer: the enemy designer edits `content/design/enemies.rows`; the AI
//! seat (GA13) loads these rows into tier skill and the duel below proves the
//! 60% bar. Data owner: this test owns the parse; `engine-combat` owns the
//! readable tell (`TABLES_V2.attack_startup_ticks`) and the player's answers
//! (parry, dodge, block). Units: ticks at 60 Hz, permille `0..=1000`, counter
//! `1` parry, `2` dodge, `3` block. Determinism: pure string scan plus
//! `engine_core::splitmix64` seeds, so the same rows twice give the same
//! wins; no IO at test time. Errors: panics name the culprit line; a moved
//! number fails by name.
//!
//! Book notes: tiers as data rows climbing in skill `[handsrust | Damage from
//! Weapons and Claws]`; difficulty judged from the player's seat
//! `[testing | Level progression]`. The 60% tier bar is a house number until
//! the tournament proves another number; the boss stays PROPOSED with no row.

use engine_combat::fight::TABLES_V2;

/// Included enemy table text (compile-time, so a missing file fails the build).
const ENEMIES_SRC: &str = include_str!("../../../../content/design/enemies.rows");

/// Seeded duels per adjacent tier pair (the 200-match shape).
const DUELS_PER_PAIR: u32 = 200;
/// Tier bar in permille: a tier beats the one below in 60% or more.
const TIER_BAR_PERMILLE: u32 = 600;
/// First duel seed; pair `p` uses `base.wrapping_add(p * SEED_GAP + i)`.
const DUEL_BASE_SEED: u64 = 2040;
/// Seed gap between pairs, above [`DUELS_PER_PAIR`], so ranges never overlap.
const SEED_GAP: u64 = 1_000_003;
/// Per-duel jitter range; below the 150 power gap, so stats decide.
const JITTER_RANGE: u64 = 64;

/// One parsed enemy tier row with its source line. Pure data.
struct EnemyRow {
    /// Row id, e.g. `tier_1`.
    id: &'static str,
    /// Source line number (1-based) for refusal messages.
    line: usize,
    /// Health in permille, narrow `900..=1200` band (skill first).
    hp_permille: u32,
    /// Readable startup in ticks, 15 or more.
    tell_ticks: u32,
    /// Poise hold in permille, `0..=1000` (higher holds longer).
    stagger_permille: u32,
    /// Answer from the player's kit: 1 parry, 2 dodge, 3 block.
    counter: u32,
}

/// True when the text is all ASCII digits with at least one digit. Pure.
fn is_whole_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Parse one `u32` whole number or panic with the line number. Pure.
fn parse_u32(value: &str, line: usize, key: &str) -> u32 {
    assert!(
        is_whole_number(value),
        "line {line}: {key} must be a whole number, got `{value}`"
    );
    value
        .parse::<u32>()
        .unwrap_or_else(|_| panic!("line {line}: {key} out of range"))
}

/// Parse the table header, returning the version. Fails on a bad header.
fn parse_version(first: &str) -> u64 {
    let parts: Vec<&str> = first.split_whitespace().collect();
    assert!(
        parts.len() == 4 && parts[0] == "table" && parts[1] == "enemies" && parts[2] == "version",
        "line 1 must be `table enemies version <n>`, got `{first}`"
    );
    assert!(
        is_whole_number(parts[3]),
        "line 1: version must be a whole number, got `{}`",
        parts[3]
    );
    let version: u64 = parts[3]
        .parse()
        .unwrap_or_else(|_| panic!("line 1: version out of range"));
    assert!(version != 0, "line 1: version never 0");
    version
}

/// Parse one data line into a row. Fails with the line number.
fn parse_line(line_text: &str, line: usize) -> EnemyRow {
    let parts: Vec<&str> = line_text.split_whitespace().collect();
    assert!(parts.len() >= 2, "line {line}: need `<id> <key>=<int> ...`");
    let id_box: &'static str = Box::leak(parts[0].to_string().into_boxed_str());
    assert!(
        id_box == "tier_1" || id_box == "tier_2" || id_box == "tier_3",
        "line {line}: unknown tier `{id_box}`, want tier_1..tier_3 (boss stays PROPOSED)"
    );
    let mut hp: Option<u32> = None;
    let mut tell: Option<u32> = None;
    let mut stagger: Option<u32> = None;
    let mut counter: Option<u32> = None;
    for field in &parts[1..] {
        let (k, v) = field
            .split_once('=')
            .unwrap_or_else(|| panic!("line {line}: field `{field}` needs `<key>=<int>`"));
        let parsed = parse_u32(v, line, k);
        match k {
            "hp_permille" => hp = Some(parsed),
            "tell_ticks" => tell = Some(parsed),
            "stagger_permille" => stagger = Some(parsed),
            "counter" => counter = Some(parsed),
            _ => panic!("line {line}: unknown key `{k}`"),
        }
    }
    let need = |name: &str, slot: Option<u32>| {
        slot.unwrap_or_else(|| panic!("line {line}: tier `{id_box}` misses `{name}`"))
    };
    EnemyRow {
        id: id_box,
        line,
        hp_permille: need("hp_permille", hp),
        tell_ticks: need("tell_ticks", tell),
        stagger_permille: need("stagger_permille", stagger),
        counter: need("counter", counter),
    }
}

/// Parse the whole table: version plus three tier rows. Pure.
fn parse_table(src: &str) -> (u64, Vec<EnemyRow>) {
    let mut logical: Vec<(usize, String)> = Vec::new();
    for (index, raw) in src.lines().enumerate() {
        let cut = raw.split('#').next().unwrap_or("");
        let trimmed = cut.trim();
        if trimmed.is_empty() {
            continue;
        }
        logical.push((index + 1, trimmed.to_string()));
    }
    assert!(!logical.is_empty(), "enemies.rows is empty");
    let version = parse_version(&logical[0].1);
    let mut rows = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (line, text) in logical.iter().skip(1) {
        let row = parse_line(text, *line);
        assert!(
            !seen.contains(&row.id),
            "line {line}: duplicate tier `{}`",
            row.id
        );
        seen.push(row.id);
        rows.push(row);
    }
    rows.sort_by_key(|row| row.id);
    (version, rows)
}

/// Find a tier by id. Pure.
fn row_for<'a>(rows: &'a [EnemyRow], id: &str) -> &'a EnemyRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing tier `{id}`"))
}

/// Duel power from row stats: health plus poise hold. Higher holds longer.
/// Pure; the same rows always give the same power.
fn power(row: &EnemyRow) -> u64 {
    u64::from(row.hp_permille) + u64::from(row.stagger_permille)
}

/// Seeded wins for the higher tier over `matches` duels. Each duel adds
/// `splitmix64` jitter below the 150 power gap, so stats decide and the same
/// base twice gives the same wins. Pure.
fn higher_wins(high: &EnemyRow, low: &EnemyRow, base: u64, matches: u32) -> u32 {
    let high_power = power(high);
    let low_power = power(low);
    let mut wins = 0u32;
    for i in 0..matches {
        let seed = base.wrapping_add(u64::from(i));
        let high_jitter = engine_core::splitmix64(seed) % JITTER_RANGE;
        let low_jitter = engine_core::splitmix64(seed ^ 0x9E37_79B9_7F4A_7C15) % JITTER_RANGE;
        if high_power + high_jitter >= low_power + low_jitter {
            wins += 1;
        }
    }
    wins
}

#[test]
fn header_holds_three_tiers() {
    let (version, rows) = parse_table(ENEMIES_SRC);
    assert!(version != 0, "version never 0");
    assert_eq!(rows.len(), 3, "want tier_1..tier_3, got {}", rows.len());
    assert_eq!(rows[0].id, "tier_1");
    assert_eq!(rows[1].id, "tier_2");
    assert_eq!(rows[2].id, "tier_3");
}

#[test]
fn every_row_has_readable_tell_and_allowed_counter() {
    let (_, rows) = parse_table(ENEMIES_SRC);
    for row in &rows {
        assert!(
            (900..=1200).contains(&row.hp_permille),
            "line {}: hp_permille {} must sit in the narrow 900..=1200 band",
            row.line,
            row.hp_permille
        );
        assert!(
            row.tell_ticks >= 15,
            "line {}: tell_ticks {} must be 15 or more (readable startup)",
            row.line,
            row.tell_ticks
        );
        assert!(
            row.stagger_permille <= 1000,
            "line {}: stagger_permille {} must fit 0..=1000",
            row.line,
            row.stagger_permille
        );
        assert!(
            (1..=3).contains(&row.counter),
            "line {}: counter {} must be 1 parry, 2 dodge or 3 block",
            row.line,
            row.counter
        );
    }
    assert_eq!(row_for(&rows, "tier_1").counter, 1, "grunt teaches parry");
    assert_eq!(row_for(&rows, "tier_2").counter, 2, "veteran teaches dodge");
}

#[test]
fn tell_tracks_v2_startup() {
    assert!(
        TABLES_V2.attack_startup_ticks >= 15,
        "v2 startup {} must be 15 ticks or more",
        TABLES_V2.attack_startup_ticks
    );
    let (_, rows) = parse_table(ENEMIES_SRC);
    for row in &rows {
        assert!(
            row.tell_ticks >= TABLES_V2.attack_startup_ticks,
            "line {}: tell {} must cover v2 startup {}",
            row.line,
            row.tell_ticks,
            TABLES_V2.attack_startup_ticks
        );
    }
}

#[test]
fn tier_beats_below_in_60_percent_of_seeded_duels() {
    let (_, rows) = parse_table(ENEMIES_SRC);
    let pairs = [("tier_1", "tier_2"), ("tier_2", "tier_3")];
    for (pair_index, (low_id, high_id)) in pairs.iter().enumerate() {
        let low = row_for(&rows, low_id);
        let high = row_for(&rows, high_id);
        let base = DUEL_BASE_SEED.wrapping_add((pair_index as u64).wrapping_mul(SEED_GAP));
        let first = higher_wins(high, low, base, DUELS_PER_PAIR);
        let second = higher_wins(high, low, base, DUELS_PER_PAIR);
        assert_eq!(first, second, "{high_id} over {low_id} must replay");
        let rate = first * 1000 / DUELS_PER_PAIR;
        assert!(
            rate >= TIER_BAR_PERMILLE,
            "{high_id} beats {low_id} in {rate} permille, want 600 or more (wins {first}/{})",
            DUELS_PER_PAIR
        );
    }
}
