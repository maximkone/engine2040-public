//! Combat rows text loader: designers edit numbers, no Rust edit.
//!
//! Consumer: practice reload (next slice) reads `content/design/combat.rows`
//! through [`parse_tables`](crate::tables_text::parse_tables) at a tick boundary; this module never steps.
//! Data owner: `engine-combat` (`Tables`, `SwapInput`, `validate_tables`).
//! Units as in `Tables`: damage permille, ticks at 60 Hz, stamina points,
//! chip counts. Determinism: same text gives same swap or same refusal.
//! Errors: [`CombatError`] only; parse failures carry the line in `value`,
//! range failures carry the culprit value. Cold path only, never tick.
//! Digits owner: [`engine_core::checked::decimal_u32`] (no local copy).
//! Caller: `crates/combat/src/lib.rs` (module wiring; game wiring in EG29).

use engine_core::checked::decimal_u32;

use crate::fight::{CombatError, Tables, TABLES_V1, TABLES_V2};
use crate::tables::MoveTableSet;
use crate::tables::{validate_tables, SwapInput};

/// Field names in `Tables` order; the only accepted keys.
const KNOWN: [&str; 16] = [
    "attack_damage",
    "attack_startup_ticks",
    "attack_recovery_ticks",
    "attack_stamina_cost",
    "dodge_stamina_cost",
    "block_hold_cost_per_tick",
    "stamina_regen_per_tick",
    "dodge_duration_ticks",
    "dodge_iframes_ticks",
    "block_chip_num",
    "block_chip_den",
    "parry_window_ticks",
    "parry_stagger_ticks",
    "hit_react_ticks",
    "hitstop_ticks",
    "knockdown_ticks",
];

/// Parse `table combat version <n>`; `n` nonzero, refused with its line.
fn parse_header(line: &str, lineno: u32) -> Result<u32, CombatError> {
    let mut parts = line.split_whitespace();
    let ok = match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(a), Some(name), Some(b), Some(_), None) => {
            a == "table" && b == "version" && name == "combat"
        }
        _ => false,
    };
    if !ok {
        return Err(CombatError::BadTableValue {
            field: "table",
            value: lineno,
        });
    }
    // Length was checked above; re-split to fetch the version token.
    let mut again = line.split_whitespace();
    let _ = again.next();
    let _ = again.next();
    let _ = again.next();
    let token = again.next().unwrap_or("");
    match decimal_u32(token) {
        Some(0) => Err(CombatError::BadTableVersion(0)),
        None => Err(CombatError::BadTableValue {
            field: "table",
            value: lineno,
        }),
        Some(version) => Ok(version),
    }
}

/// Parse one `<id> <key>=<int> ...` row into its version plus [`Tables`].
/// Unknown, duplicate or missing keys refuse with the line in `value`.
fn parse_row(line: &str, lineno: u32) -> Result<(u32, Tables), CombatError> {
    let mut words = line.split_whitespace();
    let id = words.next().ok_or(CombatError::BadTableValue {
        field: "id",
        value: lineno,
    })?;
    let (prefix, digits) = id.split_at_checked(1).ok_or(CombatError::BadTableValue {
        field: "id",
        value: lineno,
    })?;
    if prefix != "v" {
        return Err(CombatError::BadTableValue {
            field: "id",
            value: lineno,
        });
    }
    let row_version = match decimal_u32(digits) {
        Some(0) | None => {
            return Err(CombatError::BadTableValue {
                field: "id",
                value: lineno,
            });
        }
        Some(version) => version,
    };
    let mut values: [Option<u32>; 16] = [None; 16];
    for pair in words {
        let (key, val) = pair.split_once('=').ok_or(CombatError::BadTableValue {
            field: "unknown_key",
            value: lineno,
        })?;
        let mut slot: Option<usize> = None;
        for (index, known) in KNOWN.iter().enumerate() {
            if *known == key {
                slot = Some(index);
                break;
            }
        }
        let index = slot.ok_or(CombatError::BadTableValue {
            field: "unknown_key",
            value: lineno,
        })?;
        if values[index].is_some() {
            return Err(CombatError::BadTableValue {
                field: KNOWN[index],
                value: lineno,
            });
        }
        let number = decimal_u32(val).ok_or(CombatError::BadTableValue {
            field: KNOWN[index],
            value: lineno,
        })?;
        values[index] = Some(number);
    }
    for (index, known) in KNOWN.iter().enumerate() {
        if values[index].is_none() {
            return Err(CombatError::BadTableValue {
                field: known,
                value: lineno,
            });
        }
    }
    let get = |index: usize| values[index].unwrap_or(0);
    let wide = |index: usize, field: &'static str| -> Result<u16, CombatError> {
        u16::try_from(get(index)).map_err(|_| CombatError::BadTableValue {
            field,
            value: lineno,
        })
    };
    Ok((
        row_version,
        Tables {
            attack_damage: wide(0, KNOWN[0])?,
            attack_startup_ticks: get(1),
            attack_recovery_ticks: get(2),
            attack_stamina_cost: wide(3, KNOWN[3])?,
            dodge_stamina_cost: wide(4, KNOWN[4])?,
            block_hold_cost_per_tick: wide(5, KNOWN[5])?,
            stamina_regen_per_tick: wide(6, KNOWN[6])?,
            dodge_duration_ticks: get(7),
            dodge_iframes_ticks: get(8),
            block_chip_num: get(9),
            block_chip_den: get(10),
            parry_window_ticks: get(11),
            parry_stagger_ticks: get(12),
            hit_react_ticks: get(13),
            hitstop_ticks: get(14),
            knockdown_ticks: get(15),
        },
    ))
}

/// Parse one rows file into the header-version [`SwapInput`].
///
/// The file holds every shipped row as reference (v1 plus v2); the row whose
/// `v<n>` id matches the header version becomes the swap. Range checks are
/// [`validate_tables`]; v1 exact rows stay replay-only and refuse as a new
/// swap, so designers start from v2.
pub fn parse_tables(text: &str) -> Result<SwapInput, CombatError> {
    let mut lineno: u32 = 0;
    let mut header_version: Option<u32> = None;
    let mut header_line: u32 = 0;
    let mut rows: Vec<(u32, Tables, u32)> = Vec::new();
    for raw in text.lines() {
        lineno = lineno.saturating_add(1);
        let body = raw.split('#').next().unwrap_or("").trim();
        if body.is_empty() {
            continue;
        }
        if header_version.is_none() {
            let version = parse_header(body, lineno)?;
            header_version = Some(version);
            header_line = lineno;
        } else {
            let (version, tables) = parse_row(body, lineno)?;
            for (seen, _, _) in &rows {
                if *seen == version {
                    return Err(CombatError::BadTableValue {
                        field: "id",
                        value: lineno,
                    });
                }
            }
            rows.push((version, tables, lineno));
        }
    }
    let wanted = header_version.ok_or(CombatError::BadTableValue {
        field: "table",
        value: 0,
    })?;
    if rows.is_empty() {
        return Err(CombatError::BadTableValue {
            field: "id",
            value: header_line,
        });
    }
    for (version, tables, row_line) in &rows {
        if *version == wanted {
            validate_tables(&MoveTableSet::new(*version, *tables))?;
            // Shipped versions stay frozen: a v1/v2 swap must equal the
            // compiled row, otherwise designers must ship a new version.
            // Validated first so range culprits keep their field; frozen
            // edits refuse with the row line.
            let frozen_ok = match *version {
                1 => *tables == TABLES_V1,
                2 => *tables == TABLES_V2,
                _ => true,
            };
            if !frozen_ok {
                return Err(CombatError::BadTableValue {
                    field: "id",
                    value: *row_line,
                });
            }
            return SwapInput::new(*version, *tables);
        }
    }
    Err(CombatError::BadTableValue {
        field: "id",
        value: header_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{TABLES_V1, TABLES_V2};

    const V1_LINE: &str = "v1 attack_damage=120 attack_startup_ticks=4 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=4 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";
    const V2_LINE: &str = "v2 attack_damage=120 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";

    fn v2_file() -> String {
        ["table combat version 2", V2_LINE].join("\n")
    }

    #[test]
    fn v1_row_text_equals_compiled_v1() {
        let (version, tables) = parse_row(V1_LINE, 2).expect("v1 reference parses");
        assert_eq!(version, 1);
        assert_eq!(tables, TABLES_V1);
    }

    #[test]
    fn v2_file_text_equals_compiled_v2() {
        let swap = parse_tables(&v2_file()).expect("v2 file parses");
        assert_eq!(swap.version(), 2);
        assert_eq!(swap.tables(), TABLES_V2);
    }

    #[test]
    fn shipped_file_holds_both_copies() {
        let text = include_str!("../../../content/design/combat.rows");
        let swap = parse_tables(text).expect("shipped file parses to v2");
        assert_eq!(swap.tables(), TABLES_V2);
        let v1_line = text
            .lines()
            .find(|l| l.starts_with("v1 "))
            .expect("v1 line ships");
        let (_, v1) = parse_row(v1_line, 3).expect("v1 line parses");
        assert_eq!(v1, TABLES_V1);
    }

    #[test]
    fn bad_value_is_refused() {
        let bad = v2_file().replace("attack_damage=120", "attack_damage=0");
        assert_eq!(
            parse_tables(&bad),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                value: 0
            })
        );
    }

    #[test]
    fn float_unknown_and_missing_refuse_with_line() {
        let float = v2_file().replace("attack_damage=120", "attack_damage=12.0");
        assert_eq!(
            parse_tables(&float),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                value: 2
            })
        );
        let unknown = v2_file().replace("attack_damage=120", "nope_ticks=1");
        assert_eq!(
            parse_tables(&unknown),
            Err(CombatError::BadTableValue {
                field: "unknown_key",
                value: 2
            })
        );
        let missing = [
            "table combat version 2",
            &V2_LINE.replace(" knockdown_ticks=30", ""),
        ]
        .join("\n");
        assert_eq!(
            parse_tables(&missing),
            Err(CombatError::BadTableValue {
                field: "knockdown_ticks",
                value: 2
            })
        );
    }

    #[test]
    fn frozen_shipped_versions_refuse_edits() {
        // Shipped v1/v2 stay frozen: editing them in place must refuse with
        // the row line, while a new version with valid rows still swaps.
        let v1_strict = V1_LINE.replace("parry_window_ticks=4", "parry_window_ticks=3");
        let edited_v1 = ["table combat version 1", &v1_strict].join("\n");
        assert_eq!(
            parse_tables(&edited_v1),
            Err(CombatError::BadTableValue {
                field: "id",
                value: 2
            })
        );
        let edited_v2 = v2_file().replace("attack_damage=120", "attack_damage=121");
        assert_eq!(
            parse_tables(&edited_v2),
            Err(CombatError::BadTableValue {
                field: "id",
                value: 2
            })
        );
        // A new version with valid rows still swaps.
        let v3_line = V2_LINE.replace("v2 ", "v3 ");
        let v3_file = ["table combat version 3", V1_LINE, &v3_line].join("\n");
        let swap = parse_tables(&v3_file).expect("new version swaps");
        assert_eq!(swap.version(), 3);
    }

    #[test]
    fn row_digits_use_the_shared_checked_owner() {
        // Digits only in, whole number out, overflow refused: the one owner
        // the arena zones rows reader also calls, so the two rows readers
        // cannot drift into two copies of the same body.
        assert_eq!(engine_core::checked::decimal_u32("120"), Some(120));
        assert_eq!(engine_core::checked::decimal_u32("4294967296"), None);
        let over = v2_file().replace("attack_damage=120", "attack_damage=4294967296");
        assert_eq!(
            parse_tables(&over),
            Err(CombatError::BadTableValue {
                field: "attack_damage",
                value: 2
            })
        );
    }
}
