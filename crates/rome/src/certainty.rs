//! certainty: how sure we are of every piece of kit, and of every number the game shows.
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
//!
//! The registry is the same rule for the numbers. Every number the game shows lives
//! in one table with its tier, a note and its source. The loader refuses a number
//! with no tier and names its key, and the renderer asks the registry for a number
//! by key ([`Registry::render`]), so a number that is not registered cannot be drawn
//! at all. The contested numbers stay visible: the Roman foot is 295.7 mm or
//! 297.4 mm, and the card says so next to the 296 mm the game uses.

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

/// One number as the registry table holds it, before the loader has checked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim<'a> {
    /// The key the renderer asks for, for example `weapon.pilum.length`.
    pub key: &'a str,
    /// The name the HUD shows, for example `pila`.
    pub label: &'a str,
    /// The number, in units of ten to the minus `decimals` of `unit`.
    pub value: u32,
    /// How many decimals the number carries.
    pub decimals: u8,
    /// The unit of the number, for example `m`. Empty for a plain count.
    pub unit: &'a str,
    /// How sure we are of the number. `None` means the table forgot to say.
    pub tier: Option<Certainty>,
    /// Why the number says what it says.
    pub note: &'a str,
    /// Where the number comes from.
    pub source: &'a str,
}

/// One loaded number: a claim that has a tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Number<'a> {
    /// The key of the number, for example `weapon.pilum.length`.
    pub key: &'a str,
    /// The name the HUD shows, for example `pila`.
    pub label: &'a str,
    /// The number, in units of ten to the minus `decimals` of `unit`.
    pub value: u32,
    /// How many decimals the number carries.
    pub decimals: u8,
    /// The unit of the number, for example `m`. Empty for a plain count.
    pub unit: &'a str,
    /// How sure we are of the number.
    pub tier: Certainty,
    /// Why the number says what it says.
    pub note: &'a str,
    /// Where the number comes from.
    pub source: &'a str,
}

impl<'a> Number<'a> {
    /// The loaded form of one claim.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::MissingCertainty`] when the claim has no tier; the
    /// refusal names the claim's key.
    fn load(claim: &Claim<'a>) -> Result<Self, LoadError<'a>> {
        let tier = claim
            .tier
            .ok_or(LoadError::MissingCertainty { entry: claim.key })?;
        Ok(Self {
            key: claim.key,
            label: claim.label,
            value: claim.value,
            decimals: claim.decimals,
            unit: claim.unit,
            tier,
            note: claim.note,
            source: claim.source,
        })
    }

    /// The number as it is written, for example `2.0`.
    #[must_use]
    pub fn value_text(&self) -> String {
        let scale = scale(self.decimals);
        let whole = self.value / scale;
        let rest = self.value % scale;
        if self.decimals == 0 {
            return whole.to_string();
        }
        format!("{whole}.{rest:0width$}", width = usize::from(self.decimals))
    }

    /// The line the HUD draws for this number: `pila 2.0 m (Vegetius 2.15)`.
    #[must_use]
    pub fn text(&self) -> String {
        let value = self.value_text();
        if self.unit.is_empty() {
            return format!("{} {} ({})", self.label, value, self.source);
        }
        format!("{} {} {} ({})", self.label, value, self.unit, self.source)
    }
}

/// What the HUD draws for one number of the registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered<'a> {
    /// The line the HUD shows: `pila 2.0 m (Vegetius 2.15)`.
    pub text: String,
    /// The tier chip beside the number.
    pub tier: Certainty,
    /// The note the codex card shows under the number.
    pub note: &'a str,
    /// The source the codex card names.
    pub source: &'a str,
}

impl Rendered<'_> {
    /// The word the tier chip carries: `attested`, `reconstructed`, `contested` or
    /// `invented`.
    #[must_use]
    pub fn chip(&self) -> &'static str {
        self.tier.as_str()
    }
}

/// The registry: every number the game shows, with its tier, its note and its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry<'a> {
    numbers: Vec<Number<'a>>,
}

impl Registry<'static> {
    /// The registry shipped with the crate.
    ///
    /// # Errors
    ///
    /// Same as [`Registry::load`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the table.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::load(&SHIPPED_CLAIMS)
    }
}

impl<'a> Registry<'a> {
    /// Load a registry from its table of claims.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::MissingCertainty`] when a claim has no tier; the refusal
    /// names the claim's key.
    pub fn load(claims: &[Claim<'a>]) -> Result<Self, LoadError<'a>> {
        let mut numbers = Vec::with_capacity(claims.len());
        for claim in claims {
            numbers.push(Number::load(claim)?);
        }
        Ok(Self { numbers })
    }

    /// Every number of the registry, in table order.
    #[must_use]
    pub fn numbers(&self) -> &[Number<'a>] {
        &self.numbers
    }

    /// The number with this key, for example `weapon.pilum.length`.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Unregistered`] when the registry holds no such key.
    pub fn number<'k>(&self, key: &'k str) -> Result<&Number<'a>, LoadError<'k>> {
        self.numbers
            .iter()
            .find(|number| number.key == key)
            .ok_or(LoadError::Unregistered { key })
    }

    /// The HUD line for one number, with the tier chip that goes beside it.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Unregistered`] when the registry holds no such key: a
    /// number that is not registered cannot be drawn at all.
    pub fn render<'k>(&self, key: &'k str) -> Result<Rendered<'a>, LoadError<'k>> {
        let number = self.number(key)?;
        Ok(Rendered {
            text: number.text(),
            tier: number.tier,
            note: number.note,
            source: number.source,
        })
    }

    /// A digest of the loaded registry: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/registry/1");
        for number in &self.numbers {
            write_text(&mut hasher, number.key);
            write_text(&mut hasher, number.label);
            hasher.write_u32(number.value);
            hasher.write_u8(number.decimals);
            write_text(&mut hasher, number.unit);
            write_text(&mut hasher, number.tier.as_str());
            write_text(&mut hasher, number.note);
            write_text(&mut hasher, number.source);
        }
        hasher.finish()
    }
}

/// Ten to the power of the decimals of a number.
const fn scale(decimals: u8) -> u32 {
    let mut factor = 1u32;
    let mut left = decimals;
    while left > 0 {
        factor = factor.saturating_mul(10);
        left -= 1;
    }
    factor
}

/// The registry table: one claim per number the game shows.
///
/// The order of a claim is key, label, value, decimals, unit, tier, note, source.
macro_rules! claims {
    ($($key:literal => $label:literal, $value:literal, $decimals:literal, $unit:literal, $tier:ident, $note:literal, $source:literal;)*) => {
        [$(Claim {
            key: $key,
            label: $label,
            value: $value,
            decimals: $decimals,
            unit: $unit,
            tier: Some(Certainty::$tier),
            note: $note,
            source: $source,
        }),*]
    };
}

/// Every number the game shows, with its tier, its note and its source.
///
/// The table is a Rust const, so it is embedded at build time: the tick never reads
/// a file, two loads of it hash the same, and a draft table from another tool can
/// never be loaded. The values are the ones `docs/plan/rome.md` section 5 gives, in
/// the unit the source itself uses.
const SHIPPED_CLAIMS: [Claim<'static>; 38] = claims! {
    "units.roman_foot_mm" => "Roman foot", 296, 0, "mm", Contested,
        "The Roman foot is contested: 295.7 mm or 297.4 mm. We use 296 mm, and the card says so.",
        "rome.md section 5 units";
    "units.roman_mile_m" => "Roman mile", 1480, 0, "m", Reconstructed,
        "A thousand paces of five feet, about 1.48 km.",
        "rome.md section 5 units";
    "line.frontage_ft" => "Frontage per man", 3, 0, "ft", Attested,
        "Three feet of front for each man in the line.",
        "Vegetius 3.14";
    "line.rank_gap_ft" => "Gap between ranks", 6, 0, "ft", Attested,
        "Six feet from one rank to the next.",
        "Vegetius 3.14-15";
    "camp.ditch.one_night.width_ft" => "One-night ditch, width", 5, 0, "ft", Attested,
        "The smallest ditch the sources give, cut in one night.",
        "Vegetius 3.8; Hyginus 49";
    "camp.ditch.one_night.depth_ft" => "One-night ditch, depth", 3, 0, "ft", Attested,
        "Three feet deep.",
        "Vegetius 3.8; Hyginus 49";
    "camp.ditch.light.width_ft" => "Light ditch, width", 9, 0, "ft", Attested,
        "Nine feet wide, for a night that brings a light threat.",
        "Vegetius 1.24";
    "camp.ditch.light.depth_ft" => "Light ditch, depth", 7, 0, "ft", Attested,
        "Seven feet deep.",
        "Vegetius 1.24";
    "camp.ditch.serious.width_ft" => "Serious ditch, width", 12, 0, "ft", Attested,
        "Twelve feet wide, for a serious threat.",
        "Vegetius 1.24";
    "camp.ditch.serious.depth_ft" => "Serious ditch, depth", 9, 0, "ft", Attested,
        "Nine feet deep.",
        "Vegetius 1.24";
    "camp.rampart.width_ft" => "Rampart, width", 8, 0, "ft", Attested,
        "Turf, thrown up out of the ditch.",
        "Hyginus 50; Vegetius 3.8";
    "camp.rampart.height_ft" => "Rampart, height", 6, 0, "ft", Attested,
        "Six feet of turf wall.",
        "Hyginus 50; Vegetius 3.8";
    "camp.street.via_principalis_ft" => "Via principalis", 60, 0, "ft", Attested,
        "The main street across the camp.",
        "Hyginus 14";
    "camp.street.via_quintana_ft" => "Via quintana", 40, 0, "ft", Attested,
        "The second street, behind the first.",
        "Hyginus 17";
    "order.watch_count" => "Watches a night", 4, 0, "", Attested,
        "Four watches a night.",
        "Vegetius 3.8; Polybius 6.35";
    "order.watch_hours" => "Hours on a watch", 3, 0, "h", Attested,
        "A watch runs up to three hours.",
        "Vegetius 3.8; Polybius 6.35";
    "order.century_men" => "Men in a century", 80, 0, "men", Reconstructed,
        "Eighty men under a centurion and an optio; Vegetius gives a hundred for his day.",
        "rome.md section 5; Vegetius 2.14";
    "order.tent_men" => "Men in a tent", 8, 0, "men", Reconstructed,
        "Eight men in the early Imperial tent; Vegetius gives ten for his day.",
        "rome.md section 5; Vegetius 2.13";
    "march.pack_load_lb" => "Pack load", 60, 0, "lb", Attested,
        "Sixty pounds, about 20 kg, besides arms; the recruit's drill.",
        "Vegetius 1.19";
    "march.step_regular_mi" => "Regular step", 20, 0, "mi", Attested,
        "Twenty Roman miles, about 29.6 km, in five summer hours.",
        "Vegetius 1.9";
    "march.step_full_mi" => "Full step", 24, 0, "mi", Attested,
        "Twenty-four Roman miles, about 35.5 km, in five summer hours.",
        "Vegetius 1.9";
    "weapon.gladius.blade_cm" => "Gladius, blade", 45, 0, "cm", Attested,
        "The Pompeii blades run 45 to 50 cm.",
        "rome.md section 3";
    "weapon.gladius.kill_depth_in" => "Gladius, killing depth", 2, 0, "in", Attested,
        "A thrust two inches deep kills.",
        "Vegetius 1.12";
    "weapon.pilum.length" => "pila", 20, 1, "m", Reconstructed,
        "The sources give the shaft and the head; the total is the plan's round figure.",
        "Vegetius 2.15";
    "weapon.pilum.shaft_ft" => "pila, shaft", 55, 1, "ft", Attested,
        "The wooden shaft is five and a half feet.",
        "Vegetius 2.15";
    "weapon.pilum.head_in" => "pila, head", 9, 0, "in", Attested,
        "The iron head is nine inches.",
        "Vegetius 2.15";
    "weapon.pilum.throw_min_m" => "pila, shortest throw", 15, 0, "m", Attested,
        "The shortest throw that still carries.",
        "Vegetius 2.15";
    "weapon.pilum.throw_max_m" => "pila, longest throw", 25, 0, "m", Attested,
        "The far end of the effective throw.",
        "Vegetius 2.15";
    "weapon.pilum.throw_far_m" => "pila, utmost reach", 30, 0, "m", Reconstructed,
        "About thirty metres: the reach of a pilum in modern tests, not in the sources.",
        "rome.md section 5; modern tests";
    "weapon.pilum.throw_speed_mps" => "pila, throw speed", 36, 0, "m/s", Invented,
        "Ours: the arena throws at 36 m/s so a volley lands inside the thirty metres.",
        "crates/arena/src/pilum.rs";
    "siege.alesia.ditch_ft" => "Alesia, ditch", 20, 0, "ft", Attested,
        "The ditch Caesar's army dug at Alesia.",
        "Caesar BG 7.72-73";
    "siege.alesia.rampart_ft" => "Alesia, rampart", 12, 0, "ft", Attested,
        "The rampart that closed the works.",
        "Caesar BG 7.72-73";
    "siege.alesia.tower_ft" => "Alesia, towers", 80, 0, "ft", Attested,
        "A tower every eighty feet.",
        "Caesar BG 7.72-73";
    "bridge.pile_ft" => "Caesar's bridge, pile", 15, 1, "ft", Attested,
        "Piles a foot and a half thick.",
        "Caesar BG 4.17";
    "bridge.pile_gap_ft" => "Caesar's bridge, gap", 2, 0, "ft", Attested,
        "Two feet apart, in pairs.",
        "Caesar BG 4.17";
    "bridge.build_days" => "Caesar's bridge, days", 10, 0, "", Attested,
        "Ten days to build the bridge over the Rhine.",
        "Caesar BG 4.17";
    "engine.carroballistae" => "Carroballistae in a legion", 55, 0, "", Attested,
        "The late army keeps fifty-five carroballistae.",
        "Vegetius 2.25";
    "engine.onagri" => "Onagri in a legion", 10, 0, "", Attested,
        "And ten onagri.",
        "Vegetius 2.25";
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camp::{Camp, Threat};
    use crate::line::Line;

    const MANUBALLISTA: &str = "kit.manuballista";
    const FOOT: &str = "units.roman_foot_mm";
    const PILUM: &str = "weapon.pilum.length";

    #[test]
    fn a_number_with_no_tier_is_refused_and_names_its_key() {
        let claim = Claim {
            key: FOOT,
            label: "Roman foot",
            value: 296,
            decimals: 0,
            unit: "mm",
            tier: None,
            note: "The Roman foot is contested.",
            source: "rome.md section 5 units",
        };
        let error = Registry::load(&[claim]).expect_err("a number with no tier");
        assert_eq!(
            error,
            LoadError::MissingCertainty {
                entry: "units.roman_foot_mm"
            }
        );
        assert!(error.to_string().contains("units.roman_foot_mm"), "{error}");
    }

    #[test]
    fn an_unregistered_key_cannot_be_rendered() {
        let registry = Registry::shipped().expect("shipped registry");
        let error = registry
            .render("weapon.pilum.nope")
            .expect_err("a number that is not registered");
        assert_eq!(
            error,
            LoadError::Unregistered {
                key: "weapon.pilum.nope"
            }
        );
        assert!(error.to_string().contains("weapon.pilum.nope"), "{error}");
    }

    #[test]
    fn the_roman_foot_is_printed_with_the_tier_that_says_it_is_contested() {
        let registry = Registry::shipped().expect("shipped registry");
        let rendered = registry.render(FOOT).expect("the Roman foot");
        assert_eq!(rendered.tier, Certainty::Contested);
        assert_eq!(rendered.chip(), "contested");
        assert!(rendered.text.contains("296"), "{}", rendered.text);
        assert!(rendered.note.contains("295.7"), "{}", rendered.note);
        assert!(rendered.note.contains("297.4"), "{}", rendered.note);
    }

    #[test]
    fn the_hud_line_of_the_pila_reads_pila_2_0_m_vegetius_2_15() {
        let registry = Registry::shipped().expect("shipped registry");
        let rendered = registry.render(PILUM).expect("the pilum");
        assert_eq!(rendered.text, "pila 2.0 m (Vegetius 2.15)");
        assert_eq!(rendered.source, "Vegetius 2.15");
    }

    #[test]
    fn every_registered_number_carries_a_tier_a_note_and_a_source() {
        let registry = Registry::shipped().expect("shipped registry");
        for number in registry.numbers() {
            assert!(
                Certainty::ALL.contains(&number.tier),
                "{} has no tier of the four",
                number.key
            );
            assert!(!number.note.is_empty(), "{} has no note", number.key);
            assert!(!number.source.is_empty(), "{} has no source", number.key);
        }
    }

    #[test]
    fn the_registry_holds_each_key_once() {
        let registry = Registry::shipped().expect("shipped registry");
        let mut keys: Vec<&str> = registry.numbers().iter().map(|number| number.key).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a key is registered twice");
    }

    #[test]
    fn the_registry_agrees_with_the_camp_and_line_tables() {
        let registry = Registry::shipped().expect("shipped registry");
        let camp = Camp::shipped().expect("shipped camp table");
        let line = Line::shipped().expect("shipped line table");
        let foot = registry.number(FOOT).expect("the Roman foot").value;
        let mm = |key: &str| -> u32 {
            let feet = registry.number(key).expect("a registered number").value;
            feet * foot
        };
        assert_eq!(
            mm("camp.ditch.one_night.width_ft"),
            camp.ditch(Threat::OneNight).width.mm
        );
        assert_eq!(
            mm("camp.ditch.one_night.depth_ft"),
            camp.ditch(Threat::OneNight).depth.mm
        );
        assert_eq!(
            mm("camp.ditch.light.width_ft"),
            camp.ditch(Threat::Light).width.mm
        );
        assert_eq!(
            mm("camp.ditch.light.depth_ft"),
            camp.ditch(Threat::Light).depth.mm
        );
        assert_eq!(
            mm("camp.ditch.serious.width_ft"),
            camp.ditch(Threat::Serious).width.mm
        );
        assert_eq!(
            mm("camp.ditch.serious.depth_ft"),
            camp.ditch(Threat::Serious).depth.mm
        );
        assert_eq!(mm("camp.rampart.width_ft"), camp.rampart.width.mm);
        assert_eq!(mm("camp.rampart.height_ft"), camp.rampart.height.mm);
        assert_eq!(
            mm("camp.street.via_principalis_ft"),
            camp.streets.via_principalis.mm
        );
        assert_eq!(
            mm("camp.street.via_quintana_ft"),
            camp.streets.via_quintana.mm
        );
        assert_eq!(mm("line.frontage_ft"), line.frontage.mm);
        assert_eq!(mm("line.rank_gap_ft"), line.rank_gap.mm);
    }

    #[test]
    fn two_loads_of_the_registry_hash_the_same() {
        let registry = Registry::shipped().expect("shipped registry");
        let again = Registry::shipped().expect("shipped registry");
        assert_eq!(registry.hash(), again.hash());
    }

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
