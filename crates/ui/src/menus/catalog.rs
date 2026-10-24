//! catalog: the locale table as data an author can edit.
//!
//! Owner: `engine-ui`. Consumer: the static table in [`crate::menus::locale`] is the
//! sink this migrates to; the last test below builds a catalog out of that
//! table and holds both answers equal for every key and both locales, so a
//! migrated menu reads exactly the labels it reads today. Units: keys are ASCII
//! `a-z`, `0-9` and `-`; text is UTF-8. Determinism: parse, merge and resolve
//! are pure, rows sort by key, no IO and no clock. Failure: a refused table is a
//! typed [`CatalogError`] naming the row or the rule; [`Catalog::resolve`] never
//! panics and falls back to English, then to the key itself, the contract
//! [`crate::menus::locale::lookup`] keeps. Budget: one row per key, a map lookup per
//! answer.

use std::collections::BTreeMap;
use std::fmt;

use super::locale::LocaleId;

/// Most rows one catalog may hold; a table past this is a shipped data file,
/// not a menu.
pub const MAX_ROWS: usize = 4_096;
/// Longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 64;

/// One locale's text table: a key and, for each locale that has it, its text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    rows: BTreeMap<String, [Option<String>; 2]>,
}

/// Why a catalog text was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// A row is not `key = "text"`.
    BadRow {
        /// 1-based line of the row.
        line: usize,
        /// What the row should have been.
        reason: &'static str,
    },
    /// A key is empty, too long or carries a character outside `a-z 0-9 -`.
    BadKey {
        /// 1-based line of the row.
        line: usize,
        /// The key as written.
        key: String,
    },
    /// The same key appears twice for one locale.
    Duplicate {
        /// The key listed twice.
        key: String,
        /// 1-based line of the second row; 0 when it came from another table.
        line: usize,
    },
    /// More rows than [`MAX_ROWS`].
    TooManyRows {
        /// How many rows the tables hold.
        count: usize,
    },
    /// A translation exists but English does not, so it can never be read.
    MissingEnglish {
        /// The key with no English text.
        key: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRow { line, reason } => write!(f, "line {line}: {reason}"),
            Self::BadKey { line, key } => write!(
                f,
                "line {line}: `{key}` is not a key; use 1..={MAX_KEY_BYTES} of a-z, 0-9 and -"
            ),
            Self::Duplicate { key, line } => {
                if *line == 0 {
                    write!(f, "`{key}` is already in the table")
                } else {
                    write!(f, "line {line}: `{key}` is listed twice")
                }
            }
            Self::TooManyRows { count } => {
                write!(f, "{count} rows; at most {MAX_ROWS}")
            }
            Self::MissingEnglish { key } => write!(
                f,
                "`{key}` has a translation but no English text; write the English row too"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

/// The slot one locale's text sits in.
fn slot(locale: LocaleId) -> usize {
    match locale {
        LocaleId::En => 0,
        LocaleId::De => 1,
    }
}

/// A key: 1..=[`MAX_KEY_BYTES`] of `a-z`, `0-9` and `-`.
fn check_key(line: usize, key: &str) -> Result<(), CatalogError> {
    let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if key.is_empty() || key.len() > MAX_KEY_BYTES || !key.chars().all(plain) {
        return Err(CatalogError::BadKey {
            line,
            key: key.to_owned(),
        });
    }
    Ok(())
}

/// The quoted text after `=`, with `\"` and `\\` as the only escapes; anything
/// after the closing quote must be a `#` comment.
fn text_after(line: usize, rest: &str) -> Result<String, CatalogError> {
    let body = rest.trim_start();
    let Some(after_quote) = body.strip_prefix('"') else {
        return Err(CatalogError::BadRow {
            line,
            reason: "text is one \"quoted string\"",
        });
    };
    let mut text = String::new();
    let mut chars = after_quote.char_indices();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '"' => {
                let tail = after_quote[at + 1..].trim_start();
                if !tail.is_empty() && !tail.starts_with('#') {
                    return Err(CatalogError::BadRow {
                        line,
                        reason: "unexpected text after the closing quote",
                    });
                }
                return Ok(text);
            }
            '\\' => match chars.next() {
                Some((_, '"')) => text.push('"'),
                Some((_, '\\')) => text.push('\\'),
                _ => {
                    return Err(CatalogError::BadRow {
                        line,
                        reason: "only the escapes \\\" and \\\\ exist in text",
                    });
                }
            },
            _ => text.push(ch),
        }
    }
    Err(CatalogError::BadRow {
        line,
        reason: "text is missing its closing quote",
    })
}

impl Catalog {
    /// Parse one locale's table: `key = "text"` rows, `#` starting a comment.
    pub fn parse(locale: LocaleId, text: &str) -> Result<Self, CatalogError> {
        let mut rows: BTreeMap<String, [Option<String>; 2]> = BTreeMap::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.trim();
            if body.is_empty() || body.starts_with('#') {
                continue;
            }
            let Some((key_text, rest)) = body.split_once('=') else {
                return Err(CatalogError::BadRow {
                    line,
                    reason: "expected `key = \"text\"`",
                });
            };
            let key = key_text.trim();
            check_key(line, key)?;
            let value = text_after(line, rest)?;
            let row = rows.entry(key.to_owned()).or_default();
            let cell = &mut row[slot(locale)];
            if cell.is_some() {
                return Err(CatalogError::Duplicate {
                    key: key.to_owned(),
                    line,
                });
            }
            *cell = Some(value);
        }
        if rows.len() > MAX_ROWS {
            return Err(CatalogError::TooManyRows { count: rows.len() });
        }
        Ok(Self { rows })
    }

    /// Fold another table in. A key a locale already carries is refused, so two
    /// files can never disagree about one label.
    pub fn merge(&mut self, other: &Self) -> Result<(), CatalogError> {
        if self.rows.len().saturating_add(other.rows.len()) > MAX_ROWS {
            return Err(CatalogError::TooManyRows {
                count: self.rows.len().saturating_add(other.rows.len()),
            });
        }
        for (key, cells) in &other.rows {
            let row = self.rows.entry(key.clone()).or_default();
            for (cell, incoming) in row.iter_mut().zip(cells) {
                if let Some(text) = incoming {
                    if cell.is_some() {
                        return Err(CatalogError::Duplicate {
                            key: key.clone(),
                            line: 0,
                        });
                    }
                    *cell = Some(text.clone());
                }
            }
        }
        Ok(())
    }

    /// Check the one rule a shipped table must keep: every key has English
    /// text, because every other locale falls back to it. Reports the first
    /// missing key in sorted order.
    pub fn check(&self) -> Result<(), CatalogError> {
        for (key, cells) in &self.rows {
            if cells[slot(LocaleId::En)].is_none() {
                return Err(CatalogError::MissingEnglish { key: key.clone() });
            }
        }
        Ok(())
    }

    /// The text for one key: the locale's own, else English, else the key
    /// itself, which is what [`crate::menus::locale::lookup`] answers.
    #[must_use]
    pub fn resolve<'a>(&'a self, key: &'a str, locale: LocaleId) -> &'a str {
        let Some(cells) = self.rows.get(key) else {
            return key;
        };
        cells[slot(locale)]
            .as_deref()
            .or_else(|| cells[slot(LocaleId::En)].as_deref())
            .unwrap_or(key)
    }

    /// Rows in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the table names no key.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Keys in sorted order.
    pub fn keys(&self) -> impl Iterator<Item = &str> + '_ {
        self.rows.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::locale;

    const EN: &str = "\
# english
start-fight = \"Start Fight\"
quit = \"Quit\"   # the one label both screens share
";
    const DE: &str = "start-fight = \"Kampf starten\"\n";

    fn both() -> Catalog {
        let mut catalog = Catalog::parse(LocaleId::En, EN).expect("english parses");
        let german = Catalog::parse(LocaleId::De, DE).expect("german parses");
        catalog.merge(&german).expect("the locales merge");
        catalog
    }

    #[test]
    fn parse_and_resolve_falls_back_to_english_then_to_the_key() {
        let catalog = both();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog.resolve("start-fight", LocaleId::En), "Start Fight");
        assert_eq!(
            catalog.resolve("start-fight", LocaleId::De),
            "Kampf starten",
            "a translation wins over English"
        );
        assert_eq!(
            catalog.resolve("quit", LocaleId::De),
            "Quit",
            "a key with no German text reads English"
        );
        assert_eq!(
            catalog.resolve("no-such-key", LocaleId::De),
            "no-such-key",
            "an unknown key reads itself"
        );
        assert_eq!(catalog.check(), Ok(()));
        assert_eq!(
            catalog.keys().collect::<Vec<&str>>(),
            ["quit", "start-fight"]
        );
        // Escapes are read, not echoed.
        let escaped = Catalog::parse(LocaleId::En, "quote = \"He said \\\"hi\\\" \\\\ done\"\n")
            .expect("parses");
        assert_eq!(
            escaped.resolve("quote", LocaleId::En),
            "He said \"hi\" \\ done"
        );
        // A table is data: the same text builds the same table twice.
        assert_eq!(
            Catalog::parse(LocaleId::En, EN).expect("parses"),
            Catalog::parse(LocaleId::En, EN).expect("parses")
        );
    }

    #[test]
    fn check_refuses_a_translation_with_no_english_side() {
        let german = Catalog::parse(LocaleId::De, DE).expect("parses");
        assert_eq!(
            german.check(),
            Err(CatalogError::MissingEnglish {
                key: "start-fight".to_owned(),
            })
        );
        let mut catalog = both();
        let extra = Catalog::parse(LocaleId::De, "resume = \"Fortsetzen\"\n").expect("parses");
        catalog.merge(&extra).expect("merges");
        assert_eq!(
            catalog.check(),
            Err(CatalogError::MissingEnglish {
                key: "resume".to_owned(),
            }),
            "a German-only row is refused by its key"
        );
    }

    #[test]
    fn refusals_name_the_row_and_the_rule() {
        let cases: [(&str, CatalogError); 5] = [
            (
                "start-fight \"Start Fight\"\n",
                CatalogError::BadRow {
                    line: 1,
                    reason: "expected `key = \"text\"`",
                },
            ),
            (
                "start-fight = \"Start Fight\n",
                CatalogError::BadRow {
                    line: 1,
                    reason: "text is missing its closing quote",
                },
            ),
            (
                "start-fight = \"Start Fight\" extra\n",
                CatalogError::BadRow {
                    line: 1,
                    reason: "unexpected text after the closing quote",
                },
            ),
            (
                "Start Fight = \"Start Fight\"\n",
                CatalogError::BadKey {
                    line: 1,
                    key: "Start Fight".to_owned(),
                },
            ),
            (
                "quit = \"Quit\"\nquit = \"Leave\"\n",
                CatalogError::Duplicate {
                    key: "quit".to_owned(),
                    line: 2,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(Catalog::parse(LocaleId::En, text), Err(expected), "{text}");
        }
        // Two files that both name one label refuse the second one, with no
        // line to point at.
        let mut catalog = Catalog::parse(LocaleId::En, "quit = \"Quit\"\n").expect("parses");
        let again = Catalog::parse(LocaleId::En, "quit = \"Leave\"\n").expect("parses");
        assert_eq!(
            catalog.merge(&again),
            Err(CatalogError::Duplicate {
                key: "quit".to_owned(),
                line: 0,
            })
        );
    }

    #[test]
    fn the_static_table_migrates_without_a_label_changing() {
        let keys = [
            locale::KEY_START_FIGHT,
            locale::KEY_SETTINGS,
            locale::KEY_QUIT,
            locale::KEY_RESUME,
            locale::KEY_QUIT_TO_MAIN,
            locale::KEY_BACK,
            locale::KEY_VOLUME_MASTER,
            locale::KEY_VOLUME_MUSIC,
            locale::KEY_VOLUME_EFFECTS,
            locale::KEY_QUALITY,
            locale::KEY_RENDER_SCALE,
            locale::KEY_VSYNC,
        ];
        let mut catalog = Catalog::default();
        for locale_id in [LocaleId::En, LocaleId::De] {
            for key in keys {
                let file = format!("{key} = \"{}\"\n", locale::lookup(key, locale_id));
                let table = Catalog::parse(locale_id, &file).expect("the static row parses");
                catalog.merge(&table).expect("the static rows merge");
            }
        }
        assert_eq!(catalog.len(), keys.len());
        assert_eq!(catalog.check(), Ok(()));
        for key in keys {
            for locale_id in [LocaleId::En, LocaleId::De] {
                assert_eq!(
                    catalog.resolve(key, locale_id),
                    locale::lookup(key, locale_id),
                    "{key} in {locale_id:?}"
                );
            }
        }
        assert_eq!(
            catalog.resolve("no-such-key", LocaleId::De),
            locale::lookup("no-such-key", LocaleId::De)
        );
    }
}
