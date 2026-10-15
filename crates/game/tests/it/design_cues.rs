//! Design cue table: one sound cue per fight event, inside a latency budget.
//!
//! Consumer: the audio seat (EG21) reads `content/design/cues.rows` when it
//! wires `FightEvent` playback through the `CueBank`; the HUD seat reads the
//! latency budget beside `docs/design/HUD.md`. Data owner: this test owns the
//! parse; `engine-combat` owns [`FightEvent`], `engine-audio` owns the cue
//! bank. Units: bus is 0 Sfx, 1 Music, 2 Dialogue; priority is a `u8` steal
//! rank (higher survives); `latency_ms` is milliseconds from event to audible
//! cue. Determinism: parse is pure over the included text. Errors: a bad
//! header, key, unknown cue name or missing event fails with the line number.

use engine_audio::cue::{names, ARENA_CUES};
use engine_audio::dsp::Bus;
use engine_combat::fight::FightEvent;

/// Included cue table text (compile-time, no IO at test time).
const CUES_SRC: &str = include_str!("../../../../content/design/cues.rows");

/// Sound budget: a cue later than this feels detached from its hit (combat
/// feels worse as cue delay rises; 100 ms is loose).
const SOUND_BUDGET_MS: u16 = 40;

/// One parsed cue row with its source line. Pure data.
struct CueRow {
    /// Row id, e.g. `hit`.
    id: &'static str,
    /// Source line number (1-based) for refusal messages.
    line: usize,
    /// Bank cue name, one of `ARENA_CUES`.
    cue: &'static str,
    /// Mix bus code: 0 Sfx, 1 Music, 2 Dialogue.
    bus: u8,
    /// Steal rank: higher survives voice stealing.
    priority: u8,
    /// Milliseconds from event to audible cue, at most [`SOUND_BUDGET_MS`].
    latency_ms: u16,
}

/// Map one event to its table id. Pure; the match is exhaustive so a new
/// `FightEvent` variant fails to compile here until it gains a row.
fn event_id(event: &FightEvent) -> &'static str {
    match event {
        FightEvent::Hit { .. } => "hit",
        FightEvent::Blocked { .. } => "blocked",
        FightEvent::Parried { .. } => "parried",
        FightEvent::Dodged { .. } => "dodged",
        FightEvent::StaminaOut { .. } => "stamina_out",
        FightEvent::Knockdown { .. } => "knockdown",
    }
}

/// Every event variant, one value each, so the coverage test cannot miss one.
/// Pure.
fn all_events() -> [FightEvent; 6] {
    [
        FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        },
        FightEvent::Blocked {
            attacker: 0,
            victim: 1,
            chip: 24,
        },
        FightEvent::Parried {
            attacker: 0,
            victim: 1,
        },
        FightEvent::Dodged {
            attacker: 0,
            victim: 1,
        },
        FightEvent::StaminaOut { fighter: 0 },
        FightEvent::Knockdown {
            attacker: 0,
            victim: 1,
        },
    ]
}

/// Mix bus for a table code; `None` when the code names no bus. Pure.
fn bus_of(code: u8) -> Option<Bus> {
    match code {
        0 => Some(Bus::Sfx),
        1 => Some(Bus::Music),
        2 => Some(Bus::Dialogue),
        _ => None,
    }
}

/// True when the text is all ASCII digits with at least one digit. Pure.
fn is_whole_number(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Parse one `u8` whole number or panic with the line number. Pure.
fn parse_u8(value: &str, line: usize, key: &str) -> u8 {
    assert!(
        is_whole_number(value),
        "line {line}: {key} must be a whole number, got `{value}`"
    );
    value
        .parse::<u8>()
        .unwrap_or_else(|_| panic!("line {line}: {key} out of range 0..=255"))
}

/// Parse one `u16` whole number or panic with the line number. Pure.
fn parse_u16(value: &str, line: usize, key: &str) -> u16 {
    assert!(
        is_whole_number(value),
        "line {line}: {key} must be a whole number, got `{value}`"
    );
    value
        .parse::<u16>()
        .unwrap_or_else(|_| panic!("line {line}: {key} out of range 0..=65535"))
}

/// Parse the table header, returning the version. Fails on a bad header.
fn parse_version(first: &str) -> u64 {
    let parts: Vec<&str> = first.split_whitespace().collect();
    assert!(
        parts.len() == 4 && parts[0] == "table" && parts[1] == "cues" && parts[2] == "version",
        "line 1 must be `table cues version <n>`, got `{first}`"
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

/// True when `id` names a `FightEvent` variant row. Pure.
fn is_event_id(id: &str) -> bool {
    all_events().iter().any(|event| event_id(event) == id)
}

/// Parse one data line into a row. Fails with the line number.
fn parse_line(line_text: &str, line: usize) -> CueRow {
    let parts: Vec<&str> = line_text.split_whitespace().collect();
    assert!(parts.len() >= 2, "line {line}: need `<id> <key>=<int> ...`");
    let id_box: &'static str = Box::leak(parts[0].to_string().into_boxed_str());
    assert!(
        is_event_id(id_box),
        "line {line}: unknown event `{}`",
        parts[0]
    );
    let mut cue: Option<&'static str> = None;
    let mut bus: Option<u8> = None;
    let mut priority: Option<u8> = None;
    let mut latency_ms: Option<u16> = None;
    for field in &parts[1..] {
        let (k, v) = field
            .split_once('=')
            .unwrap_or_else(|| panic!("line {line}: field `{field}` needs `<key>=<int>`"));
        match k {
            // `cue` is a bank-name reference (the ids in
            // `engine-audio` cue.rs), not a number: every other key is whole.
            "cue" => {
                assert!(
                    !v.is_empty() && !v.contains(char::is_whitespace),
                    "line {line}: cue must name a bank cue, got `{v}`"
                );
                cue = Some(Box::leak(v.to_string().into_boxed_str()) as &'static str);
            }
            "bus" => bus = Some(parse_u8(v, line, k)),
            "priority" => priority = Some(parse_u8(v, line, k)),
            "latency_ms" => latency_ms = Some(parse_u16(v, line, k)),
            _ => panic!("line {line}: unknown key `{k}`"),
        }
    }
    let need_u8 = |name: &str, slot: Option<u8>| {
        slot.unwrap_or_else(|| panic!("line {line}: event `{id_box}` misses `{name}`"))
    };
    CueRow {
        id: id_box,
        line,
        cue: cue.unwrap_or_else(|| panic!("line {line}: event `{id_box}` misses `cue`")),
        bus: need_u8("bus", bus),
        priority: need_u8("priority", priority),
        latency_ms: latency_ms
            .unwrap_or_else(|| panic!("line {line}: event `{id_box}` misses `latency_ms`")),
    }
}

/// Parse the whole table: version plus one row per event. Pure.
fn parse_table(src: &str) -> (u64, Vec<CueRow>) {
    let mut logical: Vec<(usize, String)> = Vec::new();
    for (index, raw) in src.lines().enumerate() {
        let cut = raw.split('#').next().unwrap_or("");
        let trimmed = cut.trim();
        if trimmed.is_empty() {
            continue;
        }
        logical.push((index + 1, trimmed.to_string()));
    }
    assert!(!logical.is_empty(), "cues.rows is empty");
    let version = parse_version(&logical[0].1);
    let mut rows = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (line, text) in logical.iter().skip(1) {
        let row = parse_line(text, *line);
        assert!(
            !seen.contains(&row.id),
            "line {line}: duplicate event `{}`",
            row.id
        );
        seen.push(row.id);
        rows.push(row);
    }
    (version, rows)
}

/// Find a row by event id. Pure.
fn row_for<'a>(rows: &'a [CueRow], id: &str) -> &'a CueRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing event `{id}`"))
}

#[test]
fn header_version_nonzero() {
    let (version, rows) = parse_table(CUES_SRC);
    assert!(version != 0, "version never 0");
    assert_eq!(
        rows.len(),
        all_events().len(),
        "one row per event, got {}",
        rows.len()
    );
}

#[test]
fn every_event_has_a_cue() {
    let (_, rows) = parse_table(CUES_SRC);
    let mut missing = 0;
    for event in all_events().iter() {
        if rows.iter().all(|row| row.id != event_id(event)) {
            missing += 1;
        }
    }
    assert_eq!(missing, 0, "events with no cue: {missing}");
}

#[test]
fn every_row_names_a_bank_cue_on_a_live_bus() {
    let (_, rows) = parse_table(CUES_SRC);
    let bank: Vec<&str> = ARENA_CUES.iter().map(|spec| spec.name).collect();
    for row in &rows {
        assert!(
            bank.contains(&row.cue),
            "line {}: cue `{}` is no cue in the bank",
            row.line,
            row.cue
        );
        assert!(
            bus_of(row.bus).is_some(),
            "line {}: bus {} names no mix bus",
            row.line,
            row.bus
        );
    }
}

#[test]
fn latencies_stay_inside_the_sound_budget() {
    let (_, rows) = parse_table(CUES_SRC);
    let mut late = 0;
    for row in &rows {
        if row.latency_ms > SOUND_BUDGET_MS {
            late += 1;
        }
    }
    assert_eq!(
        late, 0,
        "cues past {SOUND_BUDGET_MS} ms sound budget: {late}"
    );
}

#[test]
fn cues_match_todays_mapping() {
    // The game maps each event field-for-field into its fight cue
    // (`crates/game/src/lib.rs map_cue`); impact events play their
    // same-named bank cues, a dodge is only heard as the swing whoosh, and
    // stamina-out reuses it quietly until a whiff cue lands.
    let (_, rows) = parse_table(CUES_SRC);
    let want = [
        ("hit", names::HIT),
        ("blocked", names::BLOCK),
        ("parried", names::PARRY),
        ("dodged", names::SWING),
        ("stamina_out", names::SWING),
        ("knockdown", names::KNOCKDOWN),
    ];
    for (id, cue) in want {
        assert_eq!(row_for(&rows, id).cue, cue, "event `{id}` must cue `{cue}`");
    }
    // Impact cues ride the Sfx bus the arena mixer drives.
    for id in ["hit", "blocked", "parried", "knockdown"] {
        assert_eq!(
            bus_of(row_for(&rows, id).bus),
            Some(Bus::Sfx),
            "event `{id}` must ride the Sfx bus"
        );
    }
}
