//! kit: wear and mend. A piece of kit wears per stroke and per hit, and a
//! mend puts it back to work.
//!
//! A soldier's kit wears out: the gladius loses its edge, the dolabra dulls
//! with every dig, the lorica plates lose their rivets and the hobnails wear
//! out of the caligae. The wear rule of a piece says what a stroke or a hit
//! takes off it and where it stops working (its bar). The mend job says what a
//! mend takes: the tool, the work, and the material it eats.
//!
//! The rules are data: `content/rome/tech/mend.toml` holds every number and
//! names its source in `ref`. The reader refuses a section that holds a number
//! and no `ref`, so no number enters the game unsourced. Values are whole
//! numbers only: no float ever enters a Rome rule.
//!
//! Determinism: the kit is a fixed list of pieces, the condition of a piece is
//! a whole number, and the same wear and the same mend give the same kit and
//! the same digest twice.

use core::fmt;

use crate::table::{Entry, Table};
use crate::{int, only, LoadError};
use engine_core::Fnv64;

/// The names of one piece of kit: its key and the table entries that hold its
/// wear rule and its mend job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Names {
    /// The key of the piece, for example `dolabra.edge`.
    key: &'static str,
    /// The entry that holds the wear rule, for example `wear.dolabra.edge`.
    wear_entry: &'static str,
    /// The entry that holds the mend job, for example `mend.dolabra.edge`.
    mend_entry: &'static str,
}

/// The pieces of the kit that wear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// The edge of the dolabra, the tool that digs and chops.
    DolabraEdge,
    /// The edge of the gladius.
    GladiusEdge,
    /// The lobate hinge that joins two lorica plates.
    LoricaHinge,
    /// The hobnails in the caligae.
    CaligaeHobnails,
}

impl Piece {
    /// Every piece of kit that wears, in table order.
    pub const ALL: [Self; 4] = [
        Self::DolabraEdge,
        Self::GladiusEdge,
        Self::LoricaHinge,
        Self::CaligaeHobnails,
    ];

    /// The key of the piece, for example `dolabra.edge`.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.names().key
    }

    /// Where the piece sits in the list.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::DolabraEdge => 0,
            Self::GladiusEdge => 1,
            Self::LoricaHinge => 2,
            Self::CaligaeHobnails => 3,
        }
    }

    /// The names of the piece: its key, the entry that holds its wear rule and
    /// the entry that holds its mend job.
    const fn names(self) -> Names {
        match self {
            Self::DolabraEdge => Names {
                key: "dolabra.edge",
                wear_entry: "wear.dolabra.edge",
                mend_entry: "mend.dolabra.edge",
            },
            Self::GladiusEdge => Names {
                key: "gladius.edge",
                wear_entry: "wear.gladius.edge",
                mend_entry: "mend.gladius.edge",
            },
            Self::LoricaHinge => Names {
                key: "lorica.hinge",
                wear_entry: "wear.lorica.hinge",
                mend_entry: "mend.lorica.hinge",
            },
            Self::CaligaeHobnails => Names {
                key: "caligae.hobnails",
                wear_entry: "wear.caligae.hobnails",
                mend_entry: "mend.caligae.hobnails",
            },
        }
    }
}

/// The wear rule of one piece of kit: how it wears and where it stops working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule<'a> {
    /// The condition a new piece has.
    pub full: u32,
    /// Below this condition the piece is spent and needs a mend.
    pub bar: u32,
    /// How many halves the piece is made of: a lobate hinge is two.
    pub halves: u32,
    /// What one stroke of work takes off the piece.
    pub wear_per_stroke: u32,
    /// What one hit takes off the piece.
    pub wear_per_hit: u32,
    /// The source of these numbers.
    pub source: &'a str,
}

/// One mend job: what it takes to restore a piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job<'a> {
    /// The tool the work needs.
    pub tool: &'a str,
    /// The unit the work is counted in.
    pub unit: &'a str,
    /// How much work the job takes.
    pub work: u32,
    /// The engine ticks one unit of work costs.
    pub ticks_per_unit: u32,
    /// The material the job eats, when it eats one.
    pub material: Option<&'a str>,
    /// How much of that material each half of the piece needs.
    pub per_half: u32,
    /// The source of these numbers.
    pub source: &'a str,
}

/// The mend bench: the tools at hand and the material lying on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bench<'a> {
    tools: Vec<&'a str>,
    materials: Vec<(&'a str, u32)>,
}

impl<'a> Bench<'a> {
    /// An empty bench.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A bench with a tool on it.
    #[must_use]
    pub fn with_tool(mut self, name: &'a str) -> Self {
        self.tools.push(name);
        self
    }

    /// A bench with material on it.
    #[must_use]
    pub fn with_material(mut self, name: &'a str, amount: u32) -> Self {
        self.materials.push((name, amount));
        self
    }

    /// True when the bench holds the tool.
    #[must_use]
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains(&name)
    }

    /// How much of a material the bench holds.
    #[must_use]
    pub fn material(&self, name: &str) -> u32 {
        self.materials
            .iter()
            .find(|(held, _)| *held == name)
            .map_or(0, |(_, amount)| *amount)
    }

    /// Take material off the bench; the amount taken is returned.
    pub fn take(&mut self, name: &str, amount: u32) -> u32 {
        let Some((_, held)) = self.materials.iter_mut().find(|(held, _)| *held == name) else {
            return 0;
        };
        let taken = (*held).min(amount);
        *held -= taken;
        taken
    }
}

/// What one mend did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mended<'a> {
    /// The piece that was mended.
    pub piece: Piece,
    /// The unit the work was counted in.
    pub unit: &'a str,
    /// The work the mend took.
    pub work: u32,
    /// The engine ticks the mend took.
    pub ticks: u32,
    /// The condition of the piece after the mend.
    pub condition: u32,
}

/// Why a mend did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MendError<'a> {
    /// The piece still works, so it needs no mend.
    NotSpent {
        /// The piece that was not mended.
        piece: Piece,
    },
    /// The bench holds no tool for the job.
    MissingTool {
        /// The piece that was not mended.
        piece: Piece,
        /// The tool the job needs.
        tool: &'a str,
    },
    /// The bench holds too little of the material the job eats.
    MissingMaterial {
        /// The piece that was not mended.
        piece: Piece,
        /// The material the job eats.
        material: &'a str,
        /// How much of it the job needs.
        needed: u32,
        /// How much of it the bench holds.
        have: u32,
    },
}

impl fmt::Display for MendError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSpent { piece } => write!(f, "the {} is not spent", piece.key()),
            Self::MissingTool { piece, tool } => {
                write!(f, "the mend of the {} needs the tool `{tool}`", piece.key())
            }
            Self::MissingMaterial {
                piece,
                material,
                needed,
                have,
            } => write!(
                f,
                "the mend of the {} needs {needed} of `{material}`, the bench holds {have}",
                piece.key()
            ),
        }
    }
}

impl std::error::Error for MendError<'_> {}

/// The kit of one soldier: the wear rules, the mend jobs and the condition of
/// every piece.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kit<'a> {
    rules: Vec<Rule<'a>>,
    jobs: Vec<Job<'a>>,
    conditions: Vec<u32>,
}

impl Kit<'static> {
    /// The mend table shipped in `content/rome/tech/mend.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/tech/mend.toml");

    /// Read the shipped mend table.
    ///
    /// # Errors
    ///
    /// Same as [`Kit::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Kit<'a> {
    /// Read one mend table.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or
    /// a section holds a number and no `ref`, and the typed variants when a
    /// section or a key is missing, unknown, zero or too large for its number.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let mut rules = Vec::with_capacity(Piece::ALL.len());
        let mut jobs = Vec::with_capacity(Piece::ALL.len());
        let mut conditions = Vec::with_capacity(Piece::ALL.len());
        for piece in Piece::ALL {
            let wear = rule(&table, piece)?;
            conditions.push(wear.full);
            rules.push(wear);
            jobs.push(job(&table, piece)?);
        }
        Ok(Self {
            rules,
            jobs,
            conditions,
        })
    }

    /// The wear rule of a piece.
    #[must_use]
    pub fn rule(&self, piece: Piece) -> Rule<'a> {
        self.rules[piece.index()]
    }

    /// The mend job of a piece.
    #[must_use]
    pub fn job(&self, piece: Piece) -> Job<'a> {
        self.jobs[piece.index()]
    }

    /// The condition left in a piece.
    #[must_use]
    pub fn condition(&self, piece: Piece) -> u32 {
        self.conditions[piece.index()]
    }

    /// True when the piece is below its bar and needs a mend.
    #[must_use]
    pub fn spent(&self, piece: Piece) -> bool {
        self.condition(piece) < self.rule(piece).bar
    }

    /// Put one stroke of work into a piece.
    pub fn stroke(&mut self, piece: Piece) -> u32 {
        self.strokes(piece, 1)
    }

    /// Put `count` strokes of work into a piece: the condition drops by what
    /// a stroke takes.
    pub fn strokes(&mut self, piece: Piece, count: u32) -> u32 {
        let lost = self.rule(piece).wear_per_stroke.saturating_mul(count);
        let condition = &mut self.conditions[piece.index()];
        *condition = condition.saturating_sub(lost);
        *condition
    }

    /// Put one hit into a piece.
    pub fn hit(&mut self, piece: Piece) -> u32 {
        self.hits(piece, 1)
    }

    /// Put `count` hits into a piece: the condition drops by what a hit takes.
    pub fn hits(&mut self, piece: Piece, count: u32) -> u32 {
        let lost = self.rule(piece).wear_per_hit.saturating_mul(count);
        let condition = &mut self.conditions[piece.index()];
        *condition = condition.saturating_sub(lost);
        *condition
    }

    /// Mend a piece: the bench needs the tool and the material, and the piece
    /// goes back to the condition of a new one.
    ///
    /// # Errors
    ///
    /// [`MendError::NotSpent`] when the piece still works,
    /// [`MendError::MissingTool`] when the bench holds no tool for the job, and
    /// [`MendError::MissingMaterial`] when the bench holds too little of the
    /// material the job eats.
    pub fn mend(
        &mut self,
        piece: Piece,
        bench: &mut Bench<'_>,
    ) -> Result<Mended<'a>, MendError<'a>> {
        let job = self.job(piece);
        let rule = self.rule(piece);
        if !self.spent(piece) {
            return Err(MendError::NotSpent { piece });
        }
        if !bench.has_tool(job.tool) {
            return Err(MendError::MissingTool {
                piece,
                tool: job.tool,
            });
        }
        let needed = job.per_half.saturating_mul(rule.halves);
        if let Some(material) = job.material {
            let have = bench.material(material);
            if have < needed {
                return Err(MendError::MissingMaterial {
                    piece,
                    material,
                    needed,
                    have,
                });
            }
            bench.take(material, needed);
        }
        self.conditions[piece.index()] = rule.full;
        Ok(Mended {
            piece,
            unit: job.unit,
            work: job.work,
            ticks: job.work.saturating_mul(job.ticks_per_unit),
            condition: rule.full,
        })
    }

    /// A digest of the kit: the same wear and the same mend give the same
    /// digest.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/kit/1");
        for piece in Piece::ALL {
            let rule = self.rule(piece);
            write_text(&mut hasher, piece.key());
            hasher.write_u32(rule.full);
            hasher.write_u32(rule.bar);
            hasher.write_u32(rule.halves);
            hasher.write_u32(rule.wear_per_stroke);
            hasher.write_u32(rule.wear_per_hit);
            hasher.write_u32(self.condition(piece));
        }
        hasher.finish()
    }
}

/// The wear rule of one piece, read from the table.
fn rule<'a>(table: &Table<'a>, piece: Piece) -> Result<Rule<'a>, LoadError<'a>> {
    let names = piece.names();
    let entry = table
        .entry(names.wear_entry)
        .ok_or(LoadError::MissingEntry {
            entry: names.wear_entry,
        })?;
    only(
        entry,
        &["full", "bar", "halves", "wear_per_stroke", "wear_per_hit"],
    )?;
    let full = number(entry, "full")?;
    if full == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key: "full",
        });
    }
    let bar = number(entry, "bar")?;
    if bar > full {
        return Err(LoadError::BadValue {
            entry: entry.key(),
            key: "bar",
            value: "above the full condition",
        });
    }
    let halves = number(entry, "halves")?;
    if halves == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key: "halves",
        });
    }
    Ok(Rule {
        full,
        bar,
        halves,
        wear_per_stroke: number(entry, "wear_per_stroke")?,
        wear_per_hit: number(entry, "wear_per_hit")?,
        source: entry.source(),
    })
}

/// The mend job of one piece, read from the table.
fn job<'a>(table: &Table<'a>, piece: Piece) -> Result<Job<'a>, LoadError<'a>> {
    let names = piece.names();
    let entry = table
        .entry(names.mend_entry)
        .ok_or(LoadError::MissingEntry {
            entry: names.mend_entry,
        })?;
    only(
        entry,
        &[
            "tool",
            "unit",
            "work",
            "ticks_per_unit",
            "material",
            "per_half",
        ],
    )?;
    let work = number(entry, "work")?;
    if work == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key: "work",
        });
    }
    let ticks_per_unit = number(entry, "ticks_per_unit")?;
    if ticks_per_unit == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key: "ticks_per_unit",
        });
    }
    let material = entry
        .text("material")
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let per_half = match material {
        Some(_) => number(entry, "per_half")?,
        None => {
            if entry.int("per_half").is_some() {
                return Err(LoadError::BadValue {
                    entry: entry.key(),
                    key: "per_half",
                    value: "without a material",
                });
            }
            0
        }
    };
    Ok(Job {
        tool: text(entry, "tool")?,
        unit: text(entry, "unit")?,
        work,
        ticks_per_unit,
        material,
        per_half,
        source: entry.source(),
    })
}

/// One whole number from a section that fits a `u32`.
fn number<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LoadError<'a>> {
    let value = int(entry, key)?;
    match u32::try_from(value) {
        Ok(number) => Ok(number),
        Err(_) => Err(LoadError::NumberOverflow {
            entry: entry.key(),
            key,
        }),
    }
}

/// The text of a key, when it is there and not empty.
fn text<'a>(entry: &Entry<'a>, key: &'a str) -> Result<&'a str, LoadError<'a>> {
    let value = entry.text(key).ok_or_else(|| LoadError::MissingKey {
        entry: entry.key(),
        key,
    })?;
    let value = value.trim();
    if value.is_empty() {
        return Err(LoadError::MissingKey {
            entry: entry.key(),
            key,
        });
    }
    Ok(value)
}

/// Mix one text into a digest, length first so two texts cannot swap halves.
fn write_text(hasher: &mut Fnv64, text: &str) {
    hasher.write_len(text.len());
    hasher.write_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::TableError;

    /// The shipped mend table.
    fn mend() -> Kit<'static> {
        Kit::shipped().expect("the shipped mend table reads")
    }

    /// A bench with the tools of the fabrica and some rivets and nails.
    fn bench(rivets: u32) -> Bench<'static> {
        Bench::new()
            .with_tool("whetstone")
            .with_tool("hammer")
            .with_material("rivets", rivets)
            .with_material("nails", 8)
    }

    #[test]
    fn the_mend_table_reads_every_piece() {
        let kit = mend();
        for piece in Piece::ALL {
            let rule = kit.rule(piece);
            assert!(
                rule.source.contains("rome.md"),
                "{} names no source",
                piece.key()
            );
            assert!(rule.full > 0, "{} has no condition", piece.key());
            assert!(
                rule.bar <= rule.full,
                "{} has its bar above its full",
                piece.key()
            );
            assert!(rule.halves > 0, "{} has no halves", piece.key());
            assert!(
                rule.wear_per_stroke > 0 || rule.wear_per_hit > 0,
                "{} never wears",
                piece.key()
            );
            let job = kit.job(piece);
            assert!(
                job.source.contains("rome.md"),
                "{} names no source",
                piece.key()
            );
            assert!(!job.tool.is_empty(), "{} needs no tool", piece.key());
            assert!(job.work > 0, "{} takes no work", piece.key());
        }
    }

    #[test]
    fn two_hundred_dig_strokes_blunt_the_dolabra_below_its_bar() {
        let mut kit = mend();
        let rule = kit.rule(Piece::DolabraEdge);
        assert_eq!(kit.condition(Piece::DolabraEdge), rule.full, "a new edge");
        kit.strokes(Piece::DolabraEdge, 100);
        assert!(
            !kit.spent(Piece::DolabraEdge),
            "a hundred dig strokes leave a working edge"
        );
        kit.strokes(Piece::DolabraEdge, 100);
        assert!(
            kit.spent(Piece::DolabraEdge),
            "two hundred dig strokes blunt the dolabra below its bar"
        );
        assert!(kit.condition(Piece::DolabraEdge) < rule.bar);
    }

    #[test]
    fn a_regrind_restores_the_dolabra() {
        let mut kit = mend();
        kit.strokes(Piece::DolabraEdge, 200);
        assert!(kit.spent(Piece::DolabraEdge));
        let mended = kit
            .mend(Piece::DolabraEdge, &mut bench(0))
            .expect("the regrind works");
        assert_eq!(mended.piece, Piece::DolabraEdge);
        assert_eq!(mended.unit, "stroke");
        assert!(mended.ticks > 0, "the grind takes time");
        assert_eq!(
            kit.condition(Piece::DolabraEdge),
            kit.rule(Piece::DolabraEdge).full,
            "the edge is as good as new"
        );
        assert!(!kit.spent(Piece::DolabraEdge));
    }

    #[test]
    fn a_broken_hinge_mends_with_five_rivets_per_half() {
        let mut kit = mend();
        let job = kit.job(Piece::LoricaHinge);
        assert_eq!(job.material, Some("rivets"), "the hinge is riveted");
        assert_eq!(job.per_half, 5, "five rivets per half");
        assert_eq!(
            kit.rule(Piece::LoricaHinge).halves,
            2,
            "a lobate hinge is two halves"
        );
        kit.hits(Piece::LoricaHinge, 50);
        assert!(kit.spent(Piece::LoricaHinge), "the hinge is broken");

        let mut nine = bench(9);
        let error = kit
            .mend(Piece::LoricaHinge, &mut nine)
            .expect_err("nine rivets do not mend two halves");
        assert_eq!(
            error,
            MendError::MissingMaterial {
                piece: Piece::LoricaHinge,
                material: "rivets",
                needed: 10,
                have: 9,
            }
        );
        assert_eq!(nine.material("rivets"), 9, "a refused mend takes nothing");

        let mut ten = bench(10);
        kit.mend(Piece::LoricaHinge, &mut ten)
            .expect("ten rivets mend the hinge");
        assert_eq!(ten.material("rivets"), 0, "the mend takes the rivets");
        assert!(!kit.spent(Piece::LoricaHinge), "the hinge holds again");
    }

    #[test]
    fn a_mend_without_its_tool_is_refused() {
        let mut kit = mend();
        kit.hits(Piece::GladiusEdge, 40);
        assert!(kit.spent(Piece::GladiusEdge));
        let mut bare = Bench::new();
        let error = kit
            .mend(Piece::GladiusEdge, &mut bare)
            .expect_err("no whetstone on the bench");
        assert_eq!(
            error,
            MendError::MissingTool {
                piece: Piece::GladiusEdge,
                tool: "whetstone",
            }
        );
    }

    #[test]
    fn a_piece_that_is_not_spent_needs_no_mend() {
        let mut kit = mend();
        let error = kit
            .mend(Piece::DolabraEdge, &mut bench(10))
            .expect_err("a working edge needs no grind");
        assert_eq!(
            error,
            MendError::NotSpent {
                piece: Piece::DolabraEdge
            }
        );
    }

    #[test]
    fn the_hobnails_are_replaced_with_new_nails() {
        let mut kit = mend();
        kit.strokes(Piece::CaligaeHobnails, 160);
        assert!(kit.spent(Piece::CaligaeHobnails));
        let mut nails = bench(0);
        kit.mend(Piece::CaligaeHobnails, &mut nails)
            .expect("the nails are replaced");
        assert_eq!(nails.material("nails"), 0, "the mend takes the nails");
        assert!(!kit.spent(Piece::CaligaeHobnails));
    }

    /// One play of the kit: dig, fight, mend, and wear again.
    fn play() -> Kit<'static> {
        let mut kit = mend();
        kit.strokes(Piece::DolabraEdge, 200);
        kit.hits(Piece::GladiusEdge, 40);
        kit.hits(Piece::LoricaHinge, 50);
        kit.strokes(Piece::CaligaeHobnails, 160);
        kit.mend(Piece::DolabraEdge, &mut bench(0))
            .expect("the grind");
        kit.mend(Piece::GladiusEdge, &mut bench(0))
            .expect("the regrind");
        kit.mend(Piece::LoricaHinge, &mut bench(10))
            .expect("the rivets");
        kit.mend(Piece::CaligaeHobnails, &mut bench(0))
            .expect("the nails");
        kit.strokes(Piece::DolabraEdge, 10);
        kit
    }

    #[test]
    fn the_same_wear_and_mend_give_the_same_kit_twice() {
        let first = play();
        let second = play();
        assert_eq!(first, second, "the same play gives the same kit");
        assert_eq!(first.hash(), second.hash(), "and the same digest");
        assert_ne!(first.hash(), 0, "the digest is not empty");
    }

    #[test]
    fn a_section_with_numbers_and_no_ref_is_refused() {
        let text = "[wear.dolabra.edge]\nfull = 250\nbar = 100\nhalves = 1\nwear_per_stroke = 1\nwear_per_hit = 0\n";
        let error = Kit::parse(text).expect_err("the wear rule has no source");
        assert!(
            matches!(error, LoadError::Table(TableError::MissingRef { .. })),
            "{error}"
        );
        assert!(error.to_string().contains("wear.dolabra.edge"), "{error}");
    }

    #[test]
    fn a_float_is_refused() {
        let text = Kit::SHIPPED.replace("wear_per_stroke = 1", "wear_per_stroke = 1.5");
        let error = Kit::parse(&text).expect_err("a float");
        assert!(matches!(error, LoadError::Table(_)), "{error}");
    }

    #[test]
    fn a_missing_piece_is_refused() {
        let text = Kit::SHIPPED.replace("[wear.dolabra.edge]", "[wear.dolabra.spade]");
        let error = Kit::parse(&text).expect_err("the dolabra has no rule");
        assert!(error.to_string().contains("wear.dolabra.edge"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = Kit::SHIPPED.replace("wear_per_stroke = 1", "wear_per_storke = 1");
        let error = Kit::parse(&text).expect_err("a typo in a key");
        assert!(error.to_string().contains("wear_per_storke"), "{error}");
    }
}
