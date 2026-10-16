//! Loop rows text loader: designers edit numbers, no Rust edit.
//!
//! Consumer: the mode framework reads a [`LoopSet`] at a progression
//! boundary; this module only parses and validates, it never folds a
//! ladder. Data owner: [`ladder`](crate::ladder) owns rung, glory and
//! repair counters. Units: glory awards and rank floors are counts (no
//! suffix). Determinism scope: same text gives the same set or the same
//! refusal; integer math only, rows keep file order. Errors:
//! [`LoopRowsError`] carries the 1-based line number. Budget: cold path
//! only, `O(rows)` time over one small row `Vec`; never called per tick.
//! Caller: `crates/game/src/ladder.rs` (`apply_with_tables`,
//! `rank_with_tables`, `apply_rows_text` at a progression boundary).

#[cfg(test)]
use crate::ladder::{GLORY_PER_LOOP, GLORY_PER_WIN, GLORY_RANKS, GLORY_RUNG_BONUS};

/// Field names in row order; the only accepted keys.
const KNOWN: [&str; 8] = [
    "glory_per_win",
    "glory_rung_bonus",
    "glory_per_loop",
    "glory_rank0",
    "glory_rank1",
    "glory_rank2",
    "glory_rank3",
    "glory_rank4",
];

/// Sane upper bounds for replacement rows, in counts.
const MAX_GLORY_WIN: u32 = 10_000;
const MAX_RUNG_BONUS: u32 = 1_000;
const MAX_GLORY_LOOP: u32 = 10_000;
const MAX_RANK_FLOOR: u32 = 100_000;

/// Tunable loop row: the glory numbers designers edit, in one place.
///
/// Units are counts. Rank floors rise strictly from zero, so
/// [`Ladder::rank`](crate::ladder::Ladder::rank) steps up exactly once
/// per floor. Plain integers only, so sets compare deterministically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopTables {
    /// Glory per win, counts.
    pub glory_per_win: u32,
    /// Extra glory per win per rung held, counts.
    pub glory_rung_bonus: u32,
    /// Glory granted on completing a top-rung loop, counts.
    pub glory_per_loop: u32,
    /// Glory floors per rank index, counts; first is zero, strictly rising.
    pub glory_ranks: [u32; 5],
}

/// Versioned loop set: header version plus validated rows.
///
/// The compiled ladder constants stay the truth for replays; the parsed
/// v1 row must equal them, so the file is a second door to the same
/// numbers, not a fork.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LoopSet {
    /// Table version; nonzero, 1 for the shipped tuning.
    pub version: u32,
    /// Validated tuning rows.
    pub tables: LoopTables,
}

/// Loop rows parse and range failures, with the 1-based line number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopRowsError {
    /// First content line is not `table loop version <n>`.
    Header(u32),
    /// Header version is zero or unreadable.
    Version(u32),
    /// Row id is not `v<n>`, is zero, or names a repeated version.
    RowId(u32),
    /// Key is unknown (no `=` splits to one either).
    UnknownKey(u32),
    /// A known key appears twice; culprit field carried.
    Duplicate {
        /// Repeated field name.
        field: &'static str,
        /// 1-based line number.
        line: u32,
    },
    /// An expected key never appears; culprit field carried.
    Missing {
        /// Missing field name.
        field: &'static str,
        /// 1-based line number.
        line: u32,
    },
    /// Value is not ASCII digits; culprit field carried.
    BadInt {
        /// Culprit field name.
        field: &'static str,
        /// 1-based line number.
        line: u32,
    },
    /// Keys parse but the ranges refuse them; culprit field carried.
    Invalid {
        /// Culprit field name.
        field: &'static str,
        /// 1-based line number.
        line: u32,
    },
}

impl core::fmt::Display for LoopRowsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Header(line) => write!(
                f,
                "loop header on line {line} is not `table loop version <n>`"
            ),
            Self::Version(line) => write!(f, "loop version on line {line} is zero or unreadable"),
            Self::RowId(line) => write!(f, "loop row id on line {line} is not a unique `v<n>`"),
            Self::UnknownKey(line) => write!(f, "loop unknown key on line {line}"),
            Self::Duplicate { field, line } => {
                write!(f, "loop duplicate {field} on line {line}")
            }
            Self::Missing { field, line } => {
                write!(f, "loop missing {field} on line {line}")
            }
            Self::BadInt { field, line } => {
                write!(f, "loop {field} on line {line} is not ASCII digits")
            }
            Self::Invalid { field, line } => {
                write!(f, "loop {field} on line {line} is outside its range")
            }
        }
    }
}

impl std::error::Error for LoopRowsError {}

/// ASCII digits only, no sign, no dot, checked overflow; `None` refuses.
///
/// Through [`str::parse`]: the `+` guard keeps the unsigned leading-plus
/// form out, so exactly the plain-digit strings pass, with overflow
/// refused by the parser.
fn decimal(text: &str) -> Option<u32> {
    if text.is_empty() || text.starts_with('+') {
        return None;
    }
    text.parse::<u32>().ok()
}

/// Parse `table loop version <n>`; `n` nonzero, refused with its line.
fn parse_header(line: &str, lineno: u32) -> Result<u32, LoopRowsError> {
    let mut parts = line.split_whitespace();
    let ok = match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some(a), Some(name), Some(b), Some(_), None) => {
            a == "table" && b == "version" && name == "loop"
        }
        _ => false,
    };
    if !ok {
        return Err(LoopRowsError::Header(lineno));
    }
    let mut again = line.split_whitespace();
    let _ = again.next();
    let _ = again.next();
    let _ = again.next();
    let token = again.next().unwrap_or("");
    match decimal(token) {
        Some(0) | None => Err(LoopRowsError::Version(lineno)),
        Some(version) => Ok(version),
    }
}

/// Parse one `<id> <key>=<int> ...` row into its version plus [`LoopTables`].
/// Unknown, duplicate or missing keys refuse with the line number.
fn parse_row(line: &str, lineno: u32) -> Result<(u32, LoopTables), LoopRowsError> {
    let mut words = line.split_whitespace();
    let id = words.next().ok_or(LoopRowsError::RowId(lineno))?;
    let (prefix, digits) = id.split_at_checked(1).ok_or(LoopRowsError::RowId(lineno))?;
    if prefix != "v" {
        return Err(LoopRowsError::RowId(lineno));
    }
    let row_version = match decimal(digits) {
        Some(0) | None => return Err(LoopRowsError::RowId(lineno)),
        Some(version) => version,
    };
    let mut values: [Option<u32>; 8] = [None; 8];
    for pair in words {
        let (key, val) = pair
            .split_once('=')
            .ok_or(LoopRowsError::UnknownKey(lineno))?;
        let mut slot: Option<usize> = None;
        for (index, known) in KNOWN.iter().enumerate() {
            if *known == key {
                slot = Some(index);
                break;
            }
        }
        let index = slot.ok_or(LoopRowsError::UnknownKey(lineno))?;
        if values[index].is_some() {
            return Err(LoopRowsError::Duplicate {
                field: KNOWN[index],
                line: lineno,
            });
        }
        let number = decimal(val).ok_or(LoopRowsError::BadInt {
            field: KNOWN[index],
            line: lineno,
        })?;
        values[index] = Some(number);
    }
    for (index, known) in KNOWN.iter().enumerate() {
        if values[index].is_none() {
            return Err(LoopRowsError::Missing {
                field: *known,
                line: lineno,
            });
        }
    }
    let get = |index: usize| values[index].unwrap_or(0);
    let tables = LoopTables {
        glory_per_win: get(0),
        glory_rung_bonus: get(1),
        glory_per_loop: get(2),
        glory_ranks: [get(3), get(4), get(5), get(6), get(7)],
    };
    validate_loop(&tables, lineno)?;
    Ok((row_version, tables))
}

/// One inclusive-range check with the field name carried on failure.
fn range(
    field: &'static str,
    value: u32,
    min: u32,
    max: u32,
    line: u32,
) -> Result<(), LoopRowsError> {
    if value < min || value > max {
        return Err(LoopRowsError::Invalid { field, line });
    }
    Ok(())
}

/// Range-check loop rows. First failure wins, deterministically.
///
/// Rules: awards sit inside their caps; rank floors sit inside theirs
/// with the first at zero and each later floor strictly above the one
/// before, so ranks step up exactly once per floor.
fn validate_loop(tables: &LoopTables, line: u32) -> Result<(), LoopRowsError> {
    range(
        "glory_per_win",
        tables.glory_per_win,
        0,
        MAX_GLORY_WIN,
        line,
    )?;
    range(
        "glory_rung_bonus",
        tables.glory_rung_bonus,
        0,
        MAX_RUNG_BONUS,
        line,
    )?;
    range(
        "glory_per_loop",
        tables.glory_per_loop,
        0,
        MAX_GLORY_LOOP,
        line,
    )?;
    for (index, floor) in tables.glory_ranks.iter().enumerate() {
        range(KNOWN[3 + index], *floor, 0, MAX_RANK_FLOOR, line)?;
    }
    if tables.glory_ranks[0] != 0 {
        return Err(LoopRowsError::Invalid {
            field: "glory_rank0",
            line,
        });
    }
    for index in 1..tables.glory_ranks.len() {
        if tables.glory_ranks[index - 1] >= tables.glory_ranks[index] {
            return Err(LoopRowsError::Invalid {
                field: KNOWN[3 + index],
                line,
            });
        }
    }
    Ok(())
}

/// Parse one rows file into the header-version [`LoopSet`].
///
/// The file holds every shipped row as reference; the row whose `v<n>`
/// id matches the header version becomes the set.
pub fn parse_loop(text: &str) -> Result<LoopSet, LoopRowsError> {
    let mut lineno: u32 = 0;
    let mut header_version: Option<u32> = None;
    let mut header_line: u32 = 0;
    let mut rows: Vec<(u32, LoopTables)> = Vec::new();
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
            for (seen, _) in &rows {
                if *seen == version {
                    return Err(LoopRowsError::RowId(lineno));
                }
            }
            rows.push((version, tables));
        }
    }
    let wanted = header_version.ok_or(LoopRowsError::Header(0))?;
    if rows.is_empty() {
        return Err(LoopRowsError::RowId(header_line));
    }
    for (version, tables) in &rows {
        if *version == wanted {
            return Ok(LoopSet {
                version: *version,
                tables: *tables,
            });
        }
    }
    Err(LoopRowsError::RowId(header_line))
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1_LINE: &str = "v1 glory_per_win=100 glory_rung_bonus=25 glory_per_loop=500 glory_rank0=0 glory_rank1=300 glory_rank2=900 glory_rank3=1800 glory_rank4=3600";

    fn v1_file() -> String {
        ["table loop version 1", V1_LINE].join("\n")
    }

    #[test]
    fn v1_row_text_equals_compiled() {
        let (version, tables) = parse_row(V1_LINE, 2).expect("v1 reference parses");
        assert_eq!(version, 1);
        assert_eq!(tables.glory_per_win, GLORY_PER_WIN);
        assert_eq!(tables.glory_rung_bonus, GLORY_RUNG_BONUS);
        assert_eq!(tables.glory_per_loop, GLORY_PER_LOOP);
        assert_eq!(tables.glory_ranks, GLORY_RANKS);
    }

    #[test]
    fn shipped_file_holds_v1() {
        let text = include_str!("../../../content/design/loop.rows");
        let set = parse_loop(text).expect("shipped file parses to v1");
        assert_eq!(set.version, 1);
        assert_eq!(set.tables.glory_per_win, GLORY_PER_WIN);
        assert_eq!(set.tables.glory_ranks, GLORY_RANKS);
    }

    #[test]
    fn decimal_refuses_empty_plus_nondigits_and_overflow() {
        assert_eq!(decimal(""), None);
        assert_eq!(decimal("+12"), None);
        assert_eq!(decimal("12.0"), None);
        assert_eq!(decimal("12 "), None);
        assert_eq!(decimal("nope"), None);
        assert_eq!(decimal("1_000"), None);
        assert_eq!(decimal("4294967296"), None);
        assert_eq!(decimal("007"), Some(7));
        assert_eq!(decimal("0"), Some(0));
        assert_eq!(decimal("4294967295"), Some(u32::MAX));
    }

    #[test]
    fn bad_value_is_refused() {
        let bad = v1_file().replace("glory_per_win=100", "glory_per_win=20000");
        assert_eq!(
            parse_loop(&bad),
            Err(LoopRowsError::Invalid {
                field: "glory_per_win",
                line: 2
            })
        );
        let flat = v1_file().replace("glory_rank2=900", "glory_rank2=300");
        assert_eq!(
            parse_loop(&flat),
            Err(LoopRowsError::Invalid {
                field: "glory_rank2",
                line: 2
            })
        );
    }

    #[test]
    fn float_unknown_and_missing_refuse_with_line() {
        let float = v1_file().replace("glory_per_win=100", "glory_per_win=10.0");
        assert_eq!(
            parse_loop(&float),
            Err(LoopRowsError::BadInt {
                field: "glory_per_win",
                line: 2
            })
        );
        let unknown = v1_file().replace("glory_per_win=100", "nope=1");
        assert_eq!(parse_loop(&unknown), Err(LoopRowsError::UnknownKey(2)));
        let missing = [
            "table loop version 1",
            &V1_LINE.replace(" glory_per_loop=500", ""),
        ]
        .join("\n");
        assert_eq!(
            parse_loop(&missing),
            Err(LoopRowsError::Missing {
                field: "glory_per_loop",
                line: 2
            })
        );
    }
}
