//! rows text: the one scanner behind every designer `key=value` row.
//!
//! Consumers: [`parse_tiers`](crate::depth::parse_tiers) (enemy tiers) and
//! [`parse_ai_config`](crate::curves_text::parse_ai_config) (ai curves) both
//! strip `#` comments and walk their row fields here, so the two doors refuse
//! the same shapes; each caller still maps a refusal to its own error type.
//! Units: the value is a whole non-negative number read by
//! [`parse_dec_u32`](crate::curves::parse_dec_u32). Determinism: no float, no
//! allocation, borrow only. Caller: cold only, at match build, never tick.

use crate::curves::parse_dec_u32;

/// Why one row field refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldError {
    /// Token holds no `=`.
    NoEquals,
    /// Key before `=` is empty.
    EmptyKey,
    /// Value after `=` is empty.
    EmptyValue,
    /// Value holds a `.`: a float, not a whole number.
    Float,
    /// Value is not ASCII digits.
    BadInt,
}

/// One scanned `key=value` field: the key and its whole value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Field<'a> {
    /// Key before `=`.
    pub key: &'a str,
    /// Whole value after `=`, read by
    /// [`parse_dec_u32`](crate::curves::parse_dec_u32).
    pub value: u32,
}

/// Strip a `#` comment and trim the surrounding space.
///
/// The one line cleaner for rows text: everything from `#` on is dropped,
/// then the line is trimmed, so a blank or comment-only line reads empty.
pub fn strip_comment(raw: &str) -> &str {
    let cut = raw.find('#').map_or(raw, |i| raw.split_at(i).0);
    cut.trim()
}

/// The `key=value` fields of one row line, its leading id token skipped.
#[derive(Clone, Debug)]
pub struct Fields<'a> {
    rest: core::str::SplitWhitespace<'a>,
}

impl<'a> Fields<'a> {
    /// Fields of `line` after the first whitespace token (the row id).
    pub fn after_id(line: &'a str) -> Self {
        let mut rest = line.split_whitespace();
        rest.next();
        Self { rest }
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Result<Field<'a>, FieldError>;

    fn next(&mut self) -> Option<Self::Item> {
        let token = self.rest.next()?;
        Some(scan_field(token))
    }
}

/// Read one `key=value` token into its field.
///
/// An empty value is reported before an empty key, so a lone `=` stays a bad
/// integer at both doors.
fn scan_field(token: &str) -> Result<Field<'_>, FieldError> {
    let (key, val) = token.split_once('=').ok_or(FieldError::NoEquals)?;
    if val.is_empty() {
        return Err(FieldError::EmptyValue);
    }
    if key.is_empty() {
        return Err(FieldError::EmptyKey);
    }
    if val.contains('.') {
        return Err(FieldError::Float);
    }
    let value = parse_dec_u32(val).ok_or(FieldError::BadInt)?;
    Ok(Field { key, value })
}
