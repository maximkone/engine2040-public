//! table: a strict reader for the small TOML subset the Rome tables use.
//!
//! A table is a list of sections (`[units]`, `[ditch.one_night]`) holding `key = 12` and
//! `key = "text"` lines; `#` starts a comment and blank lines are ignored. Nothing else
//! is accepted: no floats, signs, arrays, inline tables, escapes, repeated key or
//! repeated section, and a refused file names the line that refused it.
//!
//! One rule binds every Rome table: a section that carries a number must also carry a
//! non-empty `ref` string naming its source. No number enters the game unsourced.

use core::fmt;

/// One value written in a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value<'a> {
    /// A non-negative integer exactly as written.
    Int(u64),
    /// A quoted string without its quotes.
    Text(&'a str),
}

/// One section of a table: its key path, the line it starts on, and its values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    key: &'a str,
    line: usize,
    values: Vec<(&'a str, Value<'a>)>,
}

impl<'a> Entry<'a> {
    /// The section key path as written, for example `ditch.one_night`.
    #[must_use]
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// The 1-based line the section header sits on.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// The integer written under `key`, when the section holds one.
    #[must_use]
    pub fn int(&self, key: &str) -> Option<u64> {
        self.values.iter().find_map(|(name, value)| match value {
            Value::Int(number) if *name == key => Some(*number),
            _ => None,
        })
    }

    /// The string written under `key`, without its quotes.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<&'a str> {
        self.values.iter().find_map(|(name, value)| match value {
            Value::Text(text) if *name == key => Some(*text),
            _ => None,
        })
    }

    /// The `ref` text of this section: where its numbers come from.
    ///
    /// Empty only for a section that carries no numbers, which needs no source.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.text("ref").unwrap_or("")
    }

    /// Every key written in this section, in file order.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.values.iter().map(|(name, _)| *name)
    }

    fn has_ref(&self) -> bool {
        self.text("ref").is_some_and(|text| !text.trim().is_empty())
    }
}

/// A whole table: its sections in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table<'a> {
    entries: Vec<Entry<'a>>,
}

impl<'a> Table<'a> {
    /// Read a table.
    ///
    /// # Errors
    ///
    /// Returns the first [`TableError`] the text earns: a malformed line, a repeated key
    /// or section, or an entry with a number and no `ref`.
    pub fn parse(text: &'a str) -> Result<Self, TableError<'a>> {
        let mut entries: Vec<Entry<'a>> = Vec::new();
        let mut open = false;
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.trim();
            if body.is_empty() || body.starts_with('#') {
                continue;
            }
            if let Some(key) = section_header(body, line)? {
                if entries.iter().any(|entry| entry.key == key) {
                    return Err(TableError::DuplicateSection { line, key });
                }
                entries.push(Entry {
                    key,
                    line,
                    values: Vec::new(),
                });
                open = true;
                continue;
            }
            if !open {
                return Err(TableError::MissingSection { line, text: body });
            }
            let (key, value) = assignment(body, line)?;
            let Some(entry) = entries.last_mut() else {
                return Err(TableError::MissingSection { line, text: body });
            };
            if entry.values.iter().any(|(name, _)| *name == key) {
                return Err(TableError::DuplicateKey {
                    line,
                    entry: entry.key,
                    key,
                });
            }
            entry.values.push((key, value));
        }
        check_refs(&entries)?;
        Ok(Self { entries })
    }

    /// The section written under `key`, for example `ditch.light`.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&Entry<'a>> {
        self.entries.iter().find(|entry| entry.key == key)
    }
}

/// Why a table file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableError<'a> {
    /// A line is not a section header, a `key = value` pair, a comment or blank.
    Syntax {
        /// 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A `key = value` pair sits before any section header.
    MissingSection {
        /// 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A section header is not `[a.b]` with lowercase identifier segments.
    BadSection {
        /// 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// A section key appears twice.
    DuplicateSection {
        /// 1-based line number.
        line: usize,
        /// The section key.
        key: &'a str,
    },
    /// A key appears twice inside one section.
    DuplicateKey {
        /// 1-based line number.
        line: usize,
        /// The section key.
        entry: &'a str,
        /// The repeated key.
        key: &'a str,
    },
    /// A value is neither a whole number nor a quoted string.
    NotAnInteger {
        /// 1-based line number.
        line: usize,
        /// The line as written, trimmed.
        text: &'a str,
    },
    /// An entry carries a number but no `ref` naming its source.
    MissingRef {
        /// 1-based line number of the section header.
        line: usize,
        /// The section key, so the refusal names the entry.
        entry: &'a str,
    },
}

impl fmt::Display for TableError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, text } => {
                write!(f, "line {line}: not a `key = value` line: {text}")
            }
            Self::MissingSection { line, text } => {
                write!(
                    f,
                    "line {line}: `key = value` before any `[section]`: {text}"
                )
            }
            Self::BadSection { line, text } => {
                write!(f, "line {line}: not a `[a.b]` section header: {text}")
            }
            Self::DuplicateSection { line, key } => {
                write!(f, "line {line}: section `[{key}]` appears twice")
            }
            Self::DuplicateKey { line, entry, key } => {
                write!(f, "line {line}: `[{entry}]` holds `{key}` twice")
            }
            Self::NotAnInteger { line, text } => {
                write!(
                    f,
                    "line {line}: not a whole number or a quoted string: {text}"
                )
            }
            Self::MissingRef { line, entry } => {
                write!(
                    f,
                    "line {line}: `[{entry}]` has numbers but no `ref` naming its source"
                )
            }
        }
    }
}

impl std::error::Error for TableError<'_> {}

/// The section key of a `[a.b]` header, or `None` when this line is not a header.
fn section_header<'a>(body: &'a str, line: usize) -> Result<Option<&'a str>, TableError<'a>> {
    let Some(rest) = body.strip_prefix('[') else {
        return Ok(None);
    };
    let inner = rest
        .strip_suffix(']')
        .ok_or(TableError::BadSection { line, text: body })?;
    let valid =
        !inner.is_empty() && !inner.contains(['[', ']']) && inner.split('.').all(is_identifier);
    if !valid {
        return Err(TableError::BadSection { line, text: body });
    }
    Ok(Some(inner))
}

/// One `key = value` line, split into its key and its typed value.
fn assignment<'a>(body: &'a str, line: usize) -> Result<(&'a str, Value<'a>), TableError<'a>> {
    let (key, value) = body
        .split_once('=')
        .ok_or(TableError::Syntax { line, text: body })?;
    let key = key.trim();
    if !is_identifier(key) {
        return Err(TableError::Syntax { line, text: body });
    }
    Ok((key, value_of(value.trim(), line, body)?))
}

/// A value: a bare whole number, or a double-quoted string with no escapes.
fn value_of<'a>(
    text: &'a str,
    line: usize,
    line_text: &'a str,
) -> Result<Value<'a>, TableError<'a>> {
    if let Some(rest) = text.strip_prefix('"') {
        let inner = rest.strip_suffix('"').ok_or(TableError::Syntax {
            line,
            text: line_text,
        })?;
        if inner.contains('"') || inner.contains('\\') {
            return Err(TableError::Syntax {
                line,
                text: line_text,
            });
        }
        return Ok(Value::Text(inner));
    }
    let number = parse_integer(text).ok_or(TableError::NotAnInteger {
        line,
        text: line_text,
    })?;
    Ok(Value::Int(number))
}

/// A non-negative decimal integer with no sign, separator or decimal point.
fn parse_integer(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A lowercase identifier: a letter or `_`, then letters, digits and `_`.
fn is_identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Refuse any section that carries a number and no `ref` text.
fn check_refs<'a>(entries: &[Entry<'a>]) -> Result<(), TableError<'a>> {
    for entry in entries {
        let numbered = entry
            .values
            .iter()
            .any(|(_, value)| matches!(value, Value::Int(_)));
        if numbered && !entry.has_ref() {
            return Err(TableError::MissingRef {
                line: entry.line,
                entry: entry.key,
            });
        }
    }
    Ok(())
}
