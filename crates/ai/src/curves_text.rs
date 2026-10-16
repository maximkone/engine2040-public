//! ai rows text: designer numbers without a Rust edit.
//!
//! Consumer: `Brain::with_rows_text` in `gladiator.rs` builds a brain from
//! rows text. Data owner: [`AiConfig`] owns curves plus morale. Units:
//! aggression/caution/morale `0..=1000`, `engage_range_cm` centimetres.
//! Determinism: same text gives same config; integer math only. Errors:
//! [`AiRowsError`] carries the 1-based line number; range checks reuse
//! [`validate_config`](crate::curves::validate_config).
//! Caller: `crates/ai/src/gladiator.rs` (`Brain::with_rows_text`; cold only, never tick).

use crate::curves::{parse_dec_u32, AiConfig, AiCurveError, AiCurves, CURVE_LANES};

/// Rows parse failure with its 1-based line number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AiRowsError {
    /// First content line is not `table ai version <n>`.
    Header { line: usize },
    /// Header version is zero or unreadable.
    Version { line: usize },
    /// Row id is not `v<n>` or a second row exists.
    RowId { line: usize },
    /// Key is unknown or repeated.
    UnknownKey { line: usize },
    /// An expected key never appears.
    MissingKey { line: usize },
    /// Value is not ASCII digits.
    BadInt { line: usize },
    /// Value holds a float point.
    Float { line: usize },
    /// Keys parse but [`AiConfig::try_new`] refuses them.
    Invalid { line: usize, source: AiCurveError },
}

impl core::fmt::Display for AiRowsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Header { line } => write!(f, "ai rows line {line}: want `table ai version <n>`"),
            Self::Version { line } => write!(f, "ai rows line {line}: version is zero"),
            Self::RowId { line } => write!(f, "ai rows line {line}: want row `v<n>`"),
            Self::UnknownKey { line } => write!(f, "ai rows line {line}: unknown key"),
            Self::MissingKey { line } => write!(f, "ai rows line {line}: missing key"),
            Self::BadInt { line } => write!(f, "ai rows line {line}: want digits"),
            Self::Float { line } => write!(f, "ai rows line {line}: no floats"),
            Self::Invalid { line, source } => write!(f, "ai rows line {line}: {source}"),
        }
    }
}

impl std::error::Error for AiRowsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// One parsed key destination.
#[derive(Clone, Copy)]
enum Slot {
    Agg(usize),
    Cau(usize),
    Engage,
    Morale,
}

/// Map a key to its slot; `None` is unknown.
fn slot_of(key: &str) -> Option<Slot> {
    if key == "engage_range_cm" {
        return Some(Slot::Engage);
    }
    if key == "morale_default" {
        return Some(Slot::Morale);
    }
    if let Some(rest) = key.strip_prefix("aggression") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            let idx = usize::from(rest.as_bytes()[0] - b'0');
            if idx < CURVE_LANES {
                return Some(Slot::Agg(idx));
            }
        }
        return None;
    }
    if let Some(rest) = key.strip_prefix("caution") {
        if rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() {
            let idx = usize::from(rest.as_bytes()[0] - b'0');
            if idx < CURVE_LANES {
                return Some(Slot::Cau(idx));
            }
        }
        return None;
    }
    None
}

/// Header `table ai version <n>` to its nonzero version.
fn header_version(line: &str, no: usize) -> Result<u32, AiRowsError> {
    let mut parts = line.split_whitespace();
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some("table"), Some("ai"), Some("version"), Some(num), None) => {
            if num.contains('.') {
                return Err(AiRowsError::Float { line: no });
            }
            let v = parse_dec_u32(num).ok_or(AiRowsError::Header { line: no })?;
            if v == 0 {
                return Err(AiRowsError::Version { line: no });
            }
            Ok(v)
        }
        _ => Err(AiRowsError::Header { line: no }),
    }
}

/// Parse designer rows text into a validated [`AiConfig`].
///
/// Layout: line 1 `table ai version <n>`; then exactly one row
/// `v<n> aggression0=<int> ... caution0=<int> ... engage_range_cm=<int>
/// morale_default=<int>`; `#` starts a comment, blank lines ignored.
/// Whole numbers only; a float, unknown or missing key is refused with its
/// line number; ranges reuse `AiConfig::try_new`.
pub fn parse_ai_config(text: &str) -> Result<AiConfig, AiRowsError> {
    let mut version: Option<u32> = None;
    let mut header_no = 0;
    let mut row_no = 0;
    let mut row_body: Option<&str> = None;
    for (idx, raw) in text.lines().enumerate() {
        let no = idx.saturating_add(1);
        let cut = raw.find('#').map_or(raw, |i| &raw[..i]);
        let line = cut.trim();
        if line.is_empty() {
            continue;
        }
        if version.is_none() {
            version = Some(header_version(line, no)?);
            header_no = no;
        } else if row_body.is_none() {
            row_body = Some(line);
            row_no = no;
        } else {
            return Err(AiRowsError::RowId { line: no });
        }
    }
    let v = version.ok_or(AiRowsError::Header { line: 1 })?;
    let body = row_body.ok_or(AiRowsError::MissingKey { line: header_no })?;
    parse_row(body, row_no, v)
}

/// Parse the single `v<n>` row body.
fn parse_row(body: &str, no: usize, version: u32) -> Result<AiConfig, AiRowsError> {
    let mut words = body.split_whitespace();
    let id = words.next().ok_or(AiRowsError::RowId { line: no })?;
    let num = id
        .strip_prefix('v')
        .ok_or(AiRowsError::RowId { line: no })?;
    if parse_dec_u32(num) != Some(version) {
        return Err(AiRowsError::RowId { line: no });
    }
    let mut agg = [0_u16; CURVE_LANES];
    let mut cau = [0_u16; CURVE_LANES];
    let mut seen_agg = [false; CURVE_LANES];
    let mut seen_cau = [false; CURVE_LANES];
    let mut engage: Option<u32> = None;
    let mut morale: Option<i32> = None;
    for token in words {
        let (key, val) = token
            .split_once('=')
            .ok_or(AiRowsError::UnknownKey { line: no })?;
        if val.contains('=') || key.is_empty() || val.is_empty() {
            return Err(AiRowsError::BadInt { line: no });
        }
        if val.contains('.') {
            return Err(AiRowsError::Float { line: no });
        }
        let num = parse_dec_u32(val).ok_or(AiRowsError::BadInt { line: no })?;
        match slot_of(key).ok_or(AiRowsError::UnknownKey { line: no })? {
            Slot::Agg(i) => {
                if seen_agg[i] {
                    return Err(AiRowsError::UnknownKey { line: no });
                }
                seen_agg[i] = true;
                agg[i] = u16::try_from(num).map_err(|_| AiRowsError::BadInt { line: no })?;
            }
            Slot::Cau(i) => {
                if seen_cau[i] {
                    return Err(AiRowsError::UnknownKey { line: no });
                }
                seen_cau[i] = true;
                cau[i] = u16::try_from(num).map_err(|_| AiRowsError::BadInt { line: no })?;
            }
            Slot::Engage => {
                if engage.is_some() {
                    return Err(AiRowsError::UnknownKey { line: no });
                }
                engage = Some(num);
            }
            Slot::Morale => {
                if morale.is_some() {
                    return Err(AiRowsError::UnknownKey { line: no });
                }
                morale = Some(i32::try_from(num).map_err(|_| AiRowsError::BadInt { line: no })?);
            }
        }
    }
    for i in 0..CURVE_LANES {
        if !seen_agg[i] || !seen_cau[i] {
            return Err(AiRowsError::MissingKey { line: no });
        }
    }
    let engage = engage.ok_or(AiRowsError::MissingKey { line: no })?;
    let morale = morale.ok_or(AiRowsError::MissingKey { line: no })?;
    let curves = AiCurves {
        version,
        aggression: agg,
        caution: cau,
        engage_range_cm: engage,
    };
    AiConfig::try_new(curves, morale).map_err(|source| AiRowsError::Invalid { line: no, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curves::AiConfig;

    const V1: &str = "table ai version 1\nv1 aggression0=0 aggression1=125 aggression2=250 aggression3=375 aggression4=500 aggression5=625 aggression6=750 aggression7=1000 caution0=500 caution1=500 caution2=500 caution3=500 caution4=500 caution5=500 caution6=500 caution7=500 engage_range_cm=250 morale_default=500";

    #[test]
    fn v1_text_equals_defaults() {
        let file = include_str!("../../../content/design/ai.rows");
        let got = parse_ai_config(file).expect("v1 rows parse");
        assert_eq!(got, AiConfig::scripted_defaults());
        assert_eq!(parse_ai_config(V1).expect("inline v1"), got);
    }

    #[test]
    fn bad_lines_refuse_with_line() {
        let bad_range = V1.replace("engage_range_cm=250", "engage_range_cm=49");
        assert!(matches!(
            parse_ai_config(&bad_range),
            Err(AiRowsError::Invalid { line: 2, .. })
        ));
        let unknown = V1.replace("morale_default=500", "morale=500");
        assert_eq!(
            parse_ai_config(&unknown),
            Err(AiRowsError::UnknownKey { line: 2 })
        );
        let missing = V1.replace(" morale_default=500", "");
        assert_eq!(
            parse_ai_config(&missing),
            Err(AiRowsError::MissingKey { line: 2 })
        );
        let float = V1.replace("aggression1=125", "aggression1=12.5");
        assert_eq!(parse_ai_config(&float), Err(AiRowsError::Float { line: 2 }));
    }

    #[test]
    fn brain_wires_rows_text() {
        use crate::gladiator::Brain;
        use engine_input::FighterId;
        let file = include_str!("../../../content/design/ai.rows");
        let id = FighterId::new(0).expect("slot 0");
        let brain = Brain::with_rows_text(id, 1, file).expect("rows build a brain");
        assert_eq!(*brain.config(), AiConfig::scripted_defaults());
        assert!(Brain::with_rows_text(id, 1, "table ai version 0\nv0 aggression0=0").is_err());
    }
}
