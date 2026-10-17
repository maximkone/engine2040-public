use super::*;

// ---- strict JSON subset ---------------------------------------------------

/// Strict JSON value: objects keep insertion order, numbers stay split so
/// integer fields never pass through a float.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

pub(super) struct Parser<'a> {
    pub(super) cursor: Cursor<'a>,
}

impl<'a> Parser<'a> {
    pub(super) fn parse(text: &'a str) -> Result<Json, ParseError> {
        let mut parser = Self {
            cursor: Cursor::new(text),
        };
        parser.cursor.ws();
        let value = parser.value(0)?;
        parser.cursor.ws();
        parser.cursor.finish()?;
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > 16 {
            return Err(ParseError::NestingTooDeep {
                pos: self.cursor.pos(),
            });
        }
        match self.cursor.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => {
                self.cursor.lit("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.cursor.lit("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.cursor.lit("null")?;
                Ok(Json::Null)
            }
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(byte) => Err(ParseError::UnexpectedByte {
                pos: self.cursor.pos(),
                byte,
            }),
            None => Err(ParseError::EndOfInput {
                pos: self.cursor.pos(),
            }),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.cursor.bump();
        let mut entries = Vec::new();
        self.cursor.ws();
        if self.cursor.eat(b'}') {
            return Ok(Json::Obj(entries));
        }
        loop {
            self.cursor.ws();
            if self.cursor.peek() != Some(b'"') {
                return Err(ParseError::ExpectedKey {
                    pos: self.cursor.pos(),
                });
            }
            let key = self.string()?;
            self.cursor.ws();
            if !self.cursor.eat(b':') {
                return Err(ParseError::MissingColon {
                    pos: self.cursor.pos(),
                });
            }
            self.cursor.ws();
            let value = self.value(depth + 1)?;
            if entries.iter().any(|(known, _)| *known == key) {
                return Err(ParseError::DuplicateKey { key });
            }
            entries.push((key, value));
            self.cursor.ws();
            if self.cursor.eat(b'}') {
                return Ok(Json::Obj(entries));
            }
            if !self.cursor.eat(b',') {
                return Err(ParseError::MissingComma {
                    pos: self.cursor.pos(),
                });
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.cursor.bump();
        let mut items = Vec::new();
        self.cursor.ws();
        if self.cursor.eat(b']') {
            return Ok(Json::Arr(items));
        }
        loop {
            self.cursor.ws();
            items.push(self.value(depth + 1)?);
            if items.len() > 8192 {
                return Err(ParseError::ArrayTooLong);
            }
            self.cursor.ws();
            if self.cursor.eat(b']') {
                return Ok(Json::Arr(items));
            }
            if !self.cursor.eat(b',') {
                return Err(ParseError::MissingComma {
                    pos: self.cursor.pos(),
                });
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.cursor.bump();
        let mut out = String::new();
        loop {
            let Some(byte) = self.cursor.peek() else {
                return Err(ParseError::UnterminatedString {
                    pos: self.cursor.pos(),
                });
            };
            match byte {
                b'"' => {
                    self.cursor.bump();
                    return Ok(out);
                }
                b'\\' => {
                    self.cursor.bump();
                    match self.cursor.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => out.push(self.unicode()?),
                        Some(other) => {
                            return Err(ParseError::BadEscape {
                                pos: self.cursor.pos(),
                                byte: Some(other),
                            });
                        }
                        None => {
                            return Err(ParseError::UnterminatedEscape {
                                pos: self.cursor.pos(),
                            });
                        }
                    }
                    if self.cursor.peek().is_some() {
                        self.cursor.bump();
                    }
                }
                0x00..=0x1F => {
                    return Err(ParseError::ControlByte {
                        pos: self.cursor.pos(),
                    });
                }
                _ => {
                    let rest = self.cursor.rest();
                    let text = core::str::from_utf8(rest).map_err(|_| ParseError::BadUtf8 {
                        pos: self.cursor.pos(),
                    })?;
                    let Some(next) = text.chars().next() else {
                        return Err(ParseError::BadUtf8 {
                            pos: self.cursor.pos(),
                        });
                    };
                    out.push(next);
                    self.cursor.advance(next.len_utf8());
                }
            }
        }
    }

    fn unicode(&mut self) -> Result<char, ParseError> {
        self.cursor.bump();
        if self.cursor.len() - self.cursor.pos() < 4 {
            return Err(ParseError::ShortUnicode {
                pos: self.cursor.pos(),
            });
        }
        let mut unit: u32 = 0;
        for offset in 0..4 {
            let Some(digit) = self.cursor.ahead(offset) else {
                return Err(ParseError::ShortUnicode {
                    pos: self.cursor.pos(),
                });
            };
            let nibble = match digit {
                b'0'..=b'9' => u32::from(digit - b'0'),
                b'a'..=b'f' => u32::from(digit - b'a') + 10,
                b'A'..=b'F' => u32::from(digit - b'A') + 10,
                _ => {
                    return Err(ParseError::BadHex {
                        pos: self.cursor.pos() + offset,
                    });
                }
            };
            unit = unit * 16 + nibble;
        }
        self.cursor.advance(3);
        char::from_u32(unit).ok_or(ParseError::BadScalar { unit })
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.cursor.pos();
        if self.cursor.peek() == Some(b'-') {
            self.cursor.bump();
        }
        let digits = self.cursor.pos();
        while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.cursor.bump();
        }
        if self.cursor.pos() == digits {
            return Err(ParseError::BadNumber { pos: start });
        }
        let mut is_float = false;
        if self.cursor.peek() == Some(b'.') {
            is_float = true;
            self.cursor.bump();
            let frac = self.cursor.pos();
            while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor.bump();
            }
            if self.cursor.pos() == frac {
                return Err(ParseError::BadFraction { pos: start });
            }
        }
        if matches!(self.cursor.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.cursor.bump();
            if matches!(self.cursor.peek(), Some(b'+' | b'-')) {
                self.cursor.bump();
            }
            let exp = self.cursor.pos();
            while self.cursor.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.cursor.bump();
            }
            if self.cursor.pos() == exp {
                return Err(ParseError::BadExponent { pos: start });
            }
        }
        let token = core::str::from_utf8(self.cursor.lexeme(start))
            .map_err(|_| ParseError::BadNumber { pos: start })?;
        if is_float {
            let value: f64 = token.parse().map_err(|_| ParseError::BadToken {
                token: token.to_owned(),
            })?;
            if !value.is_finite() {
                return Err(ParseError::NonFinite {
                    token: token.to_owned(),
                });
            }
            Ok(Json::Float(value))
        } else {
            let value: i64 = token.parse().map_err(|_| ParseError::BadToken {
                token: token.to_owned(),
            })?;
            Ok(Json::Int(value))
        }
    }
}

// ---- spec accessors -------------------------------------------------------

pub(super) fn as_obj(value: &Json) -> Result<&[(String, Json)], ParseError> {
    match value {
        Json::Obj(entries) => Ok(entries),
        _ => Err(ParseError::ExpectedObject),
    }
}

pub(super) fn optional<'j>(
    entries: &'j [(String, Json)],
    key: &str,
) -> Result<Option<&'j Json>, ParseError> {
    Ok(entries
        .iter()
        .find(|(known, _)| known == key)
        .map(|(_, value)| value))
}

pub(super) fn as_i64(value: &Json, key: &str) -> Result<i64, ParseError> {
    match value {
        Json::Int(number) => Ok(*number),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "an integer",
        }),
    }
}

pub(super) fn as_u32(value: &Json, key: &str, max: u32) -> Result<u32, ParseError> {
    let number = as_i64(value, key)?;
    if number < 0 || u64::try_from(number).map_or(true, |wide| wide > u64::from(max)) {
        return Err(ParseError::OutOfRange {
            what: format!("field `{key}` out of range 0..={max}"),
        });
    }
    let narrowed = u32::try_from(number).map_err(|_| ParseError::OutOfRange {
        what: format!("field `{key}` out of range 0..={max}"),
    })?;
    Ok(narrowed)
}

/// An integer or float field as `f64`; integers stay within `i32`.
pub(super) fn as_f64(value: &Json, key: &str) -> Result<f64, ParseError> {
    match value {
        Json::Int(number) => {
            i32::try_from(*number)
                .map(f64::from)
                .map_err(|_| ParseError::OutOfRange {
                    what: format!("field `{key}` out of range"),
                })
        }
        Json::Float(number) => Ok(*number),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "a number",
        }),
    }
}

pub(super) fn as_bool(value: &Json, key: &str) -> Result<bool, ParseError> {
    match value {
        Json::Bool(flag) => Ok(*flag),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "a boolean",
        }),
    }
}

pub(super) fn as_str<'j>(value: &'j Json, key: &str) -> Result<&'j str, ParseError> {
    match value {
        Json::Str(text) => Ok(text),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "a string",
        }),
    }
}

pub(super) fn as_arr<'j>(value: &'j Json, key: &str) -> Result<&'j [Json], ParseError> {
    match value {
        Json::Arr(items) => Ok(items),
        _ => Err(ParseError::WrongType {
            field: key.to_owned(),
            want: "an array",
        }),
    }
}
