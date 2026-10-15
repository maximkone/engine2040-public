//! Shared parse errors and tokenizer core for the hand-rolled fixture grammars.
//!
//! Consumer: `keypoint`, `living` and `spatial_eval` parse their fixture
//! files through this module. Data owner: the caller owns the input text;
//! [`Cursor`] borrows it and owns nothing. Units: byte offsets in `pos`.
//! Determinism: pure functions over the input bytes; the same text twice
//! gives the same value or the same error. Errors: [`ParseError`]; the
//! grammars stay separate by design and only share this error type plus the
//! byte cursor and the `k=v` splitter below.

use std::fmt;

/// One typed failure for every hand-rolled fixture parser in this crate.
/// Tokenizer faults carry their byte offset; grammar rejections carry the
/// decision text with the legal values named, so a refusal always says what
/// was legal. The three grammars keep their own shapes and only share this.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// Bytes remain after the top-level value; offset carried.
    TrailingBytes {
        /// Byte offset of the first trailing byte.
        pos: usize,
    },
    /// Nesting past the 16-deep bound; offset carried.
    NestingTooDeep {
        /// Byte offset where the bound tripped.
        pos: usize,
    },
    /// A byte no grammar accepts here; byte plus offset carried.
    UnexpectedByte {
        /// Byte offset of the offending byte.
        pos: usize,
        /// The offending byte.
        byte: u8,
    },
    /// Input ended where a value was required; offset carried.
    EndOfInput {
        /// Byte offset of the end of input.
        pos: usize,
    },
    /// A `true`/`false`/`null` word misspelled; offset carried.
    BadLiteral {
        /// Byte offset where the word should start.
        pos: usize,
    },
    /// A string never closed; offset carried.
    UnterminatedString {
        /// Byte offset of the end of input.
        pos: usize,
    },
    /// A `\` escape never closed; offset carried.
    UnterminatedEscape {
        /// Byte offset of the end of input.
        pos: usize,
    },
    /// A `\` escape outside the grammar; offset plus the escaped byte when
    /// one follows, `None` when input ends right after the backslash.
    BadEscape {
        /// Byte offset of the escape.
        pos: usize,
        /// The escaped byte when one follows.
        byte: Option<u8>,
    },
    /// A raw control byte inside a string; offset carried.
    ControlByte {
        /// Byte offset of the control byte.
        pos: usize,
    },
    /// Bytes that are not valid UTF-8, or a character that cannot be decoded
    /// here; offset carried.
    BadUtf8 {
        /// Byte offset where decoding failed.
        pos: usize,
    },
    /// A `\u` escape with fewer than four hex digits; offset carried.
    ShortUnicode {
        /// Byte offset of the `\u` escape.
        pos: usize,
    },
    /// A non-hex digit inside `\uXXXX`; offset carried.
    BadHex {
        /// Byte offset of the offending digit.
        pos: usize,
    },
    /// A `\uXXXX` unit with no scalar value; unit carried.
    BadScalar {
        /// The rejected unit value.
        unit: u32,
    },
    /// A number with no digits; offset carried.
    BadNumber {
        /// Byte offset where the number starts.
        pos: usize,
    },
    /// A `.` with no fraction digits; offset carried.
    BadFraction {
        /// Byte offset where the number starts.
        pos: usize,
    },
    /// An exponent marker with no digits; offset carried.
    BadExponent {
        /// Byte offset where the number starts.
        pos: usize,
    },
    /// A numeric token the grammar cannot convert; token carried.
    BadToken {
        /// The rejected token text.
        token: String,
    },
    /// A numeric token outside the finite range; token carried.
    NonFinite {
        /// The rejected token text.
        token: String,
    },
    /// A float where the grammar takes integers only; offset carried.
    FloatRefused {
        /// Byte offset where the number starts.
        pos: usize,
    },
    /// An integer outside the target type; offset carried.
    NumberOutOfRange {
        /// Byte offset where the number starts.
        pos: usize,
    },
    /// An object key that is not a string; offset carried.
    ExpectedKey {
        /// Byte offset where the key should start.
        pos: usize,
    },
    /// An object entry missing its colon; offset carried.
    MissingColon {
        /// Byte offset where the colon should sit.
        pos: usize,
    },
    /// A duplicate key in one object or `k=v` map; key carried.
    DuplicateKey {
        /// The repeated key.
        key: String,
    },
    /// An array or object missing its comma or closing bracket; offset
    /// carried.
    MissingComma {
        /// Byte offset where the comma or bracket should sit.
        pos: usize,
    },
    /// An array past its length bound.
    ArrayTooLong,
    /// A `k=v` field with no pairs at all.
    EmptyPairs,
    /// A `k=v` pair that is not `key=value`; part carried.
    BadPair {
        /// The rejected `;`-separated part.
        part: String,
    },
    /// A required `k=v` key is absent; key carried.
    MissingKey {
        /// The absent key.
        key: String,
    },
    /// A `k=v` map with the wrong key set; comma-joined want carried.
    WantKeys {
        /// The exact legal key set, comma-joined.
        want: String,
    },
    /// A value that is not an object.
    ExpectedObject,
    /// A value that is not an array.
    ExpectedArray,
    /// A required object field is absent; name carried.
    MissingField {
        /// The absent field name.
        name: String,
    },
    /// A present value with the wrong shape; field plus the wanted shape
    /// carried.
    WrongType {
        /// The offending field name.
        field: String,
        /// The wanted shape, e.g. `an integer`.
        want: &'static str,
    },
    /// A value outside its vocabulary with the legal values named; full
    /// decision text carried.
    Unknown {
        /// What was wrong, legal values named.
        what: String,
    },
    /// A value outside its legal range with the bounds named; full decision
    /// text carried.
    OutOfRange {
        /// What was wrong, bounds named.
        what: String,
    },
    /// A value with the wrong shape for its slot (id versus file stem,
    /// version, counts, order); full decision text carried.
    Mismatch {
        /// What was wrong, expected shape named.
        what: String,
    },
    /// A lower-level fault prefixed with its grammar section (`setup`,
    /// `assert`, ...); section plus source carried.
    Context {
        /// Grammar section the fault came from.
        ctx: String,
        /// The underlying fault.
        source: Box<ParseError>,
    },
}

impl ParseError {
    /// Prefix a lower-level fault with its grammar section, so `setup` and
    /// `assert` refusals name where they came from.
    pub fn context(ctx: impl Into<String>, source: ParseError) -> Self {
        Self::Context {
            ctx: ctx.into(),
            source: Box::new(source),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrailingBytes { pos } => write!(f, "trailing bytes at {pos}"),
            Self::NestingTooDeep { pos } => write!(f, "nesting too deep at {pos}"),
            Self::UnexpectedByte { pos, byte } => {
                write!(f, "unexpected byte {byte} at {pos}")
            }
            Self::EndOfInput { pos } => write!(f, "unexpected end of input at {pos}"),
            Self::BadLiteral { pos } => write!(f, "bad literal at {pos}"),
            Self::UnterminatedString { pos } => write!(f, "unterminated string at {pos}"),
            Self::UnterminatedEscape { pos } => write!(f, "unterminated escape at {pos}"),
            Self::BadEscape {
                pos,
                byte: Some(byte),
            } => {
                write!(f, "bad escape {byte} at {pos}")
            }
            Self::BadEscape { pos, byte: None } => write!(f, "bad escape at {pos}"),
            Self::ControlByte { pos } => write!(f, "control byte at {pos}"),
            Self::BadUtf8 { pos } => write!(f, "bad utf-8 at {pos}"),
            Self::ShortUnicode { pos } => write!(f, "short unicode escape at {pos}"),
            Self::BadHex { pos } => write!(f, "bad hex digit at {pos}"),
            Self::BadScalar { unit } => write!(f, "bad scalar {unit}"),
            Self::BadNumber { pos } => write!(f, "bad number at {pos}"),
            Self::BadFraction { pos } => write!(f, "bad fraction at {pos}"),
            Self::BadExponent { pos } => write!(f, "bad exponent at {pos}"),
            Self::BadToken { token } => write!(f, "bad number `{token}`"),
            Self::NonFinite { token } => write!(f, "non-finite number `{token}`"),
            Self::FloatRefused { pos } => write!(f, "floats refused at {pos}"),
            Self::NumberOutOfRange { pos } => write!(f, "number out of range at {pos}"),
            Self::ExpectedKey { pos } => write!(f, "object key must be a string at {pos}"),
            Self::MissingColon { pos } => write!(f, "missing `:` at {pos}"),
            Self::DuplicateKey { key } => write!(f, "duplicate key `{key}`"),
            Self::MissingComma { pos } => {
                write!(f, "missing `,` or closing bracket at {pos}")
            }
            Self::ArrayTooLong => write!(f, "array too long"),
            Self::EmptyPairs => write!(f, "empty field; want k=v;k=v pairs"),
            Self::BadPair { part } => write!(f, "pair `{part}` is malformed; want k=v"),
            Self::MissingKey { key } => write!(f, "missing key `{key}`"),
            Self::WantKeys { want } => write!(f, "want keys {want}"),
            Self::ExpectedObject => write!(f, "expected object"),
            Self::ExpectedArray => write!(f, "expected array"),
            Self::MissingField { name } => write!(f, "missing field `{name}`"),
            Self::WrongType { field, want } => write!(f, "field `{field}` must be {want}"),
            Self::Unknown { what } | Self::OutOfRange { what } | Self::Mismatch { what } => {
                write!(f, "{what}")
            }
            Self::Context { ctx, source } => write!(f, "{ctx}: {source}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Byte cursor shared by the fixture tokenizers: whitespace, peeking,
/// single-byte eats, literal words and the trailing-bytes check. The JSON
/// shapes on top stay per grammar; only this cursor is shared.
#[derive(Debug)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// Borrow `text` at offset zero.
    pub fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    /// Current byte offset, for error positions.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Input length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True when the cursor sits at the end of input.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Byte under the cursor, if any.
    pub fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    /// Byte `offset` ahead of the cursor, if any.
    pub fn ahead(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Consume one expected byte; false leaves the cursor untouched.
    pub fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Advance one byte, saturating at the end of input.
    pub fn bump(&mut self) {
        self.pos = (self.pos + 1).min(self.bytes.len());
    }

    /// Advance up to `count` bytes, saturating at the end of input.
    pub fn advance(&mut self, count: usize) {
        self.pos = self.pos.saturating_add(count).min(self.bytes.len());
    }

    /// Skip JSON whitespace.
    pub fn ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    /// Consume one literal word (`true`, `false`, `null`); anything else
    /// refuses with the cursor offset.
    pub fn lit(&mut self, word: &str) -> Result<(), ParseError> {
        let start = self.pos;
        if self.bytes.len() - self.pos >= word.len()
            && &self.bytes[self.pos..self.pos + word.len()] == word.as_bytes()
        {
            self.pos += word.len();
            Ok(())
        } else {
            Err(ParseError::BadLiteral { pos: start })
        }
    }

    /// Refuse when bytes remain after the top-level value.
    pub fn finish(&self) -> Result<(), ParseError> {
        if self.pos != self.bytes.len() {
            return Err(ParseError::TrailingBytes { pos: self.pos });
        }
        Ok(())
    }

    /// Raw bytes from `start` to the cursor, clamped so a stale start reads
    /// empty instead of panicking.
    pub fn lexeme(&self, start: usize) -> &'a [u8] {
        let start = start.min(self.pos);
        &self.bytes[start..self.pos]
    }

    /// Raw bytes from the cursor to the end of input, for scanners that
    /// decode one character at a time.
    pub fn rest(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }
}

/// Find one object field by name; absent fields refuse with the name carried.
pub fn find_entry<'v, V>(entries: &'v [(String, V)], name: &str) -> Result<&'v V, ParseError> {
    entries
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value)
        .ok_or_else(|| ParseError::MissingField {
            name: name.to_owned(),
        })
}

/// Split `k=v;k=v` pairs; empty fields, malformed pairs and duplicate keys
/// refuse. The `k=v` shape is shared; what each grammar reads out of the map
/// stays its own.
pub fn split_kv(text: &str) -> Result<Vec<(String, String)>, ParseError> {
    if text.is_empty() {
        return Err(ParseError::EmptyPairs);
    }
    let mut pairs = Vec::new();
    for part in text.split(';') {
        let (key, value) = part.split_once('=').ok_or_else(|| ParseError::BadPair {
            part: part.to_owned(),
        })?;
        if key.is_empty() || value.contains('=') {
            return Err(ParseError::BadPair {
                part: part.to_owned(),
            });
        }
        if pairs
            .iter()
            .any(|(known, _): &(String, String)| known == key)
        {
            return Err(ParseError::DuplicateKey {
                key: key.to_owned(),
            });
        }
        pairs.push((key.to_owned(), value.to_owned()));
    }
    Ok(pairs)
}

/// Read one `k=v` value by key; absent keys refuse with the key carried.
pub fn kv_get<'p>(pairs: &'p [(String, String)], key: &str) -> Result<&'p str, ParseError> {
    pairs
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| ParseError::MissingKey {
            key: key.to_owned(),
        })
}

/// Require the exact `k=v` key set, in any order; anything else refuses with
/// the comma-joined want carried.
pub fn kv_expect_keys(pairs: &[(String, String)], want: &[&str]) -> Result<(), ParseError> {
    let exact = pairs.len() == want.len()
        && want
            .iter()
            .all(|key| pairs.iter().any(|(got, _)| got == key));
    if exact {
        Ok(())
    } else {
        Err(ParseError::WantKeys {
            want: want.join(","),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_skips_ws_and_eats_one_byte() {
        let mut cursor = Cursor::new("  \t\n{x");
        cursor.ws();
        assert_eq!(cursor.pos(), 4);
        assert!(cursor.eat(b'{'));
        assert_eq!(cursor.peek(), Some(b'x'));
        assert!(!cursor.eat(b'}'));
        assert_eq!(cursor.pos(), 5);
    }

    #[test]
    fn lit_words_pass_and_misspellings_carry_pos() {
        let mut cursor = Cursor::new("true!");
        cursor.lit("true").expect("word parses");
        assert_eq!(cursor.pos(), 4);
        let mut bad = Cursor::new("truX");
        let error = bad.lit("true").expect_err("misspelling refuses");
        assert_eq!(error, ParseError::BadLiteral { pos: 0 });
        assert_eq!(error.to_string(), "bad literal at 0");
    }

    #[test]
    fn finish_names_trailing_bytes() {
        let mut cursor = Cursor::new("ab");
        cursor.bump();
        cursor.bump();
        cursor.finish().expect("exact input finishes");
        let mut over = Cursor::new("ab");
        over.bump();
        let error = over.finish().expect_err("trailing byte refuses");
        assert_eq!(error, ParseError::TrailingBytes { pos: 1 });
    }

    #[test]
    fn find_entry_names_missing_fields() {
        let entries = vec![("a".to_owned(), 1)];
        assert_eq!(find_entry(&entries, "a"), Ok(&1));
        let error = find_entry(&entries, "b").expect_err("absent field refuses");
        assert_eq!(
            error,
            ParseError::MissingField {
                name: "b".to_owned()
            }
        );
        assert_eq!(error.to_string(), "missing field `b`");
    }

    #[test]
    fn kv_splitter_refuses_empty_malformed_and_duplicate() {
        assert!(split_kv("").is_err(), "empty field refuses");
        assert!(split_kv("x").is_err(), "pair without `=` refuses");
        assert!(split_kv("=v").is_err(), "empty key refuses");
        assert!(split_kv("k=a=b").is_err(), "second `=` refuses");
        assert!(split_kv("k=1;k=2").is_err(), "duplicate key refuses");
        let pairs = split_kv("x=1;z=2").expect("pairs parse");
        assert_eq!(kv_get(&pairs, "z"), Ok("2"));
        assert!(kv_get(&pairs, "y").is_err(), "absent key refuses");
        kv_expect_keys(&pairs, &["x", "z"]).expect("exact set passes");
        assert!(
            kv_expect_keys(&pairs, &["x"]).is_err(),
            "wrong key set refuses"
        );
        let error = kv_expect_keys(&pairs, &["x"]).expect_err("want carried");
        assert_eq!(error.to_string(), "want keys x");
    }

    #[test]
    fn context_prefixes_name_their_section() {
        let error = ParseError::context("setup", ParseError::EndOfInput { pos: 3 });
        assert_eq!(error.to_string(), "setup: unexpected end of input at 3");
    }
}
