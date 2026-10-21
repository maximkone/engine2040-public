//! timber: felling is season-gated, and one log becomes stakes or pila shafts.
//!
//! Vitruvius 2.9 gives the Roman rule: a tree is cut to the heart, the sap
//! drains, and only then is it felled, and that is done from early autumn to
//! spring, when the sap is down. Outside that season the felling is refused
//! with its reason. The table says which wood is cut: oak for ground work,
//! alder for marsh piles.
//!
//! The numbers live in `content/rome/tech/timber.toml`. Each of them names its
//! source in `ref`, and each section says how sure we are of it in
//! `certainty`: the season is attested, the woods are reconstructed, and the
//! counts of stakes and shafts are ours, because the sources give none.
//!
//! The table is embedded at build time with `include_str!`, so the tick never
//! reads a file, two loads of one table hash the same, and a felling replays
//! equal. Values are integers only: no float, no clock, no IO.

use std::fmt;

use crate::certainty::Certainty;
use crate::table::{Entry, Table};
use crate::{int, only, LoadError};
use engine_core::Fnv64;

/// How many woods the table holds: oak and alder.
const WOODS: usize = 2;

/// How many products a log becomes: stakes and pila shafts.
const PRODUCTS: usize = 2;

/// A month of the year, in the civil order from January.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Month {
    /// The first month of the civil year.
    January,
    /// The second month.
    February,
    /// The third month.
    March,
    /// The fourth month.
    April,
    /// The fifth month.
    May,
    /// The sixth month.
    June,
    /// The seventh month.
    July,
    /// The eighth month.
    August,
    /// The ninth month, the start of the felling season.
    September,
    /// The tenth month.
    October,
    /// The eleventh month.
    November,
    /// The twelfth month.
    December,
}

impl Month {
    /// Every month of the year, in year order.
    pub const ALL: [Self; 12] = [
        Self::January,
        Self::February,
        Self::March,
        Self::April,
        Self::May,
        Self::June,
        Self::July,
        Self::August,
        Self::September,
        Self::October,
        Self::November,
        Self::December,
    ];

    /// The name of the month, as the table writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::January => "January",
            Self::February => "February",
            Self::March => "March",
            Self::April => "April",
            Self::May => "May",
            Self::June => "June",
            Self::July => "July",
            Self::August => "August",
            Self::September => "September",
            Self::October => "October",
            Self::November => "November",
            Self::December => "December",
        }
    }

    /// Read a month name, or `None` when the name is not a month.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "January" => Some(Self::January),
            "February" => Some(Self::February),
            "March" => Some(Self::March),
            "April" => Some(Self::April),
            "May" => Some(Self::May),
            "June" => Some(Self::June),
            "July" => Some(Self::July),
            "August" => Some(Self::August),
            "September" => Some(Self::September),
            "October" => Some(Self::October),
            "November" => Some(Self::November),
            "December" => Some(Self::December),
            _ => None,
        }
    }
}

/// The wood that is felled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Species {
    /// Oak: the tree for ground work.
    Oak,
    /// Alder: the tree for marsh piles.
    Alder,
}

impl Species {
    /// Every wood of the table, in table order.
    pub const ALL: [Self; WOODS] = [Self::Oak, Self::Alder];

    /// The table entry this wood reads, for example `timber.oak`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::Oak => "timber.oak",
            Self::Alder => "timber.alder",
        }
    }

    /// The name of the wood, as the table writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Oak => "oak",
            Self::Alder => "alder",
        }
    }

    /// The place of the wood in the table's list.
    const fn index(self) -> usize {
        match self {
            Self::Oak => 0,
            Self::Alder => 1,
        }
    }

    /// Read a wood name, or `None` when the name is not a wood of the table.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "oak" => Some(Self::Oak),
            "alder" => Some(Self::Alder),
            _ => None,
        }
    }
}

/// What a log is cut into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Product {
    /// A stake for ground work.
    Stake,
    /// A pila shaft for a javelin.
    PilaShaft,
}

impl Product {
    /// Every product of the table, in table order.
    pub const ALL: [Self; PRODUCTS] = [Self::Stake, Self::PilaShaft];

    /// The table entry this product reads, for example `product.stake`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::Stake => "product.stake",
            Self::PilaShaft => "product.pila_shaft",
        }
    }

    /// The name of the product, as a man would say it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stake => "stake",
            Self::PilaShaft => "pila shaft",
        }
    }

    /// The place of the product in the table's list.
    const fn index(self) -> usize {
        match self {
            Self::Stake => 0,
            Self::PilaShaft => 1,
        }
    }
}

/// What a wood is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Ground work: the stakes of a camp and a rampart.
    GroundWork,
    /// Marsh piles: a pile driven in wet ground.
    MarshPiles,
}

impl Purpose {
    /// Every purpose, in table order.
    pub const ALL: [Self; 2] = [Self::GroundWork, Self::MarshPiles];

    /// The name of the purpose, as a man would say it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GroundWork => "ground work",
            Self::MarshPiles => "marsh piles",
        }
    }

    /// Read a purpose name, or `None` when the name is not a purpose.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ground_work" => Some(Self::GroundWork),
            "marsh_piles" => Some(Self::MarshPiles),
            _ => None,
        }
    }
}

/// When a tree may be felled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Season<'a> {
    /// The first month of the season.
    pub first: Month,
    /// The last month of the season.
    pub last: Month,
    /// Why the season runs from one month to the other.
    pub reason: &'a str,
    /// Where the season comes from.
    pub source: &'a str,
    /// How sure we are of the season.
    pub certainty: Certainty,
}

impl Season<'_> {
    /// True when a tree may be felled in this month.
    ///
    /// The season runs from the first month to the last one and wraps the end
    /// of the year when the first month comes after the last.
    #[must_use]
    pub fn holds(&self, month: Month) -> bool {
        if self.first <= self.last {
            (self.first..=self.last).contains(&month)
        } else {
            month >= self.first || month <= self.last
        }
    }
}

/// One wood of the table and what it costs to fell it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tree<'a> {
    /// The wood this entry is.
    pub species: Species,
    /// What the wood is used for.
    pub purpose: Purpose,
    /// The days the sap drains after the tree is cut to the heart.
    pub sap_days: u32,
    /// The axe strokes that fell the tree.
    pub strokes: u32,
    /// Where the numbers come from.
    pub source: &'a str,
    /// How sure we are of them.
    pub certainty: Certainty,
}

/// What one log of a wood becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Yield<'a> {
    /// The product this entry is.
    pub product: Product,
    /// The wood the product comes from, or `None` when any wood serves it.
    pub tree: Option<Species>,
    /// How many pieces one log makes.
    pub per_log: u32,
    /// Where the number comes from.
    pub source: &'a str,
    /// How sure we are of it.
    pub certainty: Certainty,
}

/// The timber table: the felling season, the woods and what a log becomes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timber<'a> {
    season: Season<'a>,
    trees: [Tree<'a>; WOODS],
    yields: [Yield<'a>; PRODUCTS],
}

impl Timber<'static> {
    /// The timber table shipped in `content/rome/tech/timber.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/tech/timber.toml");

    /// Read the shipped timber table.
    ///
    /// # Errors
    ///
    /// Same as [`Timber::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Timber<'a> {
    /// Read one timber table.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or
    /// a section has a number and no `ref`, and the typed variants when a
    /// section or key is missing, unknown, zero, too large or says nothing
    /// about how sure we are of it.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        Ok(Self {
            season: season(&table)?,
            trees: [tree(&table, Species::Oak)?, tree(&table, Species::Alder)?],
            yields: [
                yield_of(&table, Product::Stake)?,
                yield_of(&table, Product::PilaShaft)?,
            ],
        })
    }

    /// The felling season.
    #[must_use]
    pub fn season(&self) -> &Season<'a> {
        &self.season
    }

    /// The wood of this name.
    #[must_use]
    pub fn tree(&self, species: Species) -> &Tree<'a> {
        &self.trees[species.index()]
    }

    /// What a log of any wood becomes.
    #[must_use]
    pub fn product(&self, product: Product) -> &Yield<'a> {
        &self.yields[product.index()]
    }

    /// Fell trees and cut their logs into pieces.
    ///
    /// The season gates the felling: a month outside the season is refused
    /// with the reason the table gives. The wood gates the product: a stake
    /// is ground work and comes from oak, while a pila shaft may come from
    /// any wood.
    ///
    /// # Errors
    ///
    /// [`TimberError::OutOfSeason`] when the month is not a felling month,
    /// [`TimberError::WrongTree`] when the wood does not serve the product,
    /// and [`TimberError::NoLogs`] when there is no tree to fell.
    pub fn fell(
        &self,
        species: Species,
        product: Product,
        month: Month,
        logs: u32,
    ) -> Result<Felling<'a>, TimberError<'a>> {
        if logs == 0 {
            return Err(TimberError::NoLogs);
        }
        if !self.season.holds(month) {
            return Err(TimberError::OutOfSeason {
                month,
                first: self.season.first,
                last: self.season.last,
                reason: self.season.reason,
            });
        }
        let tree = self.tree(species);
        let from_log = self.product(product);
        if let Some(wanted) = from_log.tree {
            if wanted != species {
                return Err(TimberError::WrongTree {
                    product,
                    wanted,
                    got: species,
                    purpose: tree.purpose,
                });
            }
        }
        let felling = Felling {
            species,
            product,
            month,
            logs,
            pieces: from_log.per_log.saturating_mul(logs),
            sap_days: tree.sap_days,
            strokes: tree.strokes,
            source: tree.source,
            hash: 0,
        };
        let hash = felling.digest();
        Ok(Felling { hash, ..felling })
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/timber/1");
        mix_text(&mut hasher, self.season.first.as_str());
        mix_text(&mut hasher, self.season.last.as_str());
        mix_text(&mut hasher, self.season.reason);
        for tree in &self.trees {
            mix_text(&mut hasher, tree.species.as_str());
            mix_text(&mut hasher, tree.purpose.as_str());
            hasher.write_u32(tree.sap_days);
            hasher.write_u32(tree.strokes);
        }
        for from_log in &self.yields {
            mix_text(&mut hasher, from_log.product.as_str());
            match from_log.tree {
                Some(tree) => {
                    hasher.write_u8(1);
                    mix_text(&mut hasher, tree.as_str());
                }
                None => hasher.write_u8(0),
            }
            hasher.write_u32(from_log.per_log);
        }
        hasher.finish()
    }
}

/// One felling: the wood, the month, the work and the pieces of the logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Felling<'a> {
    /// The wood that was felled.
    pub species: Species,
    /// What the logs became.
    pub product: Product,
    /// The month the trees were felled in.
    pub month: Month,
    /// How many trees were felled.
    pub logs: u32,
    /// How many pieces the logs made.
    pub pieces: u32,
    /// The days the sap drained.
    pub sap_days: u32,
    /// The axe strokes the felling took.
    pub strokes: u32,
    /// Where the numbers come from.
    pub source: &'a str,
    /// A digest of the felling: the same felling digests the same.
    pub hash: u64,
}

impl Felling<'_> {
    /// A digest of the felling: the wood, the product, the month and the work.
    fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/timber/felling/1");
        mix_text(&mut hasher, self.species.as_str());
        mix_text(&mut hasher, self.product.as_str());
        mix_text(&mut hasher, self.month.as_str());
        hasher.write_u32(self.logs);
        hasher.write_u32(self.pieces);
        hasher.write_u32(self.sap_days);
        hasher.write_u32(self.strokes);
        hasher.finish()
    }
}

/// Why a felling was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimberError<'a> {
    /// The month is not a felling month.
    OutOfSeason {
        /// The month of the felling.
        month: Month,
        /// The first month of the season.
        first: Month,
        /// The last month of the season.
        last: Month,
        /// Why the season runs from one month to the other.
        reason: &'a str,
    },
    /// The wood does not serve the product.
    WrongTree {
        /// The product that was wanted.
        product: Product,
        /// The wood the product comes from.
        wanted: Species,
        /// The wood that was felled.
        got: Species,
        /// What the felled wood is for.
        purpose: Purpose,
    },
    /// There was no tree to fell.
    NoLogs,
}

impl fmt::Display for TimberError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfSeason {
                month,
                first,
                last,
                reason,
            } => write!(
                f,
                "{} is not a felling month: the season runs from {} to {}: {reason}",
                month.as_str(),
                first.as_str(),
                last.as_str()
            ),
            Self::WrongTree {
                product,
                wanted,
                got,
                purpose,
            } => write!(
                f,
                "a {} comes from {}, but {} is for {}",
                product.as_str(),
                wanted.as_str(),
                got.as_str(),
                purpose.as_str()
            ),
            Self::NoLogs => write!(f, "there is no tree to fell"),
        }
    }
}

impl std::error::Error for TimberError<'_> {}

/// The season entry.
fn season<'a>(table: &Table<'a>) -> Result<Season<'a>, LoadError<'a>> {
    let entry = table
        .entry("season")
        .ok_or(LoadError::MissingEntry { entry: "season" })?;
    only(entry, &["first_month", "last_month", "reason", "certainty"])?;
    Ok(Season {
        first: month(entry, "first_month")?,
        last: month(entry, "last_month")?,
        reason: trimmed(entry, "reason").ok_or_else(|| LoadError::MissingKey {
            entry: entry.key(),
            key: "reason",
        })?,
        source: source(entry)?,
        certainty: certainty(entry)?,
    })
}

/// One wood entry.
fn tree<'a>(table: &Table<'a>, species: Species) -> Result<Tree<'a>, LoadError<'a>> {
    let entry = table
        .entry(species.entry())
        .ok_or_else(|| LoadError::MissingEntry {
            entry: species.entry(),
        })?;
    only(entry, &["purpose", "sap_days", "strokes", "certainty"])?;
    let word = trimmed(entry, "purpose").ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key: "purpose",
    })?;
    Ok(Tree {
        species,
        purpose: Purpose::parse(word).ok_or_else(|| LoadError::BadValue {
            entry: entry.key(),
            key: "purpose",
            value: word,
        })?,
        sap_days: positive(entry, "sap_days")?,
        strokes: positive(entry, "strokes")?,
        source: source(entry)?,
        certainty: certainty(entry)?,
    })
}

/// One product entry.
fn yield_of<'a>(table: &Table<'a>, product: Product) -> Result<Yield<'a>, LoadError<'a>> {
    let entry = table
        .entry(product.entry())
        .ok_or_else(|| LoadError::MissingEntry {
            entry: product.entry(),
        })?;
    only(entry, &["tree", "per_log", "certainty"])?;
    let word = trimmed(entry, "tree").ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key: "tree",
    })?;
    let tree = match word {
        "any" => None,
        word => Some(Species::parse(word).ok_or_else(|| LoadError::BadValue {
            entry: entry.key(),
            key: "tree",
            value: word,
        })?),
    };
    Ok(Yield {
        product,
        tree,
        per_log: positive(entry, "per_log")?,
        source: source(entry)?,
        certainty: certainty(entry)?,
    })
}

/// One month from a section.
fn month<'a>(entry: &Entry<'a>, key: &'a str) -> Result<Month, LoadError<'a>> {
    let word = trimmed(entry, key).ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key,
    })?;
    Month::parse(word).ok_or_else(|| LoadError::BadValue {
        entry: entry.key(),
        key,
        value: word,
    })
}

/// One whole number from a section that must be above zero.
fn positive<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LoadError<'a>> {
    let value = u32::try_from(int(entry, key)?).map_err(|_| LoadError::NumberOverflow {
        entry: entry.key(),
        key,
    })?;
    if value == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key,
        });
    }
    Ok(value)
}

/// The trimmed text of a key, when it is there and not empty.
fn trimmed<'a>(entry: &Entry<'a>, key: &str) -> Option<&'a str> {
    entry
        .text(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The source a section names.
fn source<'a>(entry: &Entry<'a>) -> Result<&'a str, LoadError<'a>> {
    trimmed(entry, "ref").ok_or_else(|| LoadError::MissingRef { entry: entry.key() })
}

/// How sure we are of a section.
fn certainty<'a>(entry: &Entry<'a>) -> Result<Certainty, LoadError<'a>> {
    let word = trimmed(entry, "certainty")
        .ok_or_else(|| LoadError::MissingCertainty { entry: entry.key() })?;
    Certainty::parse(word).ok_or_else(|| LoadError::BadValue {
        entry: entry.key(),
        key: "certainty",
        value: word,
    })
}

/// Mix one text into a digest, length first so two texts cannot swap halves.
fn mix_text(hasher: &mut Fnv64, text: &str) {
    hasher.write_len(text.len());
    hasher.write_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped timber table.
    fn timber() -> Timber<'static> {
        Timber::shipped().expect("the shipped timber table reads")
    }

    #[test]
    fn the_shipped_timber_table_reads_the_season_and_the_woods() {
        let timber = timber();
        let season = timber.season();
        assert_eq!(
            season.first,
            Month::September,
            "the season opens in early autumn"
        );
        assert_eq!(season.last, Month::March, "and closes in spring");
        let oak = timber.tree(Species::Oak);
        assert_eq!(oak.purpose, Purpose::GroundWork, "oak is for ground work");
        let alder = timber.tree(Species::Alder);
        assert_eq!(
            alder.purpose,
            Purpose::MarshPiles,
            "alder is for marsh piles"
        );
    }

    #[test]
    fn every_section_says_where_it_comes_from_and_how_sure_we_are() {
        let timber = timber();
        let season = timber.season();
        assert!(!season.source.is_empty(), "the season names no source");
        assert!(!season.reason.is_empty(), "the season gives no reason");
        assert!(
            Certainty::ALL.contains(&season.certainty),
            "the season has no certainty of the four"
        );
        for species in Species::ALL {
            let tree = timber.tree(species);
            assert!(
                tree.source.contains("rome.md"),
                "{} names no source",
                species.as_str()
            );
            assert!(
                Certainty::ALL.contains(&tree.certainty),
                "{} has no certainty of the four",
                species.as_str()
            );
        }
        for product in Product::ALL {
            let from_log = timber.product(product);
            assert!(
                from_log.source.contains("rome.md"),
                "{} names no source",
                product.as_str()
            );
            assert!(
                Certainty::ALL.contains(&from_log.certainty),
                "{} has no certainty of the four",
                product.as_str()
            );
        }
    }

    #[test]
    fn a_spring_felling_is_refused_with_the_reason() {
        let error = timber()
            .fell(Species::Oak, Product::Stake, Month::April, 1)
            .expect_err("April is not a felling month");
        assert_eq!(
            error,
            TimberError::OutOfSeason {
                month: Month::April,
                first: Month::September,
                last: Month::March,
                reason: timber().season().reason,
            }
        );
        let reason = error.to_string();
        assert!(reason.contains("April"), "{reason}");
        assert!(reason.contains("Vitruvius 2.9"), "{reason}");
    }

    #[test]
    fn the_summer_months_are_refused_too() {
        for month in [Month::May, Month::June, Month::July, Month::August] {
            let error = timber()
                .fell(Species::Oak, Product::Stake, month, 1)
                .expect_err("the sap is up in the summer");
            assert!(
                matches!(error, TimberError::OutOfSeason { .. }),
                "{month:?}: {error}"
            );
        }
    }

    #[test]
    fn the_felling_months_are_the_ones_the_season_names() {
        for month in Month::ALL {
            let in_season = matches!(
                month,
                Month::September
                    | Month::October
                    | Month::November
                    | Month::December
                    | Month::January
                    | Month::February
                    | Month::March
            );
            let fell = timber().fell(Species::Oak, Product::Stake, month, 1);
            assert_eq!(fell.is_ok(), in_season, "{month:?} in season: {in_season}");
        }
    }

    #[test]
    fn one_log_yields_its_stake_count() {
        let timber = timber();
        let felling = timber
            .fell(Species::Oak, Product::Stake, Month::January, 1)
            .expect("January is a felling month");
        let stakes = timber.product(Product::Stake);
        assert_eq!(
            felling.pieces, stakes.per_log,
            "one log yields its stake count"
        );
        assert_eq!(felling.pieces, 8, "the table gives eight stakes a log");
        assert_eq!(felling.logs, 1);
        assert_eq!(felling.species, Species::Oak);
        assert_eq!(felling.product, Product::Stake);
    }

    #[test]
    fn more_logs_yield_more_stakes() {
        let felling = timber()
            .fell(Species::Oak, Product::Stake, Month::December, 4)
            .expect("December is a felling month");
        assert_eq!(felling.pieces, 32, "four logs make four times the stakes");
    }

    #[test]
    fn a_pila_shaft_may_come_from_either_wood() {
        for species in Species::ALL {
            let felling = timber()
                .fell(species, Product::PilaShaft, Month::February, 1)
                .expect("a shaft may come from any wood");
            assert_eq!(felling.pieces, 4, "one log makes four shafts");
            assert_eq!(felling.species, species);
        }
    }

    #[test]
    fn alder_does_not_make_stakes() {
        let error = timber()
            .fell(Species::Alder, Product::Stake, Month::October, 1)
            .expect_err("alder is for marsh piles");
        assert_eq!(
            error,
            TimberError::WrongTree {
                product: Product::Stake,
                wanted: Species::Oak,
                got: Species::Alder,
                purpose: Purpose::MarshPiles,
            }
        );
        assert!(error.to_string().contains("marsh piles"), "{error}");
    }

    #[test]
    fn the_same_felling_replays_equal() {
        let timber = timber();
        let first = timber
            .fell(Species::Oak, Product::Stake, Month::January, 2)
            .expect("the first felling");
        let second = timber
            .fell(Species::Oak, Product::Stake, Month::January, 2)
            .expect("the second felling");
        assert_eq!(first, second, "the same felling replays equal");
        assert_eq!(first.hash, second.hash, "and digests the same");
        assert_ne!(first.hash, 0, "the digest is not empty");
    }

    #[test]
    fn a_felling_without_logs_is_refused() {
        let error = timber()
            .fell(Species::Oak, Product::Stake, Month::January, 0)
            .expect_err("no tree to fell");
        assert_eq!(error, TimberError::NoLogs);
    }

    #[test]
    fn the_sap_drains_and_the_axe_works() {
        let timber = timber();
        let felling = timber
            .fell(Species::Oak, Product::Stake, Month::November, 1)
            .expect("November is a felling month");
        let oak = timber.tree(Species::Oak);
        assert_eq!(felling.sap_days, oak.sap_days, "the sap drains");
        assert_eq!(felling.strokes, oak.strokes, "the axe works");
        assert!(felling.sap_days > 0, "the sap drains for some days");
    }

    #[test]
    fn two_loads_hash_the_same() {
        assert_eq!(timber().hash(), timber().hash(), "two loads agree");
        let text = Timber::SHIPPED.replace("per_log = 8", "per_log = 9");
        let other = Timber::parse(&text).expect("a table with another count");
        assert_ne!(
            timber().hash(),
            other.hash(),
            "another count hashes differently"
        );
    }

    #[test]
    fn a_section_with_no_ref_is_refused() {
        let text = "[season]\nfirst_month = \"September\"\nlast_month = \"March\"\nreason = \"x\"\ncertainty = \"attested\"\n";
        let error = Timber::parse(text).expect_err("the season has no source");
        assert!(
            matches!(error, LoadError::MissingRef { entry: "season" }),
            "{error}"
        );
    }

    #[test]
    fn a_number_with_no_ref_is_refused_by_the_reader() {
        let text = "[season]\nfirst_month = 9\nlast_month = 3\n";
        let error = Timber::parse(text).expect_err("the season has no source");
        assert!(matches!(error, LoadError::Table(_)), "{error}");
    }

    #[test]
    fn a_section_without_a_certainty_is_refused() {
        let text = Timber::SHIPPED.replace("certainty = \"attested\"", "");
        let error = Timber::parse(&text).expect_err("the season says nothing about it");
        assert_eq!(
            error,
            LoadError::MissingCertainty { entry: "season" },
            "{error}"
        );
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = Timber::SHIPPED.replace("sap_days = 30", "sap_day = 30");
        let error = Timber::parse(&text).expect_err("a typo in a key");
        assert!(error.to_string().contains("sap_day"), "{error}");
    }

    #[test]
    fn a_float_is_refused() {
        let text = Timber::SHIPPED.replace("sap_days = 30", "sap_days = 30.5");
        let error = Timber::parse(&text).expect_err("no float in a Rome table");
        assert!(matches!(error, LoadError::Table(_)), "{error}");
    }

    #[test]
    fn a_table_without_a_wood_is_refused() {
        let text = "[season]\nfirst_month = \"September\"\nlast_month = \"March\"\nreason = \"x\"\ncertainty = \"attested\"\nref = \"rome.md section 4.4\"\n";
        let error = Timber::parse(text).expect_err("no wood to fell");
        assert!(matches!(error, LoadError::MissingEntry { .. }), "{error}");
    }
}
