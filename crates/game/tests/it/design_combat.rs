//! Design combat frame data: readable tells and answers as checks.
//!
//! Consumer: the designer and the gameplay seat read these numbers, not prose.
//! Data owner: this test owns its tapes; `engine-combat` owns the fight step
//! and the versioned rows. Units: ticks at 60 Hz, damage in permille.
//! Determinism scope: same rows plus same tape give the same events.
//! Errors: fresh [`FightState`] only, so no construction failure beyond the
//! two-fighter cap this file always satisfies.

use engine_combat::fight::{FightEvent, FightState, TABLES_V1, TABLES_V2};
use engine_combat::step_with_tables;
use engine_input::fighter::{ATTACK, BLOCK, DODGE};
use engine_input::{Buttons, FighterId, FighterInput};

/// One input for a slot with neutral stick and facing. Pure.
fn input(fighter: u8, buttons: u8) -> FighterInput {
    FighterInput {
        tick: 0,
        fighter: FighterId::new(fighter).expect("slot 0 or 1 is valid"),
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: Buttons(buttons),
    }
}

/// Step one tick with explicit buttons for both slots. Pure harness.
fn step_pair(
    state: &mut FightState,
    tables: &engine_combat::fight::Tables,
    a: u8,
    b: u8,
) -> Vec<FightEvent> {
    step_with_tables(&[input(0, a), input(1, b)], state, tables)
}

/// True when events hold the named outcome from slot 0 to slot 1. Pure.
fn has(events: &[FightEvent], kind: &str) -> bool {
    events.iter().any(|event| {
        let attacker_ok = matches!(
            event,
            FightEvent::Hit { attacker: 0, .. }
                | FightEvent::Blocked { attacker: 0, .. }
                | FightEvent::Parried { attacker: 0, .. }
                | FightEvent::Dodged { attacker: 0, .. }
        );
        let victim_ok = matches!(
            event,
            FightEvent::Hit { victim: 1, .. }
                | FightEvent::Blocked { victim: 1, .. }
                | FightEvent::Parried { victim: 1, .. }
                | FightEvent::Dodged { victim: 1, .. }
        );
        let kind_ok = match kind {
            "hit" => matches!(event, FightEvent::Hit { .. }),
            "blocked" => matches!(event, FightEvent::Blocked { .. }),
            "parried" => matches!(event, FightEvent::Parried { .. }),
            "dodged" => matches!(event, FightEvent::Dodged { .. }),
            _ => false,
        };
        attacker_ok && victim_ok && kind_ok
    })
}

#[test]
fn v2_startup_is_readable() {
    assert!(
        TABLES_V2.attack_startup_ticks >= 15,
        "v2 startup {} must be 15 ticks or more (0.25 s tell)",
        TABLES_V2.attack_startup_ticks
    );
}

#[test]
fn v2_parry_window_shorter_than_startup() {
    assert!(
        TABLES_V2.parry_window_ticks < TABLES_V2.attack_startup_ticks,
        "v2 parry {} must sit below startup {}",
        TABLES_V2.parry_window_ticks,
        TABLES_V2.attack_startup_ticks
    );
    assert!(
        TABLES_V2.parry_window_ticks <= 8,
        "v2 parry {} must be 8 ticks or fewer",
        TABLES_V2.parry_window_ticks
    );
}

#[test]
fn dodge_iframes_inside_duration() {
    assert!(
        TABLES_V1.dodge_duration_ticks > 0,
        "v1 dodge must last at least one tick"
    );
    assert!(
        TABLES_V1.dodge_iframes_ticks <= TABLES_V1.dodge_duration_ticks,
        "v1 i-frames {} must sit inside duration {}",
        TABLES_V1.dodge_iframes_ticks,
        TABLES_V1.dodge_duration_ticks
    );
    assert!(
        TABLES_V2.dodge_duration_ticks > 0,
        "v2 dodge must last at least one tick"
    );
    assert!(
        TABLES_V2.dodge_iframes_ticks <= TABLES_V2.dodge_duration_ticks,
        "v2 i-frames {} must sit inside duration {}",
        TABLES_V2.dodge_iframes_ticks,
        TABLES_V2.dodge_duration_ticks
    );
}

#[test]
fn hit_tape_gives_hit() {
    let tables = &TABLES_V1;
    let mut state = FightState::new(2).expect("two fighters fit");
    let horizon = tables.attack_startup_ticks + tables.attack_recovery_ticks + 6;
    let mut seen = false;
    for step in 0..horizon {
        let attacker = if step == 0 { ATTACK } else { 0 };
        let events = step_pair(&mut state, tables, attacker, 0);
        if has(&events, "hit") {
            seen = true;
            break;
        }
    }
    assert!(seen, "idle victim takes a clean hit");
}

#[test]
fn blocked_tape_gives_blocked() {
    let tables = &TABLES_V1;
    let mut state = FightState::new(2).expect("two fighters fit");
    // Victim holds block from the first tick; the attack starts late so the
    // hold has aged out of the parry window before impact.
    let press_at = 6;
    let horizon = press_at + tables.attack_startup_ticks + tables.attack_recovery_ticks + 6;
    let mut seen = false;
    for step in 0..horizon {
        let attacker = if step == press_at { ATTACK } else { 0 };
        let events = step_pair(&mut state, tables, attacker, BLOCK);
        if has(&events, "blocked") {
            seen = true;
            break;
        }
    }
    assert!(seen, "late attack into a held block chips");
}

#[test]
fn parried_tape_gives_parried() {
    let tables = &TABLES_V1;
    let mut state = FightState::new(2).expect("two fighters fit");
    // Victim presses block two ticks after the attack, so the hold is still
    // inside the parry window when startup completes.
    let horizon = tables.attack_startup_ticks + tables.attack_recovery_ticks + 6;
    let mut seen = false;
    for step in 0..horizon {
        let attacker = if step == 0 { ATTACK } else { 0 };
        let victim = if step >= 2 { BLOCK } else { 0 };
        let events = step_pair(&mut state, tables, attacker, victim);
        if has(&events, "parried") {
            seen = true;
            break;
        }
    }
    assert!(seen, "block pressed near impact parries");
}

#[test]
fn dodged_tape_gives_dodged() {
    let tables = &TABLES_V1;
    let mut state = FightState::new(2).expect("two fighters fit");
    let horizon = tables.attack_startup_ticks + tables.attack_recovery_ticks + 6;
    let mut seen = false;
    for step in 0..horizon {
        let attacker = if step == 0 { ATTACK } else { 0 };
        let victim = if step == 0 { DODGE } else { 0 };
        let events = step_pair(&mut state, tables, attacker, victim);
        if has(&events, "dodged") {
            seen = true;
            break;
        }
    }
    assert!(seen, "dodge i-frames negate the hit");
}

#[test]
fn stamina_out_tape_gives_stamina_out() {
    let tables = &TABLES_V1;
    let mut state = FightState::new(2).expect("two fighters fit");
    state.fighters[0].stamina = 0;
    let events = step_pair(&mut state, tables, ATTACK, 0);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, FightEvent::StaminaOut { fighter: 0 })),
        "attack with no stamina reports StaminaOut"
    );
}

/// Included weapon table text (compile-time, so a missing file fails the build).
const WEAPONS_SRC: &str = include_str!("../../../../content/design/weapons.rows");

/// One parsed weapon row with its source line. Pure data.
struct WeaponRow {
    /// Row id, e.g. `gladius`.
    id: &'static str,
    /// Source line number (1-based) for refusal messages.
    line: usize,
    /// Weapon type code: 1 dagger thrust, 2 sword cut, 3 spear thrust.
    weapon_type: u32,
    /// Damage in permille, `1..=1000`.
    damage_permille: u32,
    /// Startup ticks at 60 Hz, fewer is faster.
    speed_ticks: u32,
    /// Hands to wield: 1 or 2.
    hands: u32,
    /// Poise hold in permille, `0..=1000`.
    stagger_permille: u32,
    /// Stamina cost in points, `0..=100`.
    stamina_cost: u32,
    /// Answer from the player's kit: 1 parry, 2 dodge, 3 block.
    counter: u32,
}

/// True when the text is all ASCII digits with at least one digit. Pure.
fn weapons_is_whole_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Parse one `u32` whole number or panic with the line number. Pure.
fn weapons_parse_u32(value: &str, line: usize, key: &str) -> u32 {
    assert!(
        weapons_is_whole_number(value),
        "line {line}: {key} must be a whole number, got `{value}`"
    );
    value
        .parse::<u32>()
        .unwrap_or_else(|_| panic!("line {line}: {key} out of range"))
}

/// Parse the table header, returning the version. Fails on a bad header.
fn weapons_parse_version(first: &str) -> u64 {
    let parts: Vec<&str> = first.split_whitespace().collect();
    assert!(
        parts.len() == 4 && parts[0] == "table" && parts[1] == "weapons" && parts[2] == "version",
        "line 1 must be `table weapons version <n>`, got `{first}`"
    );
    assert!(
        weapons_is_whole_number(parts[3]),
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
fn weapons_parse_line(line_text: &str, line: usize) -> WeaponRow {
    let parts: Vec<&str> = line_text.split_whitespace().collect();
    assert!(parts.len() >= 2, "line {line}: need `<id> <key>=<int> ...`");
    let id_box: &'static str = Box::leak(parts[0].to_string().into_boxed_str());
    let mut weapon_type: Option<u32> = None;
    let mut damage: Option<u32> = None;
    let mut speed: Option<u32> = None;
    let mut hands: Option<u32> = None;
    let mut stagger: Option<u32> = None;
    let mut stamina: Option<u32> = None;
    let mut counter: Option<u32> = None;
    for field in &parts[1..] {
        let (k, v) = field
            .split_once('=')
            .unwrap_or_else(|| panic!("line {line}: field `{field}` needs `<key>=<int>`"));
        let parsed = weapons_parse_u32(v, line, k);
        match k {
            "type" => weapon_type = Some(parsed),
            "damage_permille" => damage = Some(parsed),
            "speed_ticks" => speed = Some(parsed),
            "hands" => hands = Some(parsed),
            "stagger_permille" => stagger = Some(parsed),
            "stamina_cost" => stamina = Some(parsed),
            "counter" => counter = Some(parsed),
            _ => panic!("line {line}: unknown key `{k}`"),
        }
    }
    let need = |name: &str, slot: Option<u32>| {
        slot.unwrap_or_else(|| panic!("line {line}: weapon `{id_box}` misses `{name}`"))
    };
    WeaponRow {
        id: id_box,
        line,
        weapon_type: need("type", weapon_type),
        damage_permille: need("damage_permille", damage),
        speed_ticks: need("speed_ticks", speed),
        hands: need("hands", hands),
        stagger_permille: need("stagger_permille", stagger),
        stamina_cost: need("stamina_cost", stamina),
        counter: need("counter", counter),
    }
}

/// Parse the whole weapons table: version plus one row per weapon. Pure.
fn weapons_parse_table(src: &str) -> (u64, Vec<WeaponRow>) {
    let mut logical: Vec<(usize, String)> = Vec::new();
    for (index, raw) in src.lines().enumerate() {
        let cut = raw.split('#').next().unwrap_or("");
        let trimmed = cut.trim();
        if trimmed.is_empty() {
            continue;
        }
        logical.push((index + 1, trimmed.to_string()));
    }
    assert!(!logical.is_empty(), "weapons.rows is empty");
    let version = weapons_parse_version(&logical[0].1);
    let mut rows = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (line, text) in logical.iter().skip(1) {
        let row = weapons_parse_line(text, *line);
        assert!(
            !seen.contains(&row.id),
            "line {line}: duplicate weapon `{}`",
            row.id
        );
        seen.push(row.id);
        rows.push(row);
    }
    rows.sort_by_key(|row| row.speed_ticks);
    (version, rows)
}

#[test]
fn weapons_header_holds_four() {
    let (version, rows) = weapons_parse_table(WEAPONS_SRC);
    assert!(version != 0, "version never 0");
    assert_eq!(rows.len(), 4, "want four Rome weapons, got {}", rows.len());
    for row in &rows {
        assert!(
            (1..=3).contains(&row.weapon_type),
            "line {}: type {} must be 1 dagger, 2 sword or 3 spear",
            row.line,
            row.weapon_type
        );
        assert!(
            (1..=1000).contains(&row.damage_permille),
            "line {}: damage_permille {} must sit in 1..=1000",
            row.line,
            row.damage_permille
        );
        assert!(
            (1..=60).contains(&row.speed_ticks),
            "line {}: speed_ticks {} must sit in 1..=60",
            row.line,
            row.speed_ticks
        );
        assert!(
            (1..=2).contains(&row.hands),
            "line {}: hands {} must be 1 or 2",
            row.line,
            row.hands
        );
        assert!(
            row.stagger_permille <= 1000,
            "line {}: stagger_permille {} must fit 0..=1000",
            row.line,
            row.stagger_permille
        );
        assert!(
            row.stamina_cost <= 100,
            "line {}: stamina_cost {} must fit 0..=100",
            row.line,
            row.stamina_cost
        );
    }
}

#[test]
fn fast_weapons_hit_for_less() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    assert!(
        rows.len() >= 2,
        "need at least two weapons to compare speed"
    );
    for pair in rows.windows(2) {
        let (fast, slow) = (&pair[0], &pair[1]);
        assert!(
            fast.speed_ticks <= slow.speed_ticks,
            "rows must sort by speed_ticks"
        );
        assert!(
            fast.damage_permille < slow.damage_permille,
            "fast {} (speed {} damage {}) must hit for less than slow {} (speed {} damage {})",
            fast.id,
            fast.speed_ticks,
            fast.damage_permille,
            slow.id,
            slow.speed_ticks,
            slow.damage_permille
        );
        assert!(
            fast.stamina_cost <= slow.stamina_cost,
            "fast {} (stamina {}) must not cost more than slow {} (stamina {})",
            fast.id,
            fast.stamina_cost,
            slow.id,
            slow.stamina_cost
        );
    }
}

#[test]
fn every_weapon_has_a_counter() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    for row in &rows {
        assert!(
            (1..=3).contains(&row.counter),
            "line {}: counter {} must be 1 parry, 2 dodge or 3 block",
            row.line,
            row.counter
        );
    }
}
