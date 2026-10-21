//! certainty: the kit table, and how sure we are of every piece of it.
//!
//! Every item in `content/rome/kit.toml` says how sure we are (`attested`,
//! `reconstructed`, `contested`, `invented`), names its source in `ref`, and, when an
//! object survives, says where it was found (`found`: site, year found, date of the
//! object). An open question about an item (the manuballista: hand-held or on a
//! stand) is a `reading` with its standing, so the codex card shows the debate
//! instead of hiding it.
//!
//! The loader refuses an item without a `certainty` or without a `ref`, and the
//! refusal names the item's key. The table is embedded at build time with
//! `include_str!`, so the tick never reads a file, two loads of one table hash the
//! same, and a draft table from another tool can never be loaded. Values are strings
//! and whole numbers only: no float, no clock, no IO.

use crate::table::{Entry, Table};
use crate::{only, LoadError};
use engine_core::Fnv64;

/// The suffix of a find section: `kit.gladius.found` is the find of `kit.gladius`.
const FOUND: &str = ".found";

/// How sure we are of a piece of kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    /// An object was found and its date is fixed.
    Attested,
    /// Built from texts and pictures.
    Reconstructed,
    /// Scholars disagree.
    Contested,
    /// Ours, with no source behind it.
    Invented,
}

impl Certainty {
    /// Every certainty, in the order the plan lists them.
    pub const ALL: [Self; 4] = [
        Self::Attested,
        Self::Reconstructed,
        Self::Contested,
        Self::Invented,
    ];

    /// The word this certainty is written as in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attested => "attested",
            Self::Reconstructed => "reconstructed",
            Self::Contested => "contested",
            Self::Invented => "invented",
        }
    }

    /// Read the word of a certainty, or `None` when it is not one of the four.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "attested" => Some(Self::Attested),
            "reconstructed" => Some(Self::Reconstructed),
            "contested" => Some(Self::Contested),
            "invented" => Some(Self::Invented),
            _ => None,
        }
    }
}

/// How the scholars stand on an open question about a piece of kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Scholars disagree; the card says so.
    Debated,
    /// The sources agree.
    Settled,
}

impl Standing {
    /// The word this standing is written as in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debated => "debated",
            Self::Settled => "settled",
        }
    }

    /// Read the word of a standing, or `None` when it is not one of the two.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "debated" => Some(Self::Debated),
            "settled" => Some(Self::Settled),
            _ => None,
        }
    }
}

/// Where a piece of kit was found, when an object of it survives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Find<'a> {
    /// The site the object was found at.
    pub site: &'a str,
    /// The year it was found, when the sources give one.
    pub year_found: Option<u32>,
    /// The date of the object itself, when the sources fix it.
    pub object_date: Option<&'a str>,
}

/// An open question about a piece of kit, and how the scholars stand on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading<'a> {
    /// The question, for example `hand-held or on a stand`.
    pub question: &'a str,
    /// How the scholars stand on it.
    pub standing: Standing,
}

/// One piece of kit and how sure we are of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item<'a> {
    /// The key of the item in the table, for example `kit.manuballista`.
    pub key: &'a str,
    /// How sure we are of this item.
    pub certainty: Certainty,
    /// Where the claim comes from, the item's `ref`.
    pub source: &'a str,
    /// The find, when an object of this item survives.
    pub found: Option<Find<'a>>,
    /// An open question about the item, when the sources have one.
    pub reading: Option<Reading<'a>>,
}

/// The kit table: every piece of kit the game gives a soldier, with its evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kit<'a> {
    items: Vec<Item<'a>>,
}

impl Kit<'static> {
    /// The kit table shipped in `content/rome/kit.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/kit.toml");

    /// Read the shipped kit table.
    ///
    /// # Errors
    ///
    /// Same as [`Kit::parse`]; a test keeps the shipped table valid, so this doubles as
    /// a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Kit<'a> {
    /// Read one kit table.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table, and the
    /// typed variants when an item has no `certainty` or no `ref`, a certainty is not
    /// one of the four words, a find has no site, or a reading has no standing.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let keys = section_keys(text);
        let mut items: Vec<Item<'a>> = Vec::new();
        for key in &keys {
            if key.ends_with(FOUND) {
                continue;
            }
            let entry = table
                .entry(key)
                .ok_or(LoadError::MissingEntry { entry: key })?;
            items.push(item(entry)?);
        }
        for key in &keys {
            let Some(item_key) = key.strip_suffix(FOUND) else {
                continue;
            };
            let entry = table
                .entry(key)
                .ok_or(LoadError::MissingEntry { entry: key })?;
            let found = found(entry)?;
            let item = items
                .iter_mut()
                .find(|item| item.key == item_key)
                .ok_or(LoadError::MissingEntry { entry: item_key })?;
            item.found = Some(found);
        }
        Ok(Self { items })
    }

    /// Every item of the kit, in table order.
    #[must_use]
    pub fn items(&self) -> &[Item<'a>] {
        &self.items
    }

    /// The item with this key, for example `kit.manuballista`.
    #[must_use]
    pub fn item(&self, key: &str) -> Option<&Item<'a>> {
        self.items.iter().find(|item| item.key == key)
    }

    /// A digest of the loaded kit: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/kit/1");
        for item in &self.items {
            write_text(&mut hasher, item.key);
            write_text(&mut hasher, item.certainty.as_str());
            write_text(&mut hasher, item.source);
            match item.found {
                Some(found) => {
                    hasher.write_u8(1);
                    write_text(&mut hasher, found.site);
                    hasher.write_u32(found.year_found.unwrap_or(0));
                    match found.object_date {
                        Some(date) => {
                            hasher.write_u8(1);
                            write_text(&mut hasher, date);
                        }
                        None => hasher.write_u8(0),
                    }
                }
                None => hasher.write_u8(0),
            }
            match item.reading {
                Some(reading) => {
                    hasher.write_u8(1);
                    write_text(&mut hasher, reading.question);
                    write_text(&mut hasher, reading.standing.as_str());
                }
                None => hasher.write_u8(0),
            }
        }
        hasher.finish()
    }
}

/// One item section: its certainty, its source and any open question.
fn item<'a>(entry: &Entry<'a>) -> Result<Item<'a>, LoadError<'a>> {
    let key = entry.key();
    only(entry, &["certainty", "reading", "reading_status"])?;
    let certainty = match text(entry, "certainty") {
        Some(word) => Certainty::parse(word).ok_or(LoadError::BadValue {
            entry: key,
            key: "certainty",
            value: word,
        })?,
        None => return Err(LoadError::MissingCertainty { entry: key }),
    };
    let source = text(entry, "ref").ok_or(LoadError::MissingRef { entry: key })?;
    Ok(Item {
        key,
        certainty,
        source,
        found: None,
        reading: reading(entry)?,
    })
}

/// One find section: the site, the year found and the date of the object.
fn found<'a>(entry: &Entry<'a>) -> Result<Find<'a>, LoadError<'a>> {
    let key = entry.key();
    only(entry, &["site", "year_found", "object_date"])?;
    let site = text(entry, "site").ok_or(LoadError::MissingKey {
        entry: key,
        key: "site",
    })?;
    let year_found = match entry.int("year_found") {
        Some(year) => {
            if year == 0 {
                return Err(LoadError::Zero {
                    entry: key,
                    key: "year_found",
                });
            }
            Some(u32::try_from(year).map_err(|_| LoadError::NumberOverflow {
                entry: key,
                key: "year_found",
            })?)
        }
        None => None,
    };
    if text(entry, "ref").is_none() {
        return Err(LoadError::MissingRef { entry: key });
    }
    Ok(Find {
        site,
        year_found,
        object_date: text(entry, "object_date"),
    })
}

/// The open question of an item, when the sources have one.
fn reading<'a>(entry: &Entry<'a>) -> Result<Option<Reading<'a>>, LoadError<'a>> {
    let key = entry.key();
    match (text(entry, "reading"), text(entry, "reading_status")) {
        (None, None) => Ok(None),
        (Some(question), Some(status)) => {
            let standing = Standing::parse(status).ok_or(LoadError::BadValue {
                entry: key,
                key: "reading_status",
                value: status,
            })?;
            Ok(Some(Reading { question, standing }))
        }
        (Some(_), None) => Err(LoadError::MissingKey {
            entry: key,
            key: "reading_status",
        }),
        (None, Some(_)) => Err(LoadError::MissingKey {
            entry: key,
            key: "reading",
        }),
    }
}

/// The trimmed text of a key, when it is there and not empty.
fn text<'a>(entry: &Entry<'a>, key: &str) -> Option<&'a str> {
    entry
        .text(key)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The section keys written in a table, in file order.
///
/// [`Table`] reads the values of a section but cannot list its sections, and the kit
/// table is data: every item it names is loaded. This reads the headers the way
/// [`Table::parse`] does, so a table that parses always lists its sections.
fn section_keys(text: &str) -> Vec<&str> {
    let mut keys = Vec::new();
    for raw in text.lines() {
        let body = raw.trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if let Some(inner) = body
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            keys.push(inner);
        }
    }
    keys
}

/// Mix one text into a digest, length first so two texts cannot swap halves.
fn write_text(hasher: &mut Fnv64, text: &str) {
    hasher.write_len(text.len());
    hasher.write_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANUBALLISTA: &str = "kit.manuballista";

    #[test]
    fn the_kit_table_loads() {
        let kit = Kit::shipped().expect("shipped kit table");
        assert_eq!(
            kit.items().len(),
            17,
            "the kit of the period and the rare kit"
        );
        for item in kit.items() {
            assert!(
                item.source.contains("rome.md"),
                "{} names no source: {}",
                item.key,
                item.source
            );
        }
    }

    #[test]
    fn every_item_says_how_sure_we_are() {
        let kit = Kit::shipped().expect("shipped kit table");
        for item in kit.items() {
            assert!(
                Certainty::ALL.contains(&item.certainty),
                "{} has no certainty of the four",
                item.key
            );
            assert!(!item.source.is_empty(), "{} has no source", item.key);
        }
    }

    #[test]
    fn the_manuballista_is_attested_and_found_in_1999() {
        let kit = Kit::shipped().expect("shipped kit table");
        let item = kit.item(MANUBALLISTA).expect("the manuballista");
        assert_eq!(item.certainty, Certainty::Attested);
        let found = item.found.expect("the Xanten-Wardt find");
        assert_eq!(found.site, "Xanten-Wardt");
        assert_eq!(found.year_found, Some(1999));
        assert_eq!(found.object_date, Some("mid 1st century AD"));
    }

    #[test]
    fn the_manuballista_reading_is_debated() {
        let kit = Kit::shipped().expect("shipped kit table");
        let item = kit.item(MANUBALLISTA).expect("the manuballista");
        let reading = item.reading.expect("the hand-held question");
        assert!(
            reading.question.contains("hand-held"),
            "{}",
            reading.question
        );
        assert_eq!(reading.standing, Standing::Debated);
    }

    #[test]
    fn an_item_without_a_certainty_is_refused_and_names_its_key() {
        let text = "[kit.lorica]\nref = \"rome.md section 3\"\n";
        let error = Kit::parse(text).expect_err("item without a certainty");
        assert_eq!(
            error,
            LoadError::MissingCertainty {
                entry: "kit.lorica"
            }
        );
        assert!(error.to_string().contains("kit.lorica"), "{error}");
    }

    #[test]
    fn an_item_without_a_ref_is_refused_and_names_its_key() {
        let text = "[kit.lorica]\ncertainty = \"attested\"\n";
        let error = Kit::parse(text).expect_err("item without a ref");
        assert_eq!(
            error,
            LoadError::MissingRef {
                entry: "kit.lorica"
            }
        );
        assert!(error.to_string().contains("kit.lorica"), "{error}");
    }

    #[test]
    fn a_certainty_outside_the_four_is_refused() {
        let text = "[kit.lorica]\ncertainty = \"maybe\"\nref = \"rome.md section 3\"\n";
        let error = Kit::parse(text).expect_err("certainty outside the four");
        assert!(error.to_string().contains("kit.lorica"), "{error}");
        assert!(error.to_string().contains("maybe"), "{error}");
    }

    #[test]
    fn a_draft_table_is_refused() {
        let text = "draft = true\ntable = \"camp\"\n\n[ditch.light]\nref = \"x\"\n";
        let error = Kit::parse(text).expect_err("a draft is not game data");
        assert!(matches!(error, LoadError::Table(_)), "{error}");
    }

    #[test]
    fn two_loads_hash_the_same() {
        let kit = Kit::shipped().expect("shipped kit table");
        let again = Kit::shipped().expect("shipped kit table");
        assert_eq!(kit.hash(), again.hash());
        let other =
            Kit::parse("[kit.lorica]\ncertainty = \"attested\"\nref = \"rome.md section 3\"\n")
                .expect("one item kit");
        assert_ne!(
            kit.hash(),
            other.hash(),
            "a different kit hashes differently"
        );
    }
}
