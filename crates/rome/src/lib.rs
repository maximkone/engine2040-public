//! Rome: the game's shipped Roman practice tables, read into typed integer values.
//!
//! Every number comes from `docs/plan/rome.md` section 5 and carries a `ref` naming its
//! source; [`table`] refuses an entry that has a number but no `ref`. Lengths land in
//! millimetres (`u32`) with their source text, so code and cooked data agree on units and
//! on provenance, and times land in ticks once a table needs them.
//!
//! The kit table is the other half of the same rule: every piece of kit says how sure we
//! are of it ([`certainty`]), names its source, and says where the object was found when
//! one survives. An item without a certainty or without a source is refused by name.
//!
//! The registry is the rule for the numbers: every number the game shows lives in one
//! table with its tier, a note and its source ([`Registry`]). The loader refuses a
//! number with no tier, and the renderer asks the registry for a number by key, so a
//! number that is not registered cannot be drawn at all.
//!
//! The tables are embedded at build time with `include_str!` ([`Camp::shipped`],
//! [`Line::shipped`], [`Kit::shipped`]) or as a Rust const ([`Registry::shipped`]), so
//! the tick never reads a file, two loads of one table hash the same, and no clock, IO
//! or render edge exists. Values are integers only: no float ever enters a Rome rule.

pub mod camp;
pub mod certainty;
pub mod line;
pub mod table;

use core::fmt;
use engine_core::Fnv64;

pub use camp::{Camp, Ditch, Rampart, Streets, Threat};
pub use certainty::{
    Certainty, Claim, Find, Item, Kit, Number, Reading, Registry, Rendered, Standing,
};
pub use line::Line;
pub use table::{Entry, Table, TableError};

/// A length in whole millimetres, with the source text that gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measure<'a> {
    /// The length in millimetres.
    pub mm: u32,
    /// The `ref` of the table entry this number came from.
    pub source: &'a str,
}

/// Why a table could not be read into its typed form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError<'a> {
    /// The file is not a well-formed table, or an entry has a number and no `ref`.
    Table(TableError<'a>),
    /// A required section is missing.
    MissingEntry {
        /// The section key, for example `ditch.light`.
        entry: &'a str,
    },
    /// A required key is missing from a section that exists.
    MissingKey {
        /// The section key.
        entry: &'a str,
        /// The missing key.
        key: &'a str,
    },
    /// A section holds a key the typed form does not know.
    UnknownKey {
        /// The section key.
        entry: &'a str,
        /// The unknown key.
        key: &'a str,
    },
    /// A value that must be above zero is zero.
    Zero {
        /// The section key.
        entry: &'a str,
        /// The zero value's key.
        key: &'a str,
    },
    /// A length does not fit a `u32` of millimetres.
    LengthOverflow {
        /// The section key.
        entry: &'a str,
        /// The overflowing key.
        key: &'a str,
    },
    /// A kit item or a registered number has no `certainty`, so we cannot say how
    /// sure we are of it.
    MissingCertainty {
        /// The item key, for example `kit.manuballista`, or the number's key.
        entry: &'a str,
    },
    /// A kit item has no `ref` naming where the claim comes from.
    MissingRef {
        /// The item key, for example `kit.manuballista`.
        entry: &'a str,
    },
    /// A number was asked for that the registry does not hold, so it cannot be drawn.
    Unregistered {
        /// The key that was asked for, for example `weapon.pilum.length`.
        key: &'a str,
    },
    /// A value is not one the typed form knows.
    BadValue {
        /// The section key.
        entry: &'a str,
        /// The key the bad value sits under.
        key: &'a str,
        /// The value as written.
        value: &'a str,
    },
    /// A number does not fit the type the key holds.
    NumberOverflow {
        /// The section key.
        entry: &'a str,
        /// The overflowing key.
        key: &'a str,
    },
}

impl fmt::Display for LoadError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(error) => error.fmt(f),
            Self::MissingEntry { entry } => write!(f, "table has no `[{entry}]` entry"),
            Self::MissingKey { entry, key } => write!(f, "`[{entry}]` has no `{key}`"),
            Self::UnknownKey { entry, key } => write!(f, "`[{entry}]` has an unknown key `{key}`"),
            Self::Zero { entry, key } => write!(f, "`[{entry}]` {key} must be above zero"),
            Self::LengthOverflow { entry, key } => {
                write!(f, "`[{entry}]` {key} does not fit a millimetre length")
            }
            Self::MissingCertainty { entry } => {
                write!(f, "`[{entry}]` has no `certainty` saying how sure we are")
            }
            Self::MissingRef { entry } => {
                write!(f, "`[{entry}]` has no `ref` naming its source")
            }
            Self::Unregistered { key } => {
                write!(f, "`{key}` is not registered, so it cannot be drawn")
            }
            Self::BadValue { entry, key, value } => {
                write!(
                    f,
                    "`[{entry}]` {key} is `{value}`, which this crate does not read"
                )
            }
            Self::NumberOverflow { entry, key } => {
                write!(f, "`[{entry}]` {key} does not fit its number")
            }
        }
    }
}

impl std::error::Error for LoadError<'_> {}

impl<'a> From<TableError<'a>> for LoadError<'a> {
    fn from(error: TableError<'a>) -> Self {
        Self::Table(error)
    }
}

/// Refuse a key the typed form does not read; `ref` is always allowed.
pub(crate) fn only<'a>(entry: &Entry<'a>, allowed: &[&str]) -> Result<(), LoadError<'a>> {
    for key in entry.keys() {
        if key == "ref" || allowed.contains(&key) {
            continue;
        }
        return Err(LoadError::UnknownKey {
            entry: entry.key(),
            key,
        });
    }
    Ok(())
}

/// One required integer from a section.
pub(crate) fn int<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u64, LoadError<'a>> {
    entry.int(key).ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key,
    })
}

/// One required length in Roman feet, converted to millimetres.
pub(crate) fn length<'a>(
    entry: &Entry<'a>,
    key: &'a str,
    mm_per_ft: u32,
) -> Result<Measure<'a>, LoadError<'a>> {
    let feet = int(entry, key)?;
    let mm = feet
        .checked_mul(u64::from(mm_per_ft))
        .and_then(|mm| u32::try_from(mm).ok())
        .ok_or_else(|| LoadError::LengthOverflow {
            entry: entry.key(),
            key,
        })?;
    Ok(Measure {
        mm,
        source: entry.source(),
    })
}

/// The table's own millimetres per Roman foot, which every length converts through.
pub(crate) fn mm_per_foot<'a>(table: &Table<'a>) -> Result<u32, LoadError<'a>> {
    let units = table
        .entry("units")
        .ok_or(LoadError::MissingEntry { entry: "units" })?;
    only(units, &["mm_per_ft"])?;
    let mm = int(units, "mm_per_ft")?;
    let mm = u32::try_from(mm).map_err(|_| LoadError::LengthOverflow {
        entry: "units",
        key: "mm_per_ft",
    })?;
    if mm == 0 {
        return Err(LoadError::Zero {
            entry: "units",
            key: "mm_per_ft",
        });
    }
    Ok(mm)
}

/// A digest of loaded millimetre values under a version tag.
pub(crate) fn digest<'a>(tag: &[u8], lengths: impl Iterator<Item = Measure<'a>>) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(tag);
    for length in lengths {
        hasher.write_u32(length.mm);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VEGETIUS_38: &str = "rome.md section 5, Vegetius 3.8; Hyginus 49";

    #[test]
    fn shipped_camp_reads_a_one_night_ditch() {
        let camp = Camp::shipped().expect("shipped camp table");
        let ditch = camp.ditch(Threat::OneNight);
        assert_eq!(ditch.width.mm, 1480, "5 ft at 296 mm");
        assert_eq!(ditch.depth.mm, 888, "3 ft at 296 mm");
        assert!(
            ditch.width.source.contains(VEGETIUS_38),
            "{}",
            ditch.width.source
        );
    }

    #[test]
    fn every_threat_has_its_own_ditch() {
        let camp = Camp::shipped().expect("shipped camp table");
        let light = camp.ditch(Threat::Light);
        let serious = camp.ditch(Threat::Serious);
        assert_eq!((light.width.mm, light.depth.mm), (2664, 2072), "9 x 7 ft");
        assert_eq!(
            (serious.width.mm, serious.depth.mm),
            (3552, 2664),
            "12 x 9 ft"
        );
        assert_eq!(camp.ditch(Threat::OneNight).width.mm, 1480, "5 x 3 ft");
    }

    #[test]
    fn rampart_and_streets_read_the_camp_plan() {
        let camp = Camp::shipped().expect("shipped camp table");
        assert_eq!(
            (camp.rampart.width.mm, camp.rampart.height.mm),
            (2368, 1776),
            "8 x 6 ft"
        );
        assert_eq!(camp.streets.via_principalis.mm, 17760, "60 ft");
        assert_eq!(camp.streets.via_quintana.mm, 11840, "40 ft");
        assert!(camp
            .streets
            .via_principalis
            .source
            .contains("Hyginus 14, 17"));
    }

    #[test]
    fn line_reads_frontage_and_rank_gap() {
        let line = Line::shipped().expect("shipped line table");
        assert_eq!(line.frontage.mm, 888, "3 ft");
        assert_eq!(line.rank_gap.mm, 1776, "6 ft");
        assert!(line.rank_gap.source.contains("Vegetius 3.14"));
    }

    #[test]
    fn an_entry_with_numbers_and_no_ref_is_refused() {
        let text = "[units]\nmm_per_ft = 296\nref = \"rome.md section 5 units\"\n\n[ditch.light]\nwidth_ft = 9\ndepth_ft = 7\n";
        let error = Table::parse(text).expect_err("entry without ref");
        assert_eq!(
            error,
            TableError::MissingRef {
                line: 5,
                entry: "ditch.light"
            }
        );
        assert!(error.to_string().contains("ditch.light"), "{error}");
        assert!(
            matches!(Camp::parse(text), Err(LoadError::Table(_))),
            "camp load refuses it too"
        );
    }

    #[test]
    fn a_float_is_refused_at_its_line() {
        let text = "[units]\nmm_per_ft = 296.5\nref = \"rome.md section 5 units\"\n";
        let error = Table::parse(text).expect_err("float");
        assert_eq!(
            error,
            TableError::NotAnInteger {
                line: 2,
                text: "mm_per_ft = 296.5"
            }
        );
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = Camp::SHIPPED.replace("depth_ft = 3", "dept_ft = 3");
        let error = Camp::parse(&text).expect_err("typo in a key");
        assert!(error.to_string().contains("dept_ft"), "{error}");
    }

    #[test]
    fn lengths_convert_through_the_tables_own_foot() {
        let text = Camp::SHIPPED.replace("mm_per_ft = 296", "mm_per_ft = 100");
        let camp = Camp::parse(&text).expect("camp table");
        assert_eq!(camp.ditch(Threat::OneNight).width.mm, 500);
    }

    #[test]
    fn two_loads_hash_the_same() {
        let camp = Camp::shipped().expect("camp table");
        let again = Camp::shipped().expect("camp table");
        assert_eq!(camp.hash(), again.hash());
        let line = Line::shipped().expect("line table");
        assert_eq!(line.hash(), Line::shipped().expect("line table").hash());
        assert_ne!(
            camp.hash(),
            line.hash(),
            "the two tables hash under different tags"
        );
    }
}
