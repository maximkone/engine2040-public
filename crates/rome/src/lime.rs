//! lime: burn the stone, slake the hot lime, mix the mortar.
//!
//! The loop is the Roman one from `docs/plan/rome.md` section 4.4: burn the stone
//! in a kiln, where a third of its weight leaves as gas; slake the lime while it is
//! hot; then mix it with sand. The numbers live in `content/rome/tech/lime.toml` and
//! every one of them names its source in `ref`.
//!
//! The mixes are the ones Vitruvius 2.5 gives through the plan: 1 lime : 3 pit
//! sand, or 1 lime : 2 river sand. The right mix for the sand at hand sets at full
//! strength; a mix off the ratio sets weaker, and the table says by how much.
//!
//! Integers only: masses in grams, strength in per-mille, the kiln in hours of game
//! time. No clock, no IO, no float: the same stone and the same mix give the same
//! lime and the same digest twice.

use core::fmt;

use engine_core::Fnv64;

use crate::table::{Entry, Table, TableError};
use crate::{only, LoadError};

/// The sand a mortar is mixed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sand {
    /// Sand from a pit: angular, and it takes more lime.
    Pit,
    /// Sand from a river: rounded, and it takes less lime.
    River,
}

impl Sand {
    /// Every sand kind, in table order.
    pub const ALL: [Self; 2] = [Self::Pit, Self::River];

    /// The table entry this sand reads, for example `mix.pit_sand`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::Pit => "mix.pit_sand",
            Self::River => "mix.river_sand",
        }
    }

    /// The word this sand is written as.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pit => "pit_sand",
            Self::River => "river_sand",
        }
    }

    /// Position in the table's sand list.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Pit => 0,
            Self::River => 1,
        }
    }
}

/// What the kiln takes off the stone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burn<'a> {
    /// The part of the weight that leaves as gas.
    pub lost_numerator: u32,
    /// The parts the weight is counted in.
    pub lost_denominator: u32,
    /// Where the rule comes from.
    pub source: &'a str,
}

/// One sand and the ratio its right mix wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mix<'a> {
    /// The sand of this mix.
    pub sand: Sand,
    /// The parts of sand the right mix wants per part of lime.
    pub sand_per_lime: u32,
    /// Where the ratio comes from.
    pub source: &'a str,
}

/// How strong a mix sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strength<'a> {
    /// The strength of the right mix, in per-mille.
    pub full_per_mille: u32,
    /// What each part of sand off the right ratio costs, in per-mille.
    pub penalty_per_part: u32,
    /// Where the rule comes from.
    pub source: &'a str,
}

/// The lime table: the burn, the two mixes and the strength they set at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lime<'a> {
    id: &'a str,
    burn: Burn<'a>,
    mixes: [Mix<'a>; Sand::ALL.len()],
    strength: Strength<'a>,
}

/// The stone that goes into the kiln, and how long the kiln burns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kiln {
    /// The stone that goes in, in grams.
    pub stone_grams: u32,
    /// How long the kiln burns, in hours of game time.
    pub hours: u32,
}

impl Kiln {
    /// A kiln with stone in it and a time to burn.
    #[must_use]
    pub const fn new(stone_grams: u32, hours: u32) -> Self {
        Self { stone_grams, hours }
    }
}

/// What the kiln made: the hot quicklime and what left the stone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Burnt {
    /// The quicklime, in grams.
    pub lime_grams: u32,
    /// The gas that left the stone, in grams.
    pub gas_grams: u32,
    /// The dust the stone left, in grams: the part that does not fall in thirds.
    pub dust_grams: u32,
    /// The engine ticks the burn took. One tick is one hour of game time.
    pub ticks: u32,
}

/// The slaked lime, ready to mix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slaked {
    /// The slaked lime, in grams.
    pub lime_grams: u32,
}

/// One mortar mix: what went into it and how strong it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mortar {
    /// The sand of the mix.
    pub sand: Sand,
    /// The parts of sand per part of lime in the mix.
    pub sand_per_lime: u32,
    /// The lime of the mix, in grams.
    pub lime_grams: u32,
    /// The sand of the mix, in grams.
    pub sand_grams: u32,
    /// The strength the mix sets at, in per-mille.
    pub strength_per_mille: u32,
    /// A digest of the mix: the same mix always digests the same.
    pub hash: u64,
}

impl Mortar {
    /// A digest of the mix: the same mix always digests the same.
    fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/lime/mortar/1");
        hasher.write_len(self.sand.as_str().len());
        hasher.write_str(self.sand.as_str());
        hasher.write_u32(self.sand_per_lime);
        hasher.write_u32(self.lime_grams);
        hasher.write_u32(self.sand_grams);
        hasher.write_u32(self.strength_per_mille);
        hasher.finish()
    }
}

/// Why the lime loop refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LimeError<'a> {
    /// The table could not be read into its typed form.
    Load(LoadError<'a>),
    /// The burn rule is written backwards: the stone would keep nothing.
    BadBurn,
    /// The kiln was given no stone.
    EmptyStone,
    /// The kiln was given no time.
    EmptyKiln,
    /// There is no lime to slake or to mix.
    EmptyMix,
    /// The mix wants no sand at all.
    NoSand,
    /// The mass does not fit in a gram count.
    TooMuch,
}

impl fmt::Display for LimeError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => error.fmt(f),
            Self::BadBurn => write!(f, "the burn rule takes all the weight of the stone"),
            Self::EmptyStone => write!(f, "the kiln was given no stone"),
            Self::EmptyKiln => write!(f, "the kiln was given no time"),
            Self::EmptyMix => write!(f, "there is no lime to work"),
            Self::NoSand => write!(f, "the mix wants no sand"),
            Self::TooMuch => write!(f, "the mass does not fit in a gram count"),
        }
    }
}

impl std::error::Error for LimeError<'_> {}

impl<'a> From<LoadError<'a>> for LimeError<'a> {
    fn from(error: LoadError<'a>) -> Self {
        Self::Load(error)
    }
}

impl<'a> From<TableError<'a>> for LimeError<'a> {
    fn from(error: TableError<'a>) -> Self {
        Self::Load(error.into())
    }
}

impl Lime<'static> {
    /// The lime table shipped in `content/rome/tech/lime.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/tech/lime.toml");

    /// Read the shipped lime table.
    ///
    /// # Errors
    ///
    /// Same as [`Lime::parse`]; a test keeps the shipped table valid, so this doubles
    /// as a build check on the content file.
    pub fn shipped() -> Result<Self, LimeError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Lime<'a> {
    /// Read one lime table.
    ///
    /// # Errors
    ///
    /// Returns [`LimeError::Load`] when the text is not a well-formed table or a
    /// section holds a number and no `ref`, and the typed variants when a section or
    /// key is missing, unknown or zero.
    pub fn parse(text: &'a str) -> Result<Self, LimeError<'a>> {
        let table = Table::parse(text)?;
        let recipe = table
            .entry("recipe")
            .ok_or(LoadError::MissingEntry { entry: "recipe" })?;
        only(recipe, &["id"])?;
        let id = recipe
            .text("id")
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or(LoadError::MissingKey {
                entry: "recipe",
                key: "id",
            })?;
        Ok(Self {
            id,
            burn: burn(&table)?,
            mixes: [mix(&table, Sand::Pit)?, mix(&table, Sand::River)?],
            strength: strength(&table)?,
        })
    }

    /// The recipe's id, as the table writes it.
    #[must_use]
    pub fn id(&self) -> &'a str {
        self.id
    }

    /// The burn rule: the part of the weight that leaves the stone.
    #[must_use]
    pub fn burn_rule(&self) -> Burn<'a> {
        self.burn
    }

    /// The right mix for this sand.
    #[must_use]
    pub fn right_mix(&self, sand: Sand) -> Mix<'a> {
        self.mixes[sand.index()]
    }

    /// The strength rule: the full strength and what a wrong mix costs.
    #[must_use]
    pub fn strength_rule(&self) -> Strength<'a> {
        self.strength
    }

    /// A digest of the loaded numbers: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/lime/1");
        hasher.write_len(self.id.len());
        hasher.write_str(self.id);
        hasher.write_u32(self.burn.lost_numerator);
        hasher.write_u32(self.burn.lost_denominator);
        for mix in &self.mixes {
            hasher.write_len(mix.sand.as_str().len());
            hasher.write_str(mix.sand.as_str());
            hasher.write_u32(mix.sand_per_lime);
        }
        hasher.write_u32(self.strength.full_per_mille);
        hasher.write_u32(self.strength.penalty_per_part);
        hasher.finish()
    }

    /// Burn the stone: a third of its weight leaves as gas.
    ///
    /// The kiln is a time skip: one tick is one hour of game time, and the caller
    /// says how long the kiln burns. A stone that does not fall in thirds leaves
    /// the rest as dust, so the kiln keeps every gram it was given.
    ///
    /// # Errors
    ///
    /// [`LimeError::EmptyStone`] when the kiln gets no stone, [`LimeError::EmptyKiln`]
    /// when it gets no time, and [`LimeError::TooMuch`] when the weight does not fit
    /// in a gram count.
    pub fn burn(&self, kiln: Kiln) -> Result<Burnt, LimeError<'a>> {
        if kiln.stone_grams == 0 {
            return Err(LimeError::EmptyStone);
        }
        if kiln.hours == 0 {
            return Err(LimeError::EmptyKiln);
        }
        let stone = u64::from(kiln.stone_grams);
        let numerator = u64::from(self.burn.lost_numerator);
        let denominator = u64::from(self.burn.lost_denominator);
        let gas = stone * numerator / denominator;
        let lime = stone * (denominator - numerator) / denominator;
        let dust = stone - gas - lime;
        Ok(Burnt {
            lime_grams: grams(lime)?,
            gas_grams: grams(gas)?,
            dust_grams: grams(dust)?,
            ticks: kiln.hours,
        })
    }

    /// Slake the hot lime: the lime takes up water and is ready to mix.
    ///
    /// The water is not counted here: this version tracks the mass of the lime, and
    /// the slake is what turns the quicklime into the slaked lime the mortar wants.
    ///
    /// # Errors
    ///
    /// [`LimeError::EmptyMix`] when there is no lime to slake.
    pub fn slake(&self, burnt: Burnt) -> Result<Slaked, LimeError<'a>> {
        if burnt.lime_grams == 0 {
            return Err(LimeError::EmptyMix);
        }
        Ok(Slaked {
            lime_grams: burnt.lime_grams,
        })
    }

    /// Mix the slaked lime with sand.
    ///
    /// The right mix for the sand at hand sets at full strength. A mix off the ratio
    /// sets weaker: every part of sand over or under the right ratio costs the
    /// table's step.
    ///
    /// # Errors
    ///
    /// [`LimeError::EmptyMix`] when there is no lime to mix, [`LimeError::NoSand`]
    /// when the mix wants no sand at all, and [`LimeError::TooMuch`] when the sand
    /// does not fit in a gram count.
    pub fn mix(
        &self,
        slaked: Slaked,
        sand: Sand,
        sand_per_lime: u32,
    ) -> Result<Mortar, LimeError<'a>> {
        if slaked.lime_grams == 0 {
            return Err(LimeError::EmptyMix);
        }
        if sand_per_lime == 0 {
            return Err(LimeError::NoSand);
        }
        let sand_grams = u64::from(slaked.lime_grams) * u64::from(sand_per_lime);
        let off = self.right_mix(sand).sand_per_lime.abs_diff(sand_per_lime);
        let cost = u64::from(off) * u64::from(self.strength.penalty_per_part);
        let strength = u64::from(self.strength.full_per_mille).saturating_sub(cost);
        let mortar = Mortar {
            sand,
            sand_per_lime,
            lime_grams: slaked.lime_grams,
            sand_grams: grams(sand_grams)?,
            strength_per_mille: grams(strength)?,
            hash: 0,
        };
        let hash = mortar.digest();
        Ok(Mortar { hash, ..mortar })
    }
}

/// The burn rule: what part of the weight leaves the stone.
fn burn<'a>(table: &Table<'a>) -> Result<Burn<'a>, LimeError<'a>> {
    let entry = table
        .entry("burn")
        .ok_or(LoadError::MissingEntry { entry: "burn" })?;
    only(entry, &["lost_numerator", "lost_denominator"])?;
    let lost_numerator = number(entry, "lost_numerator")?;
    let lost_denominator = number(entry, "lost_denominator")?;
    if lost_denominator == 0 {
        return Err(LoadError::Zero {
            entry: "burn",
            key: "lost_denominator",
        }
        .into());
    }
    if lost_numerator >= lost_denominator {
        return Err(LimeError::BadBurn);
    }
    Ok(Burn {
        lost_numerator,
        lost_denominator,
        source: entry.source(),
    })
}

/// One sand's mix: the ratio the right mix wants.
fn mix<'a>(table: &Table<'a>, sand: Sand) -> Result<Mix<'a>, LimeError<'a>> {
    let entry = table
        .entry(sand.entry())
        .ok_or_else(|| LoadError::MissingEntry {
            entry: sand.entry(),
        })?;
    only(entry, &["sand_per_lime"])?;
    let sand_per_lime = number(entry, "sand_per_lime")?;
    if sand_per_lime == 0 {
        return Err(LoadError::Zero {
            entry: sand.entry(),
            key: "sand_per_lime",
        }
        .into());
    }
    Ok(Mix {
        sand,
        sand_per_lime,
        source: entry.source(),
    })
}

/// The strength rule: the full strength and what a wrong mix costs.
fn strength<'a>(table: &Table<'a>) -> Result<Strength<'a>, LimeError<'a>> {
    let entry = table
        .entry("strength")
        .ok_or(LoadError::MissingEntry { entry: "strength" })?;
    only(entry, &["full_per_mille", "penalty_per_part"])?;
    Ok(Strength {
        full_per_mille: number(entry, "full_per_mille")?,
        penalty_per_part: number(entry, "penalty_per_part")?,
        source: entry.source(),
    })
}

/// One whole number from a section.
fn number<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LimeError<'a>> {
    let value = entry.int(key).ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key,
    })?;
    u32::try_from(value).map_err(|_| {
        LoadError::NumberOverflow {
            entry: entry.key(),
            key,
        }
        .into()
    })
}

/// A mass in grams, when it fits in a gram count.
fn grams<'a>(value: u64) -> Result<u32, LimeError<'a>> {
    u32::try_from(value).map_err(|_| LimeError::TooMuch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped lime table.
    fn lime() -> Lime<'static> {
        Lime::shipped().expect("the shipped lime table reads")
    }

    /// A kiln with three kilos of stone and a day of firing.
    fn kiln() -> Kiln {
        Kiln {
            stone_grams: 3000,
            hours: 24,
        }
    }

    /// The slaked lime a kiln of stone gives.
    fn slaked() -> Slaked {
        lime()
            .slake(lime().burn(kiln()).expect("the kiln burns"))
            .expect("the hot lime slakes")
    }

    #[test]
    fn the_shipped_lime_table_reads_the_burn_and_the_two_mixes() {
        let lime = lime();
        assert_eq!(lime.burn_rule().lost_numerator, 1, "a third leaves");
        assert_eq!(
            lime.burn_rule().lost_denominator,
            3,
            "the weight is counted in thirds"
        );
        assert_eq!(
            lime.right_mix(Sand::Pit).sand_per_lime,
            3,
            "1 lime : 3 pit sand (Vitruvius 2.5)"
        );
        assert_eq!(
            lime.right_mix(Sand::River).sand_per_lime,
            2,
            "1 lime : 2 river sand (Vitruvius 2.5)"
        );
        assert!(
            lime.burn_rule().source.contains("rome.md"),
            "the burn names its source: {}",
            lime.burn_rule().source
        );
    }

    #[test]
    fn the_burn_loses_one_third_of_the_mass() {
        let burnt = lime().burn(kiln()).expect("the kiln burns");
        assert_eq!(burnt.lime_grams, 2000, "two thirds stays as quicklime");
        assert_eq!(burnt.gas_grams, 1000, "a third of the weight leaves");
        assert_eq!(burnt.dust_grams, 0, "the stone fell in thirds");
        assert_eq!(
            burnt.lime_grams + burnt.gas_grams + burnt.dust_grams,
            3000,
            "the kiln keeps the mass it was given"
        );
        assert_eq!(burnt.ticks, 24, "one tick is one hour of game time");
    }

    #[test]
    fn the_right_mix_sets_at_full_strength() {
        let lime = lime();
        let slaked = slaked();
        let pit = lime.mix(slaked, Sand::Pit, 3).expect("the pit mix is made");
        assert_eq!(
            pit.strength_per_mille, 1000,
            "the right mix is full strength"
        );
        assert_eq!(pit.sand_grams, 6000, "three parts of sand to one of lime");
        let river = lime
            .mix(slaked, Sand::River, 2)
            .expect("the river mix is made");
        assert_eq!(
            river.strength_per_mille, 1000,
            "the right mix is full strength"
        );
    }

    #[test]
    fn a_wrong_mix_sets_weaker() {
        let lime = lime();
        let slaked = slaked();
        let rich = lime.mix(slaked, Sand::Pit, 2).expect("the mix is made");
        assert!(
            rich.strength_per_mille < 1000,
            "a wrong mix sets weaker: {}",
            rich.strength_per_mille
        );
        assert_eq!(
            rich.strength_per_mille, 750,
            "one part off the ratio costs a quarter of the strength"
        );
        let lean = lime.mix(slaked, Sand::River, 3).expect("the mix is made");
        assert!(
            lean.strength_per_mille < 1000,
            "a wrong mix sets weaker: {}",
            lean.strength_per_mille
        );
    }

    #[test]
    fn the_same_stone_and_the_same_mix_give_the_same_mortar_twice() {
        let lime = lime();
        let first = lime.mix(slaked(), Sand::Pit, 3).expect("the first mix");
        let second = lime.mix(slaked(), Sand::Pit, 3).expect("the second mix");
        assert_eq!(first, second, "the same mix twice");
        assert_eq!(first.hash, second.hash, "and the same digest");
        assert_ne!(first.hash, 0, "the digest is not empty");
    }

    #[test]
    fn two_loads_of_the_table_hash_the_same() {
        assert_eq!(lime().hash(), lime().hash(), "two loads of one table agree");
    }

    #[test]
    fn a_section_with_numbers_and_no_ref_is_refused() {
        let text =
            "[recipe]\nid = \"lime.mortar\"\n\n[burn]\nlost_numerator = 1\nlost_denominator = 3\n";
        let error = Lime::parse(text).expect_err("the burn has no source");
        assert!(error.to_string().contains("burn"), "{error}");
    }

    #[test]
    fn an_empty_kiln_is_refused() {
        let error = lime()
            .burn(Kiln {
                stone_grams: 0,
                hours: 24,
            })
            .expect_err("no stone to burn");
        assert_eq!(error, LimeError::EmptyStone);
        let error = lime()
            .burn(Kiln {
                stone_grams: 100,
                hours: 0,
            })
            .expect_err("no time to burn");
        assert_eq!(error, LimeError::EmptyKiln);
    }

    #[test]
    fn a_mix_without_sand_is_refused() {
        let error = lime()
            .mix(slaked(), Sand::Pit, 0)
            .expect_err("a mix with no sand is not mortar");
        assert_eq!(error, LimeError::NoSand);
    }
}
