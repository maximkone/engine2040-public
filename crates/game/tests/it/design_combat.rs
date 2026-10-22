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
#[expect(
    clippy::assertions_on_constants,
    reason = "table pin: a retune must fail this test with its message"
)]
fn v2_startup_is_readable() {
    assert!(
        TABLES_V2.attack_startup_ticks >= 15,
        "v2 startup {} must be 15 ticks or more (0.25 s tell)",
        TABLES_V2.attack_startup_ticks
    );
}

#[test]
#[expect(
    clippy::assertions_on_constants,
    reason = "table pin: a retune must fail this test with its message"
)]
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
#[expect(
    clippy::assertions_on_constants,
    reason = "table pin: a retune must fail this test with its message"
)]
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

/// Does a defender who answers with `hold` at `press` take `kind` from an
/// attacker who presses `ATTACK` at tick 0? One pure scripted tape; the
/// press tick is always computed from the tables by the caller.
fn answer_tape(tables: &engine_combat::fight::Tables, press: u32, hold: u8, kind: &str) -> bool {
    let mut state = FightState::new(2).expect("two fighters fit");
    let horizon = tables.attack_startup_ticks + tables.attack_recovery_ticks + 8;
    for step in 0..horizon {
        let attacker = if step == 0 { ATTACK } else { 0 };
        // Block is held from its press on; a dodge is a one-tick press.
        let victim = if hold == DODGE {
            if step == press {
                DODGE
            } else {
                0
            }
        } else if step >= press {
            hold
        } else {
            0
        };
        if has(&step_pair(&mut state, tables, attacker, victim), kind) {
            return true;
        }
    }
    false
}

/// The parry span one row buys, measured on scripted tapes instead of
/// computed: `(first_parry_press, parry_presses)`, where a press is the tick
/// the victim presses and holds block while the attacker presses `ATTACK` at
/// tick 0, and a press counts when the impact comes out `Parried`. Pure; one
/// scanned tape per tick of the attacker's tell.
fn parry_span(tables: &engine_combat::Tables) -> (u32, u32) {
    let startup = tables.attack_startup_ticks;
    let presses: Vec<u32> = (0..=startup)
        .filter(|press| answer_tape(tables, *press, BLOCK, "parried"))
        .collect();
    let first = *presses
        .first()
        .expect("a block held into the impact parries");
    let count = u32::try_from(presses.len()).expect("a tell of at most 60 ticks fits u32");
    (first, count)
}

/// The answer a row names must be an outcome the engine can produce, not a
/// word: for every `counter` code in `weapons.rows` (1 parry, 2 dodge,
/// 3 block) a scripted tape under `TABLES_V2` where the defender answers
/// with that input yields the matching [`FightEvent`]. The press ticks are
/// measured off the tables one tick at a time, never typed in, so a change
/// to the startup or to either window moves this test with the row it
/// checks.
#[test]
fn every_counter_code_names_a_live_outcome() {
    let tables = &TABLES_V2;
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    // Parry: the far edge of the window, measured. The earliest press that
    // still parries; one tick before it only blocks, and the answer named by
    // a row is taken from that measured edge.
    let (earliest_parry, parry_presses) = parry_span(tables);
    assert!(
        earliest_parry > 0,
        "a row with a parry window of {} must still leave tell ticks where a parry is late",
        tables.parry_window_ticks
    );
    let past_parry = earliest_parry - 1;
    assert!(
        !answer_tape(tables, past_parry, BLOCK, "parried"),
        "a press {past_parry} ticks early must be past the parry window {}",
        tables.parry_window_ticks
    );
    assert!(
        answer_tape(tables, past_parry, BLOCK, "blocked"),
        "one tick past the window still blocks, not parries"
    );
    // The window is a span of press ticks, not a single late press: every
    // press from the measured edge to the impact tick parries.
    assert_eq!(
        parry_presses,
        tables.parry_window_ticks + 1,
        "the window {} must buy {} parrying press ticks, measured {parry_presses}",
        tables.parry_window_ticks,
        tables.parry_window_ticks + 1
    );
    // Dodge: the i-frames must still be live at impact.
    let dodge_press = tables
        .attack_startup_ticks
        .saturating_sub(tables.dodge_iframes_ticks.saturating_sub(1));
    let mut kinds = 0u8;
    for row in &rows {
        // 1 parry, 2 dodge, 3 block: the code form of the `avoid` word.
        let answered = match row.counter {
            1 => answer_tape(tables, earliest_parry, BLOCK, "parried"),
            2 => answer_tape(tables, dodge_press, DODGE, "dodged"),
            _ => answer_tape(tables, 0, BLOCK, "blocked"),
        };
        assert!(
            answered,
            "line {}: weapon `{}` names counter {} ({}), so that answer must land",
            row.line, row.id, row.counter, row.avoid
        );
        kinds |= match row.counter {
            1 => 1,
            2 => 2,
            _ => 4,
        };
    }
    assert_eq!(
        kinds, 7,
        "the rows must answer with all three counters, got mask {kinds}"
    );
}

/// ADR 0010, the parry 8 to 13 sweep: a parry must never cover a whole
/// move's tell, so the window stays below the fastest weapon's startup in
/// `weapons.rows` (`docs/design/FORMAT.md`: startup counts up to and
/// including the first active tick). Candidates 12 and 13 cover the
/// pugio's whole 12-tick tell; the shipped v2 window sits below it. The
/// 8-tick house cap is held by `v2_parry_window_shorter_than_startup`.
#[test]
fn parry_window_stays_below_the_fastest_tell() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    let fastest = rows.first().expect("rows sort fastest first").speed_ticks;
    assert!(
        TABLES_V2.parry_window_ticks < fastest,
        "v2 parry {} must sit below the fastest tell {} ({} ticks): a wider window can be held before the swing starts",
        TABLES_V2.parry_window_ticks,
        rows.first().expect("rows sort fastest first").id,
        fastest
    );
}

/// ADR 0010, the parry sweep 8 to 13, measured instead of argued. Startup is
/// held at the v2 row and each candidate is a copy of `TABLES_V2` with only
/// `parry_window_ticks` moved, so one number moves at a time (Adams ch8).
/// Two gates decide a candidate, and both are read off tapes:
///
/// - Tell gate: the parry may not answer the whole tell of the fastest weapon
///   in `weapons.rows`, or the attacker's feint has no meaning (Bycer ch8:
///   a tell and a commit).
/// - Half gate: the row may answer at most half of its own tell
///   (`parry_window_ticks * 2 <= attack_startup_ticks`), or pressing late in
///   a swing is a promise the defender can always keep (Sellers ch9-10: a
///   gate needs a cost).
///
/// The sweep's numbers are pasted in `docs/adr/0010-parry-window.md`: with
/// startup 15 and the pugio's 12-tick tell, candidates 12 and 13 fall to the
/// tell gate, 8 to 11 pass it at 600-800 permille of the tell covered and all
/// fail the half gate, and the shipped 7 covers 533 permille (8 of 15 ticks)
/// and sits on it. This test holds that verdict.
#[test]
fn parry_sweep_8_to_13_leaves_a_verdict_the_shipped_row_sits_on() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    let fastest_row = rows.first().expect("rows sort fastest first");
    let fastest = fastest_row.speed_ticks;
    let startup = TABLES_V2.attack_startup_ticks;
    assert_eq!(
        startup, 15,
        "the ADR 0010 sweep holds startup at the v2 row (15 ticks)"
    );
    let mut fell_to_tell = 0u32;
    let mut fell_to_half = 0u32;
    for candidate in 8..=13u32 {
        let mut tables = TABLES_V2;
        tables.parry_window_ticks = candidate;
        let (earliest, presses) = parry_span(&tables);
        assert_eq!(
            presses,
            candidate + 1,
            "window {candidate} must buy {} parrying press ticks, measured {presses}",
            candidate + 1
        );
        assert!(
            earliest < startup,
            "window {candidate} must leave {} tell ticks where a parry is late",
            startup.saturating_sub(candidate)
        );
        if presses > fastest {
            fell_to_tell += 1;
        }
        if candidate * 2 > startup {
            fell_to_half += 1;
        }
    }
    assert_eq!(
        (fell_to_tell, fell_to_half),
        (2, 6),
        "candidates 12 and 13 swallow the {}'s {fastest}-tick tell and every candidate 8 to 13 answers more than half the {startup}-tick tell",
        fastest_row.id
    );
    // The widest window both gates allow, and the row that ships.
    let widest = (1..=60u32)
        .filter(|window| window * 2 <= startup && *window < fastest)
        .max()
        .expect("some window fits both gates");
    assert_eq!(
        widest, TABLES_V2.parry_window_ticks,
        "the shipped parry {} must be the widest window both gates allow",
        TABLES_V2.parry_window_ticks
    );
    // The number the sweep decided on: covered press ticks of the tell, in
    // permille, for the shipped row as measured by its own tapes.
    let (earliest, covered) = parry_span(&TABLES_V2);
    assert_eq!(
        (earliest, covered, covered * 1000 / startup),
        (8, 8, 533),
        "the shipped parry {} covers {covered} of {startup} tell ticks",
        TABLES_V2.parry_window_ticks
    );
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
    /// Avoid word from the frame-data fields (`docs/design/FORMAT.md`):
    /// `parry`, `dodge`, `block` or `range`.
    avoid: &'static str,
    /// Live ticks: steel can hit only here, at least one.
    active_ticks: u32,
    /// Committed ticks after active, `0..=60`.
    recovery_ticks: u32,
    /// Punish window in ticks; sits inside recovery, at least one.
    punish_ticks: u32,
    /// Mass of the weapon in grams (FEEL1): a heavier weapon starts later,
    /// recovers longer and staggers harder.
    mass_g: u32,
}

/// The avoid word for a counter code: 1 parry, 2 dodge, 3 block
/// (`docs/design/FORMAT.md`: `counter` is the code form of `avoid`). Pure.
fn avoid_word_for_counter(counter: u32) -> &'static str {
    match counter {
        1 => "parry",
        2 => "dodge",
        _ => "block",
    }
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
    let mut avoid: Option<&'static str> = None;
    let mut active: Option<u32> = None;
    let mut recovery: Option<u32> = None;
    let mut punish: Option<u32> = None;
    let mut mass: Option<u32> = None;
    for field in &parts[1..] {
        let (k, v) = field
            .split_once('=')
            .unwrap_or_else(|| panic!("line {line}: field `{field}` needs `<key>=<int>`"));
        if k == "avoid" {
            // The avoid answer is a word, not a number (FORMAT.md frame data).
            assert!(
                matches!(v, "parry" | "dodge" | "block" | "range"),
                "line {line}: avoid `{v}` must be parry, dodge, block or range"
            );
            avoid = Some(Box::leak(v.to_string().into_boxed_str()));
            continue;
        }
        let parsed = weapons_parse_u32(v, line, k);
        match k {
            "type" => weapon_type = Some(parsed),
            "damage_permille" => damage = Some(parsed),
            "speed_ticks" => speed = Some(parsed),
            "hands" => hands = Some(parsed),
            "stagger_permille" => stagger = Some(parsed),
            "stamina_cost" => stamina = Some(parsed),
            "counter" => counter = Some(parsed),
            "active_ticks" => active = Some(parsed),
            "recovery_ticks" => recovery = Some(parsed),
            "punish_ticks" => punish = Some(parsed),
            "mass_g" => mass = Some(parsed),
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
        avoid: avoid.unwrap_or_else(|| panic!("line {line}: weapon `{id_box}` misses `avoid`")),
        active_ticks: need("active_ticks", active),
        recovery_ticks: need("recovery_ticks", recovery),
        punish_ticks: need("punish_ticks", punish),
        mass_g: need("mass_g", mass),
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
fn weapons_header_holds_five() {
    let (version, rows) = weapons_parse_table(WEAPONS_SRC);
    assert!(version != 0, "version never 0");
    assert_eq!(rows.len(), 5, "want five Rome weapons, got {}", rows.len());
    for row in &rows {
        assert!(
            (1..=20_000).contains(&row.mass_g),
            "line {}: mass_g {} must sit in 1..=20000",
            row.line,
            row.mass_g
        );
        assert!(
            (1..=4).contains(&row.weapon_type),
            "line {}: type {} must be 1 thrust, 2 cut, 3 throw or 4 blow",
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

/// The counting rule of `docs/design/FORMAT.md`: `startup` counts up to and
/// including the first active tick. On a press-at-zero tape under V2 rows the
/// first `Hit` lands on step `attack_startup_ticks`, so the tell the player
/// read lasted exactly `startup` ticks.
#[test]
fn first_active_tick_is_startup() {
    let tables = &TABLES_V2;
    let mut state = FightState::new(2).expect("two fighters fit");
    let horizon = tables.attack_startup_ticks + tables.attack_recovery_ticks + 6;
    let mut landed_on = None;
    for step in 0..horizon {
        let attacker = if step == 0 { ATTACK } else { 0 };
        let events = step_pair(&mut state, tables, attacker, 0);
        if has(&events, "hit") {
            landed_on = Some(step);
            break;
        }
    }
    assert_eq!(
        landed_on,
        Some(tables.attack_startup_ticks),
        "under v2 the first hit must land on the startup tick (FORMAT.md counting rule)"
    );
}

/// Frame data per move (`docs/design/FORMAT.md`): every row carries
/// `startup active recovery` in ticks plus the two player questions, and the
/// punish window sits inside recovery.
#[test]
fn every_move_carries_frame_data() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    for row in &rows {
        assert!(
            row.active_ticks >= 1,
            "line {}: weapon `{}` needs at least one active tick",
            row.line,
            row.id
        );
        assert!(
            row.punish_ticks >= 1,
            "line {}: weapon `{}` needs a punish window of at least one tick",
            row.line,
            row.id
        );
        assert!(
            row.punish_ticks <= row.recovery_ticks,
            "line {}: weapon `{}` punish {} must sit inside recovery {}",
            row.line,
            row.id,
            row.punish_ticks,
            row.recovery_ticks
        );
        assert!(
            row.speed_ticks + row.active_ticks + row.recovery_ticks <= 60,
            "line {}: weapon `{}` startup+active+recovery {} must fit 60 ticks",
            row.line,
            row.id,
            row.speed_ticks + row.active_ticks + row.recovery_ticks
        );
    }
}

/// The avoid answer and the counter code never disagree: `avoid` is the
/// word form of `counter` (`docs/design/FORMAT.md`).
#[test]
fn avoid_matches_counter() {
    let (_, rows) = weapons_parse_table(WEAPONS_SRC);
    for row in &rows {
        assert_eq!(
            row.avoid,
            avoid_word_for_counter(row.counter),
            "line {}: weapon `{}` avoid `{}` must be the word for counter {}",
            row.line,
            row.id,
            row.avoid,
            row.counter
        );
    }
}
