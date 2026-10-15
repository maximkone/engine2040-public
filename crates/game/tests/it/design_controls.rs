//! Design controls: one key, one pad button and feedback per verb.
//!
//! Consumer: the gamepad seat (EG27/UIX14) and the gesture seat (GP46) read
//! `content/design/controls.rows`; the HUD seat reads the feedback flags.
//! Data owner: this test owns the parse; `engine-input` owns [`Bindings`].
//! Units: key codes address [`Key`](engine_input::map::Key) storage,
//! pad codes are gamepad button indices, hand is 1 left or 2 right,
//! flags are 0/1. Determinism: parse is pure over the included text.
//! Errors: a bad header, key, or missing row fails with the line number.

use engine_input::bindings::{Bindings, BoundAction, InputConfig, PhysicalButton};

/// Included controls table text (compile-time, no IO at test time).
const CONTROLS_SRC: &str = include_str!("../../../../content/design/controls.rows");

/// One parsed controls row with its source line. Pure data.
struct ControlRow {
    /// Row id, e.g. `attack`.
    id: &'static str,
    /// Source line number (1-based) for refusal messages.
    line: usize,
    /// 1 when the verb is primary (shares no key with another primary).
    primary: u8,
    /// Keyboard code (see `map::Key`).
    key: u8,
    /// Gamepad button index.
    pad: u8,
    /// 1 left hand, 2 right hand.
    hand: u8,
    /// Sound feedback flag.
    fb_sound: u8,
    /// Animation feedback flag.
    fb_anim: u8,
    /// Camera feedback flag.
    fb_camera: u8,
    /// UI feedback flag.
    fb_ui: u8,
    /// 1 when the verb needs a hold.
    hold: u8,
    /// 1 when a toggle alternative works.
    toggle_ok: u8,
}

/// Map a row id to its bound action. Pure.
fn verb_action(id: &str) -> Option<BoundAction> {
    match id {
        "attack" => Some(BoundAction::Attack),
        "block" => Some(BoundAction::Block),
        "dodge" => Some(BoundAction::Dodge),
        "move_forward" => Some(BoundAction::MoveForward),
        "move_back" => Some(BoundAction::MoveBack),
        "move_left" => Some(BoundAction::MoveLeft),
        "move_right" => Some(BoundAction::MoveRight),
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

/// Parse the table header, returning the version. Fails on a bad header.
fn parse_version(first: &str) -> u64 {
    let parts: Vec<&str> = first.split_whitespace().collect();
    assert!(
        parts.len() == 4 && parts[0] == "table" && parts[1] == "controls" && parts[2] == "version",
        "line 1 must be `table controls version <n>`, got `{first}`"
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
fn parse_line(line_text: &str, line: usize) -> ControlRow {
    let parts: Vec<&str> = line_text.split_whitespace().collect();
    assert!(parts.len() >= 2, "line {line}: need `<id> <key>=<int> ...`");
    let id_box: &'static str = Box::leak(parts[0].to_string().into_boxed_str());
    assert!(
        verb_action(id_box).is_some(),
        "line {line}: unknown verb `{}`",
        parts[0]
    );
    let mut primary: Option<u8> = None;
    let mut key: Option<u8> = None;
    let mut pad: Option<u8> = None;
    let mut hand: Option<u8> = None;
    let mut fb_sound: Option<u8> = None;
    let mut fb_anim: Option<u8> = None;
    let mut fb_camera: Option<u8> = None;
    let mut fb_ui: Option<u8> = None;
    let mut hold: Option<u8> = None;
    let mut toggle_ok: Option<u8> = None;
    for field in &parts[1..] {
        let (k, v) = field
            .split_once('=')
            .unwrap_or_else(|| panic!("line {line}: field `{field}` needs `<key>=<int>`"));
        let parsed = parse_u8(v, line, k);
        match k {
            "primary" => primary = Some(parsed),
            "key" => key = Some(parsed),
            "pad" => pad = Some(parsed),
            "hand" => hand = Some(parsed),
            "fb_sound" => fb_sound = Some(parsed),
            "fb_anim" => fb_anim = Some(parsed),
            "fb_camera" => fb_camera = Some(parsed),
            "fb_ui" => fb_ui = Some(parsed),
            "hold" => hold = Some(parsed),
            "toggle_ok" => toggle_ok = Some(parsed),
            _ => panic!("line {line}: unknown key `{k}`"),
        }
    }
    let need = |name: &str, slot: Option<u8>| {
        slot.unwrap_or_else(|| panic!("line {line}: verb `{id_box}` misses `{name}`"))
    };
    let row = ControlRow {
        id: id_box,
        line,
        primary: need("primary", primary),
        key: need("key", key),
        pad: need("pad", pad),
        hand: need("hand", hand),
        fb_sound: need("fb_sound", fb_sound),
        fb_anim: need("fb_anim", fb_anim),
        fb_camera: need("fb_camera", fb_camera),
        fb_ui: need("fb_ui", fb_ui),
        hold: need("hold", hold),
        toggle_ok: need("toggle_ok", toggle_ok),
    };
    for (name, value) in [
        ("primary", row.primary),
        ("fb_sound", row.fb_sound),
        ("fb_anim", row.fb_anim),
        ("fb_camera", row.fb_camera),
        ("fb_ui", row.fb_ui),
        ("hold", row.hold),
        ("toggle_ok", row.toggle_ok),
    ] {
        assert!(
            value <= 1,
            "line {line}: {name} must be 0 or 1, got {value}"
        );
    }
    assert!(
        row.hand == 1 || row.hand == 2,
        "line {line}: hand must be 1 left or 2 right, got {}",
        row.hand
    );
    row
}

/// Parse the whole table: version plus one row per verb. Pure.
fn parse_table(src: &str) -> (u64, Vec<ControlRow>) {
    let mut logical: Vec<(usize, String)> = Vec::new();
    for (index, raw) in src.lines().enumerate() {
        let cut = raw.split('#').next().unwrap_or("");
        let trimmed = cut.trim();
        if trimmed.is_empty() {
            continue;
        }
        logical.push((index + 1, trimmed.to_string()));
    }
    assert!(!logical.is_empty(), "controls.rows is empty");
    let version = parse_version(&logical[0].1);
    let mut rows = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (line, text) in logical.iter().skip(1) {
        let row = parse_line(text, *line);
        assert!(
            !seen.contains(&row.id),
            "line {line}: duplicate verb `{}`",
            row.id
        );
        seen.push(row.id);
        rows.push(row);
    }
    (version, rows)
}

/// Find a row by verb id. Pure.
fn row_for<'a>(rows: &'a [ControlRow], id: &str) -> &'a ControlRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing verb `{id}`"))
}

#[test]
fn header_version_nonzero() {
    let (version, rows) = parse_table(CONTROLS_SRC);
    assert!(version != 0, "version never 0");
    assert_eq!(rows.len(), 7, "one row per verb, got {}", rows.len());
}

#[test]
fn verbs_match_default_bindings() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    let defaults = Bindings::new();
    // Every verb exists in the default table with the same key code.
    for row in &rows {
        let action = verb_action(row.id)
            .unwrap_or_else(|| panic!("line {}: unknown verb `{}`", row.line, row.id));
        match defaults.button(action) {
            PhysicalButton::Key(code) => assert_eq!(
                code, row.key,
                "line {}: verb `{}` key {} must equal default {code}",
                row.line, row.id, row.key
            ),
            other => panic!(
                "line {}: default binding for `{}` is not a key: {other:?}",
                row.line, row.id
            ),
        }
    }
    // Every default binding is a verb: buttons with no action are 0.
    let mut uncovered = 0;
    for action in BoundAction::ALL {
        if rows.iter().any(|row| verb_action(row.id) == Some(action)) {
            continue;
        }
        uncovered += 1;
    }
    assert_eq!(uncovered, 0, "buttons with no action: {uncovered}");
}

#[test]
fn primary_keys_unique() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    let mut keys: Vec<u8> = Vec::new();
    for row in rows.iter().filter(|row| row.primary == 1) {
        assert!(
            !keys.contains(&row.key),
            "line {}: primary key {} is shared",
            row.line,
            row.key
        );
        keys.push(row.key);
    }
    assert!(!keys.is_empty(), "at least one primary verb");
}

#[test]
fn every_verb_has_feedback() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    let mut silent = 0;
    for row in &rows {
        if row.fb_sound + row.fb_anim + row.fb_camera + row.fb_ui == 0 {
            silent += 1;
        }
    }
    assert_eq!(silent, 0, "actions with no feedback: {silent}");
}

#[test]
fn attack_right_defend_left() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    assert_eq!(
        row_for(&rows, "attack").hand,
        2,
        "attack rides the right hand"
    );
    assert_eq!(row_for(&rows, "block").hand, 1, "block rides the left hand");
    assert_eq!(row_for(&rows, "dodge").hand, 1, "dodge rides the left hand");
}

#[test]
fn hold_toggle_accessibility() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    for row in &rows {
        assert!(
            !(row.hold == 1 && row.toggle_ok == 1),
            "line {}: verb `{}` needs a hold where a toggle works",
            row.line,
            row.id
        );
    }
}

#[test]
fn every_verb_rebindable() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    for (index, row) in rows.iter().enumerate() {
        let action = verb_action(row.id)
            .unwrap_or_else(|| panic!("line {}: unknown verb `{}`", row.line, row.id));
        let mut table = Bindings::new();
        let original = table.button(action);
        let probe = PhysicalButton::Pad(100 + index as u8);
        table.rebind(action, probe);
        assert_eq!(
            table.button(action),
            probe,
            "line {}: verb `{}` did not rebind",
            row.line,
            row.id
        );
        table.rebind(action, original);
        assert_eq!(table.button(action), original);
    }
    // Save roundtrip keeps every row (config path never bricks input).
    let kept = InputConfig::from_bindings(&Bindings::new()).to_bindings();
    assert_eq!(kept, Bindings::new(), "config roundtrip must keep defaults");
}

#[test]
fn pad_buttons_distinct() {
    let (_, rows) = parse_table(CONTROLS_SRC);
    let mut pads: Vec<u8> = Vec::new();
    for row in &rows {
        assert!(
            !pads.contains(&row.pad),
            "line {}: pad button {} is shared",
            row.line,
            row.pad
        );
        pads.push(row.pad);
    }
}
