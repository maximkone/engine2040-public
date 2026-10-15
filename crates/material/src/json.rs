//! json: a small deterministic JSON reader and writer for material text.
//!
//! Owner: `engine-material`. The record is text an AI writes, so the reader
//! keeps every number as its lexeme (the sim half converts decimals to Q16
//! in integer math, the look half parses `f32`) and keeps object keys in
//! document order with duplicates refused. The writer emits one canonical
//! pretty form (two-space indent, scalar arrays inline, `\n` line ends) so
//! `write(parse(write(x))) == write(x)` byte for byte on every host.
//! Bounds: [`MAX_JSON_BYTES`] and [`MAX_DEPTH`]. Errors: [`JsonError`] with
//! the byte offset; nothing here panics on user text.

use std::fmt;

/// Largest accepted document; material records are a few KiB.
pub const MAX_JSON_BYTES: usize = 64 * 1024;
/// Deepest accepted nesting; records nest four levels.
pub const MAX_DEPTH: usize = 32;

/// One JSON value. Numbers stay as their source lexeme so the caller picks
/// the conversion (exact decimal to Q16, or `f32`).
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number lexeme as written (`-?int(.frac)?(e[+-]?digits)?`).
    Number(String),
    /// A string with escapes resolved.
    Str(String),
    /// An array in document order.
    Array(Vec<Value>),
    /// An object in document order; keys are unique.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Member `key` of an object; `None` for other kinds or a missing key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string, if this is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(text) => Some(text),
            _ => None,
        }
    }

    /// The number lexeme, if this is a number.
    #[must_use]
    pub fn number_text(&self) -> Option<&str> {
        match self {
            Self::Number(text) => Some(text),
            _ => None,
        }
    }

    /// The elements, if this is an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The members, if this is an object.
    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Object(members) => Some(members),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Short kind name for diagnostics.
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Number(_) => "number",
            Self::Str(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }
}

/// Why a document was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JsonErrorKind {
    /// Document larger than [`MAX_JSON_BYTES`]; size carried.
    TooLarge {
        /// Offending byte count.
        bytes: usize,
    },
    /// Text ended inside a value.
    UnexpectedEnd,
    /// A character that cannot start or continue the value here.
    UnexpectedChar(char),
    /// A number lexeme JSON does not allow (leading zero, lone minus, bad exponent).
    BadNumber,
    /// An escape other than `\" \\ \/ \b \f \n \r \t \uXXXX`.
    BadEscape,
    /// A `\u` escape that is not four hex digits or a bad surrogate pair.
    BadUnicode,
    /// A raw control character inside a string.
    ControlChar,
    /// The same key twice in one object; key carried.
    DuplicateKey(String),
    /// Nesting deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Text after the top-level value.
    TrailingText,
}

/// A refused document: what and at which byte offset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct JsonError {
    /// Byte offset of the offending character (document length at the end).
    pub offset: usize,
    /// What was wrong.
    pub kind: JsonErrorKind,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json at byte {}: ", self.offset)?;
        match &self.kind {
            JsonErrorKind::TooLarge { bytes } => {
                write!(f, "{bytes} bytes exceed {MAX_JSON_BYTES}")
            }
            JsonErrorKind::UnexpectedEnd => f.write_str("unexpected end of text"),
            JsonErrorKind::UnexpectedChar(ch) => write!(f, "unexpected character {ch:?}"),
            JsonErrorKind::BadNumber => f.write_str("malformed number"),
            JsonErrorKind::BadEscape => f.write_str("unknown string escape"),
            JsonErrorKind::BadUnicode => f.write_str("bad \\u escape"),
            JsonErrorKind::ControlChar => f.write_str("raw control character in string"),
            JsonErrorKind::DuplicateKey(key) => write!(f, "duplicate key {key:?}"),
            JsonErrorKind::TooDeep => write!(f, "nesting deeper than {MAX_DEPTH}"),
            JsonErrorKind::TrailingText => f.write_str("text after the value"),
        }
    }
}

impl std::error::Error for JsonError {}

/// Parse one JSON document. Whitespace around the value is allowed; anything
/// else after it is [`JsonErrorKind::TrailingText`].
pub fn parse(text: &str) -> Result<Value, JsonError> {
    parse_up_to(text, MAX_JSON_BYTES)
}

/// [`parse`] with a caller's byte cap: the importers read documents (a glTF
/// JSON chunk) far larger than a material record.
pub fn parse_up_to(text: &str, max_bytes: usize) -> Result<Value, JsonError> {
    if text.len() > max_bytes {
        return Err(JsonError {
            offset: 0,
            kind: JsonErrorKind::TooLarge { bytes: text.len() },
        });
    }
    let mut cursor = Cursor {
        bytes: text.as_bytes(),
        at: 0,
    };
    cursor.skip_ws();
    let value = cursor.value(0)?;
    cursor.skip_ws();
    if cursor.at < cursor.bytes.len() {
        return Err(cursor.fail(JsonErrorKind::TrailingText));
    }
    Ok(value)
}

/// Byte cursor over the document; every method leaves `at` on the next
/// unread byte or returns an error naming the offset.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn fail(&self, kind: JsonErrorKind) -> JsonError {
        JsonError {
            offset: self.at,
            kind,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect_literal(&mut self, word: &[u8], value: Value) -> Result<Value, JsonError> {
        for (i, &expected) in word.iter().enumerate() {
            match self.bytes.get(self.at + i) {
                None => {
                    return Err(JsonError {
                        offset: self.at + i,
                        kind: JsonErrorKind::UnexpectedEnd,
                    })
                }
                Some(&byte) if byte != expected => {
                    return Err(JsonError {
                        offset: self.at + i,
                        kind: JsonErrorKind::UnexpectedChar(char::from(byte)),
                    })
                }
                Some(_) => {}
            }
        }
        self.at += word.len();
        Ok(value)
    }

    fn unexpected(&self) -> JsonError {
        match self.peek() {
            None => self.fail(JsonErrorKind::UnexpectedEnd),
            Some(byte) => self.fail(JsonErrorKind::UnexpectedChar(char::from(byte))),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.fail(JsonErrorKind::TooDeep));
        }
        match self.peek() {
            None => Err(self.fail(JsonErrorKind::UnexpectedEnd)),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') => self.expect_literal(b"true", Value::Bool(true)),
            Some(b'f') => self.expect_literal(b"false", Value::Bool(false)),
            Some(b'n') => self.expect_literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.unexpected()),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.at += 1;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.unexpected());
            }
            let key_at = self.at;
            let key = self.string()?;
            if members.iter().any(|(seen, _)| *seen == key) {
                return Err(JsonError {
                    offset: key_at,
                    kind: JsonErrorKind::DuplicateKey(key),
                });
            }
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.unexpected());
            }
            self.at += 1;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.unexpected()),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.fail(JsonErrorKind::BadNumber));
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.fail(JsonErrorKind::BadNumber)),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(self.fail(JsonErrorKind::BadNumber));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(self.fail(JsonErrorKind::BadNumber));
            }
        }
        let lexeme = std::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| self.fail(JsonErrorKind::BadNumber))?;
        Ok(Value::Number(lexeme.to_string()))
    }

    fn escape_point(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.fail(JsonErrorKind::UnexpectedEnd))?;
        let code =
            engine_core::parse_hex4(digits).ok_or_else(|| self.fail(JsonErrorKind::BadUnicode))?;
        self.at += 4;
        Ok(code)
    }

    fn unicode_escape(&mut self, out: &mut String) -> Result<(), JsonError> {
        let escape_at = self.at;
        let high = self.escape_point()?;
        let code = if (0xD800..0xDC00).contains(&high) {
            if self.bytes.get(self.at..self.at + 2) != Some(b"\\u") {
                return Err(JsonError {
                    offset: escape_at,
                    kind: JsonErrorKind::BadUnicode,
                });
            }
            self.at += 2;
            let low = self.escape_point()?;
            if !(0xDC00..0xE000).contains(&low) {
                return Err(JsonError {
                    offset: escape_at,
                    kind: JsonErrorKind::BadUnicode,
                });
            }
            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
        } else {
            high
        };
        let Some(ch) = char::from_u32(code) else {
            return Err(JsonError {
                offset: escape_at,
                kind: JsonErrorKind::BadUnicode,
            });
        };
        out.push(ch);
        Ok(())
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.fail(JsonErrorKind::UnexpectedEnd));
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let Some(escape) = self.peek() else {
                        return Err(self.fail(JsonErrorKind::UnexpectedEnd));
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => self.unicode_escape(&mut out)?,
                        _ => {
                            // Point at the backslash that opened the escape.
                            self.at -= 2;
                            return Err(self.fail(JsonErrorKind::BadEscape));
                        }
                    }
                }
                0x00..=0x1f => return Err(self.fail(JsonErrorKind::ControlChar)),
                _ => {
                    // Copy one whole UTF-8 scalar: the document is `&str`, so
                    // the lead byte tells the length and the slice is valid.
                    let end = (self.at + utf8_len(byte)).min(self.bytes.len());
                    let piece = std::str::from_utf8(&self.bytes[self.at..end])
                        .map_err(|_| self.fail(JsonErrorKind::ControlChar))?;
                    out.push_str(piece);
                    self.at = end;
                }
            }
        }
    }
}

/// Byte length of the UTF-8 sequence that starts with `lead`.
fn utf8_len(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead < 0xE0 {
        2
    } else if lead < 0xF0 {
        3
    } else {
        4
    }
}

/// Canonical pretty text of a value (see the module docs). Ends without a
/// trailing newline; callers add one when writing a file.
#[must_use]
pub fn write(value: &Value) -> String {
    let mut out = String::new();
    write_into(&mut out, value, 0);
    out
}

/// Append `text` as a JSON string literal with escapes.
fn write_str(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_into(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(lexeme) => out.push_str(lexeme),
        Value::Str(text) => write_str(out, text),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) if items.iter().all(is_scalar) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_into(out, item, depth + 1);
            }
            out.push(']');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                write_into(out, item, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push(']');
        }
        Value::Object(members) if members.is_empty() => out.push_str("{}"),
        Value::Object(members) => {
            out.push_str("{\n");
            for (i, (key, member)) in members.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                indent(out, depth + 1);
                write_str(out, key);
                out.push_str(": ");
                write_into(out, member, depth + 1);
            }
            out.push('\n');
            indent(out, depth);
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_at(text: &str) -> (usize, JsonErrorKind) {
        let err = parse(text).expect_err("must refuse");
        (err.offset, err.kind)
    }

    #[test]
    fn parses_nested_document_in_order() {
        let doc = parse(r#"{"b": [1, 2.5, -3e2], "a": {"x": "y\né"}, "n": null, "t": true}"#)
            .expect("valid");
        let members = doc.as_object().expect("object");
        assert_eq!(members[0].0, "b");
        assert_eq!(members[1].0, "a");
        assert_eq!(
            doc.get("b").and_then(Value::as_array).map(<[Value]>::len),
            Some(3)
        );
        assert_eq!(
            doc.get("b")
                .and_then(Value::as_array)
                .map(|a| a[2].number_text()),
            Some(Some("-3e2"))
        );
        assert_eq!(
            doc.get("a")
                .and_then(|a| a.get("x"))
                .and_then(Value::as_str),
            Some("y\n\u{e9}")
        );
        assert_eq!(doc.get("n"), Some(&Value::Null));
        assert_eq!(doc.get("t").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn surrogate_pairs_decode_and_bad_ones_refuse() {
        let doc = parse(r#""😀""#).expect("valid pair");
        assert_eq!(doc.as_str(), Some("\u{1F600}"));
        assert_eq!(kind_at(r#""\ud83dx""#).1, JsonErrorKind::BadUnicode);
        assert_eq!(kind_at(r#""\ud83dA""#).1, JsonErrorKind::BadUnicode);
        assert_eq!(kind_at(r#""\uzzzz""#).1, JsonErrorKind::BadUnicode);
    }

    #[test]
    fn refusals_name_the_offset() {
        assert_eq!(
            kind_at("{\"a\": 1,}"),
            (8, JsonErrorKind::UnexpectedChar('}'))
        );
        assert_eq!(kind_at("[1, 2"), (5, JsonErrorKind::UnexpectedEnd));
        assert_eq!(kind_at("01"), (1, JsonErrorKind::BadNumber));
        assert_eq!(kind_at("-"), (1, JsonErrorKind::BadNumber));
        assert_eq!(kind_at("1."), (2, JsonErrorKind::BadNumber));
        assert_eq!(kind_at("1e"), (2, JsonErrorKind::BadNumber));
        assert_eq!(kind_at(r#""\q""#), (1, JsonErrorKind::BadEscape));
        assert_eq!(kind_at("\"a\tb\""), (2, JsonErrorKind::ControlChar));
        assert_eq!(kind_at("{} x"), (3, JsonErrorKind::TrailingText));
        assert_eq!(kind_at("tru"), (3, JsonErrorKind::UnexpectedEnd));
        assert_eq!(kind_at("trux"), (3, JsonErrorKind::UnexpectedChar('x')));
        assert_eq!(kind_at("@"), (0, JsonErrorKind::UnexpectedChar('@')));
    }

    #[test]
    fn duplicate_keys_refuse_at_the_second_key() {
        assert_eq!(
            kind_at(r#"{"a": 1, "a": 2}"#),
            (9, JsonErrorKind::DuplicateKey("a".to_string()))
        );
    }

    #[test]
    fn depth_and_size_caps_hold() {
        let deep = "[".repeat(MAX_DEPTH + 2);
        assert_eq!(kind_at(&deep).1, JsonErrorKind::TooDeep);
        let shallow = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&shallow).is_ok());
        let big = format!("\"{}\"", "x".repeat(MAX_JSON_BYTES));
        assert_eq!(
            kind_at(&big).1,
            JsonErrorKind::TooLarge {
                bytes: MAX_JSON_BYTES + 2
            }
        );
    }

    #[test]
    fn parse_up_to_honours_a_caller_cap() {
        let doc = r#"{"a": 1}"#;
        let capped = parse_up_to(doc, doc.len()).expect("exact cap parses");
        assert_eq!(capped, parse(doc).expect("default cap parses"));
    }

    #[test]
    fn parse_up_to_refuses_over_the_cap() {
        let doc = r#"{"a": 1}"#;
        let err = parse_up_to(doc, doc.len() - 1).expect_err("over cap refuses");
        assert_eq!(
            err,
            JsonError {
                offset: 0,
                kind: JsonErrorKind::TooLarge { bytes: doc.len() },
            }
        );
    }

    #[test]
    fn writer_is_canonical_and_round_trips() {
        let text = "{\n  \"look\": {\n    \"base_color\": [0.82, 0.8, 0.76],\n    \"name\": \"a\\\"b\"\n  },\n  \"layers\": [\n    {\n      \"kind\": \"wet\"\n    }\n  ],\n  \"empty\": {},\n  \"none\": []\n}";
        let value = parse(text).expect("valid");
        let written = write(&value);
        assert_eq!(written, text);
        let again = write(&parse(&written).expect("valid twice"));
        assert_eq!(again, written);
    }

    #[test]
    fn writer_escapes_control_characters() {
        let value = Value::Str("a\u{1}b\u{8}".to_string());
        assert_eq!(write(&value), "\"a\\u0001b\\b\"");
        let back = parse(&write(&value)).expect("valid");
        assert_eq!(back, value);
    }

    #[test]
    fn kind_names_cover_every_variant() {
        let names: Vec<&str> = [
            Value::Null,
            Value::Bool(false),
            Value::Number("1".to_string()),
            Value::Str(String::new()),
            Value::Array(vec![]),
            Value::Object(vec![]),
        ]
        .iter()
        .map(Value::kind_name)
        .collect();
        assert_eq!(
            names,
            ["null", "bool", "number", "string", "array", "object"]
        );
    }

    #[test]
    fn error_display_names_offset_and_cause() {
        let err = parse("{\"a\": 1, \"a\": 2}").expect_err("dup");
        assert_eq!(err.to_string(), "json at byte 9: duplicate key \"a\"");
    }
}
