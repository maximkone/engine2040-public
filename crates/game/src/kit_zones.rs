//! Hand-placed zone table (`content/design/arena-zones.rows`): the one file
//! the design gate (`tools/level-check.mjs`) and this reader both read.

use super::{parse_err, KitError, KIT_ACTOR_COUNT};

/// Hand-placed zone table (`content/design/arena-zones.rows`): the one file
/// the design gate (`tools/level-check.mjs`) and this reader both read.
pub const KIT_ZONE_ROWS: &str = include_str!("../../../content/design/arena-zones.rows");
/// Row id of the entrance tunnel: the gate anchor owner.
pub const KIT_ZONE_GATE_ID: &str = "gate_tunnel";
/// Rows the shipped table holds: one per placement actor plus `centre`.
pub(super) const KIT_ZONE_COUNT: usize = KIT_ACTOR_COUNT + 1;

/// One row of the hand-placed zone table.
///
/// Units: whole centimeters; `class` is 0 none, 1 magnet, 2 curtain, 3
/// generator, 4 bumper, and `advantage`/`dominance`/`risk` are the row's
/// `0|1` flags. Fields: `id` actor or zone name, `cx_cm`/`cz_cm` the centre,
/// `radius_cm` the mouth or hold radius (above zero), `height_cm` the height.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KitZoneRow {
    pub id: &'static str,
    pub class: u8,
    pub cx_cm: i16,
    pub cz_cm: i16,
    pub radius_cm: i16,
    pub height_cm: i16,
    pub advantage: bool,
    pub dominance: bool,
    pub risk: bool,
}

/// Row error carrying the 1-based line, like `tools/level-check.mjs`.
fn zone_err(line: usize, detail: &str) -> KitError {
    parse_err(&format!("zones line {line}: {detail}"))
}

/// Parse the zone table text (`table arena-zones version 1`, then one
/// `<id> class=<0..4> cx_cm=<int> cz_cm=<int> radius_cm=<int> height_cm=<int>
/// advantage=<0|1> dominance=<0|1> risk=<0|1>` row; `#` comments and blank
/// lines skipped). Every rule mirrors `tools/level-check.mjs` `parseZones`.
pub(super) fn kit_zone_rows_from_text(text: &'static str) -> Result<Vec<KitZoneRow>, KitError> {
    const KEYS: [&str; 8] = [
        "class",
        "cx_cm",
        "cz_cm",
        "radius_cm",
        "height_cm",
        "advantage",
        "dominance",
        "risk",
    ];
    let mut header = false;
    let mut rows: Vec<KitZoneRow> = Vec::with_capacity(KIT_ZONE_COUNT);
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let body = raw.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        if !header {
            if body != "table arena-zones version 1" {
                return Err(zone_err(line, "first row must be the header"));
            }
            header = true;
            continue;
        }
        let mut cells = body.split_whitespace();
        let Some(id) = cells.next().filter(|id| !id.contains('=')) else {
            return Err(zone_err(line, "misses actor id"));
        };
        let mut values = [0i16; KEYS.len()];
        let mut mask = 0u8;
        for cell in cells {
            let Some((key, value)) = cell.split_once('=') else {
                return Err(zone_err(line, "bad cell"));
            };
            let Some(slot) = KEYS.iter().position(|known| *known == key) else {
                return Err(zone_err(line, "unknown key"));
            };
            let digits = value.strip_prefix('-').unwrap_or(value);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(zone_err(line, "whole number only"));
            }
            mask |= 1 << slot;
            values[slot] = value
                .parse::<i16>()
                .map_err(|_| zone_err(line, "out of range"))?;
        }
        if mask != u8::MAX {
            return Err(zone_err(line, "missing or duplicate key"));
        }
        if !(0..=4).contains(&values[0]) {
            return Err(zone_err(line, "class must be 0..4"));
        }
        if values[3] <= 0 {
            return Err(zone_err(line, "radius_cm must be above 0"));
        }
        for (slot, key) in [(5usize, "advantage"), (6, "dominance"), (7, "risk")] {
            if values[slot] != 0 && values[slot] != 1 {
                return Err(zone_err(line, &format!("{key} must be 0|1")));
            }
        }
        rows.push(KitZoneRow {
            id,
            class: u8::try_from(values[0]).unwrap_or(0),
            cx_cm: values[1],
            cz_cm: values[2],
            radius_cm: values[3],
            height_cm: values[4],
            advantage: values[5] == 1,
            dominance: values[6] == 1,
            risk: values[7] == 1,
        });
    }
    if !header {
        return Err(parse_err("zones: misses the table header"));
    }
    if rows.len() != KIT_ZONE_COUNT {
        return Err(parse_err(&format!(
            "zones: {KIT_ZONE_COUNT} rows, saw {}",
            rows.len()
        )));
    }
    Ok(rows)
}

/// The hand-placed zone rows, in file order.
///
/// Consumer: the twin's zone report (`crates/game/src/bin/arena.rs`) and the
/// ring's gate anchor (`crates/game/src/fight_ring.rs`), both reading the file
/// `tools/level-check.mjs` gates. Units: whole centimeters, flags `0|1`.
/// Determinism: same bytes give the same rows in file order (no map). Errors:
/// [`KitError::Parse`] with the 1-based line of a bad row, or the row count
/// from a missing actor row. Budget: 12 rows once at init, zero per tick.
pub fn kit_zone_rows() -> Result<Vec<KitZoneRow>, KitError> {
    kit_zone_rows_from_text(KIT_ZONE_ROWS)
}
