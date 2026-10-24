//! scutum: the legionary's shield, built and re-covered, and the damage it shows.
//!
//! Polybius 6.23 gives the build of the scutum: two layers of planks glued
//! together, a cover of linen and then of calfskin, and a rim of iron or
//! bronze round the edge. `content/rome/tech/scutum.toml` holds that build and
//! the wear of a blow; every number in it names its source in `ref`, and the
//! strict reader refuses a number with no source, so no number enters the
//! game unsourced.
//!
//! A blow wears the cover first and the boards after it. The cover is the
//! linen and the calfskin; the boards are the glued planks. The boards hold
//! together until a blow passes their last strength, and that blow splits
//! them: the shield is lost. A re-cover puts a new cover on a shield whose
//! boards still hold; the tears in the old cover are gone and the gashes in
//! the boards stay.
//!
//! The drawn shield is what a renderer paints: a tear for every blow that cut
//! the cover, a gash for every blow that cut the boards, and the split when
//! the boards go. Units are integers only: no float ever enters a Rome rule,
//! and the same table and the same blows give the same digest.

use core::fmt;

use engine_core::Fnv64;

use crate::table::{Entry, Table};
use crate::{int, only, LoadError, Measure};

/// The condition of a whole part of a shield, in per mille.
const WHOLE: u32 = 1000;
/// Most letters one boss takes: the row's cap of 24 strokes (KIT4).
pub const MAX_PUNCH_LETTERS: usize = 24;
/// Magic of a shield's save image (`SCUT`).
const SAVE_MAGIC: [u8; 4] = *b"SCUT";
/// Version of the save image's layout: a reader must know it.
const SAVE_VERSION: u16 = 1;

/// How the plank layers of a shield are joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joint {
    /// The layers are glued together.
    Glued,
    /// The layers are nailed together.
    Nailed,
}

impl Joint {
    /// The word this joint is written as in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Glued => "glued",
            Self::Nailed => "nailed",
        }
    }

    /// Read the word of a joint, or `None` when it is not one of the two.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "glued" => Some(Self::Glued),
            "nailed" => Some(Self::Nailed),
            _ => None,
        }
    }
}

/// What the rim round the edge of a shield is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RimMaterial {
    /// Iron.
    Iron,
    /// Bronze.
    Bronze,
    /// Iron or bronze: what the legion has decides.
    IronOrBronze,
}

impl RimMaterial {
    /// The word this material is written as in the table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Iron => "iron",
            Self::Bronze => "bronze",
            Self::IronOrBronze => "iron_or_bronze",
        }
    }

    /// Read the word of a material, or `None` when it is not one of the three.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "iron" => Some(Self::Iron),
            "bronze" => Some(Self::Bronze),
            "iron_or_bronze" => Some(Self::IronOrBronze),
            _ => None,
        }
    }
}

/// How a shield is built: the boards and the cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Build<'a> {
    /// How many plank layers the boards have.
    pub plank_layers: u32,
    /// How the plank layers are joined.
    pub joint: Joint,
    /// How many linen layers the cover has.
    pub linen_layers: u32,
    /// How many calfskin layers the cover has.
    pub calfskin_layers: u32,
    /// Where the build comes from.
    pub source: &'a str,
}

/// The rim round the edge of a shield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rim<'a> {
    /// How many rim strips the build needs.
    pub strips: u32,
    /// What the rim is made of.
    pub material: RimMaterial,
    /// Where the rim comes from.
    pub source: &'a str,
}

/// The scutum table: how a shield is built and how it wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scutum<'a> {
    /// The height of the shield, in millimetres.
    pub height: Measure<'a>,
    /// The width of the shield, in millimetres.
    pub width: Measure<'a>,
    /// How many cells the face is cut into, across.
    pub cells_x: u32,
    /// How many cells the face is cut into, down.
    pub cells_y: u32,
    /// How the shield is built.
    pub build: Build<'a>,
    /// The rim round the edge.
    pub rim: Rim<'a>,
    /// How much of the cover one blow takes, in per mille.
    pub cover_wear: u32,
    /// How much of the boards one blow takes, in per mille.
    pub board_wear: u32,
    /// Where the wear numbers come from.
    pub source: &'a str,
}

impl Scutum<'static> {
    /// The scutum table shipped in `content/rome/tech/scutum.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/tech/scutum.toml");

    /// Read the shipped scutum table.
    ///
    /// # Errors
    ///
    /// Same as [`Scutum::parse`]; a test keeps the shipped table valid, so this
    /// doubles as a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Scutum<'a> {
    /// Read one scutum table into its typed form.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or
    /// an entry has a number and no `ref`, and the typed variants when a
    /// section or key is missing, unknown, zero, too large for its number, or
    /// a word the typed form does not know.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let shield = table
            .entry("shield")
            .ok_or(LoadError::MissingEntry { entry: "shield" })?;
        only(shield, &["height_mm", "width_mm", "cells_x", "cells_y"])?;
        let height = measure(shield, "height_mm")?;
        let width = measure(shield, "width_mm")?;
        let cells_x = count(shield, "cells_x")?;
        let cells_y = count(shield, "cells_y")?;
        let build = build(&table)?;
        let rim = rim(&table)?;
        let wear = table
            .entry("wear")
            .ok_or(LoadError::MissingEntry { entry: "wear" })?;
        only(wear, &["cover_hit_wear", "board_hit_wear"])?;
        let cover_wear = count(wear, "cover_hit_wear")?;
        let board_wear = count(wear, "board_hit_wear")?;
        Ok(Self {
            height,
            width,
            cells_x,
            cells_y,
            build,
            rim,
            cover_wear,
            board_wear,
            source: wear.source(),
        })
    }

    /// How many cells the face of the shield has.
    #[must_use]
    pub fn cells(&self) -> u32 {
        self.cells_x * self.cells_y
    }

    /// How many blows the cover takes before it is gone.
    #[must_use]
    pub fn cover_hits(&self) -> u32 {
        WHOLE.div_ceil(self.cover_wear)
    }

    /// How many blows the boards take before they split.
    #[must_use]
    pub fn board_hits(&self) -> u32 {
        WHOLE.div_ceil(self.board_wear)
    }

    /// How many blows the shield takes before the boards split.
    ///
    /// The cover wears first and the boards after it, so the budget is the
    /// blows of the cover plus the blows of the boards.
    #[must_use]
    pub fn hit_budget(&self) -> u32 {
        self.cover_hits() + self.board_hits()
    }

    /// A digest of the loaded table: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/scutum/1");
        hasher.write_u32(self.height.mm);
        hasher.write_u32(self.width.mm);
        hasher.write_u32(self.cells_x);
        hasher.write_u32(self.cells_y);
        hasher.write_u32(self.build.plank_layers);
        hasher.write_str(self.build.joint.as_str());
        hasher.write_u32(self.build.linen_layers);
        hasher.write_u32(self.build.calfskin_layers);
        hasher.write_u32(self.rim.strips);
        hasher.write_str(self.rim.material.as_str());
        hasher.write_u32(self.cover_wear);
        hasher.write_u32(self.board_wear);
        hasher.finish()
    }
}

/// One material a shield is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The plank layers of the boards.
    Planks,
    /// The linen layer of the cover.
    Linen,
    /// The calfskin layer of the cover.
    Calfskin,
    /// The rim round the edge.
    Rim,
}

impl Part {
    /// The name of the material, as a refusal names it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Planks => "planks",
            Self::Linen => "linen",
            Self::Calfskin => "calfskin",
            Self::Rim => "rim",
        }
    }
}

/// The materials at hand for a shield.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Parts {
    /// How many plank layers are at hand.
    pub planks: u32,
    /// How many linen layers are at hand.
    pub linen: u32,
    /// How many calfskin layers are at hand.
    pub calfskin: u32,
    /// How many rim strips are at hand.
    pub rim: u32,
}

impl Parts {
    /// The materials one shield needs, as the table asks for them.
    #[must_use]
    pub fn for_build(spec: &Scutum<'_>) -> Self {
        Self {
            planks: spec.build.plank_layers,
            linen: spec.build.linen_layers,
            calfskin: spec.build.calfskin_layers,
            rim: spec.rim.strips,
        }
    }

    /// How much of one material is at hand.
    #[must_use]
    pub fn amount(&self, part: Part) -> u32 {
        match part {
            Part::Planks => self.planks,
            Part::Linen => self.linen,
            Part::Calfskin => self.calfskin,
            Part::Rim => self.rim,
        }
    }

    /// Put materials on the pile.
    pub fn add(&mut self, part: Part, amount: u32) {
        let held = self.amount(part).saturating_add(amount);
        match part {
            Part::Planks => self.planks = held,
            Part::Linen => self.linen = held,
            Part::Calfskin => self.calfskin = held,
            Part::Rim => self.rim = held,
        }
    }

    /// Take materials off the pile and say how much was taken.
    pub fn take(&mut self, part: Part, amount: u32) -> u32 {
        let taken = self.amount(part).min(amount);
        let left = self.amount(part) - taken;
        match part {
            Part::Planks => self.planks = left,
            Part::Linen => self.linen = left,
            Part::Calfskin => self.calfskin = left,
            Part::Rim => self.rim = left,
        }
        taken
    }
}

/// Why a shield could not be built or re-covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldError {
    /// A material is short: the work cannot go on.
    MissingPart {
        /// The material that is short.
        part: Part,
        /// How much the work needs.
        needed: u32,
        /// How much is at hand.
        held: u32,
    },
    /// The boards are split: the shield is lost and no cover saves it.
    Split,
    /// A stroke carried a byte outside the Latin capitals and the space.
    NotALatinCapital {
        /// The byte the punch was handed.
        letter: u8,
    },
    /// The boss is full: the cap of letters is already punched in.
    InscriptionFull {
        /// How many strokes one boss takes.
        cap: usize,
    },
    /// Save bytes that carry no shield.
    BadSave {
        /// The byte the reader stopped at.
        at: usize,
    },
}

impl fmt::Display for ShieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPart { part, needed, held } => {
                write!(f, "the work needs {needed} {}, {held} at hand", part.name())
            }
            Self::Split => write!(f, "the boards are split: the shield is lost"),
            Self::NotALatinCapital { letter } => {
                write!(f, "{letter} is not a Latin capital")
            }
            Self::InscriptionFull { cap } => {
                write!(f, "the boss is full: {cap} letters")
            }
            Self::BadSave { at } => write!(f, "the save carries no shield at byte {at}"),
        }
    }
}

impl std::error::Error for ShieldError {}

/// What a mark on the drawn shield is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// The cover is cut: a tear in the linen or the calfskin.
    Tear,
    /// The boards are cut: a gash in the planks.
    Gash,
    /// The boards are split: the shield is lost.
    Split,
}

impl MarkKind {
    /// The word this mark is written as.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tear => "tear",
            Self::Gash => "gash",
            Self::Split => "split",
        }
    }
}

/// One mark on the drawn shield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// The cell of the face the mark sits in, counted over the face.
    pub cell: u32,
    /// What the mark is.
    pub kind: MarkKind,
}

/// What one blow on the shield did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The blow cut the cover.
    Cover,
    /// The blow cut the boards.
    Boards,
    /// The blow split the boards: the shield is lost.
    Split,
    /// The shield was already lost: the blow did nothing.
    Lost,
}

/// The drawn shield: what a renderer paints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawing {
    /// The height of the shield, in millimetres.
    pub height_mm: u32,
    /// The width of the shield, in millimetres.
    pub width_mm: u32,
    /// How many cells the face is cut into, across.
    pub cells_x: u32,
    /// How many cells the face is cut into, down.
    pub cells_y: u32,
    /// The marks on the face, in the order they were made.
    pub marks: Vec<Mark>,
    /// The condition of the cover, in per mille.
    pub cover: u32,
    /// The condition of the boards, in per mille.
    pub boards: u32,
    /// The boards are split: the shield is lost.
    pub split: bool,
}

impl Drawing {
    /// The size of one cell of the face, in millimetres.
    #[must_use]
    pub fn cell_mm(&self) -> (u32, u32) {
        (self.width_mm / self.cells_x, self.height_mm / self.cells_y)
    }
}

/// A shield in a man's hands: its cover, its boards and the damage it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shield {
    cover: u32,
    boards: u32,
    hits: u32,
    marks: Vec<Mark>,
    split: bool,
    inscription: Punched,
}

/// The letters punched into a shield's boss, one stroke per letter, no heap:
/// the owner's name, as on the Tyne boss of Junius Dubitatus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Punched {
    letters: [u8; MAX_PUNCH_LETTERS],
    len: u8,
}

impl Punched {
    /// No stroke yet: a shield out of the workshop carries no name.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            letters: [0; MAX_PUNCH_LETTERS],
            len: 0,
        }
    }

    /// How many strokes are punched in.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// No stroke is punched in.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The letters as text, in the order they were punched.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Only Latin capitals and the space are written, so the slice is UTF-8.
        core::str::from_utf8(&self.letters[..self.len()]).unwrap_or("")
    }

    /// Punch one letter: one stroke of the hammer.
    ///
    /// Latin capitals `A..=Z` and the space take. The space is a stroke of its
    /// own, as the word gap in a boss name was punched like a letter.
    ///
    /// # Errors
    ///
    /// Returns [`ShieldError::NotALatinCapital`] for any other byte and
    /// [`ShieldError::InscriptionFull`] once [`MAX_PUNCH_LETTERS`] strokes are
    /// in. A refused stroke leaves the boss as it was.
    pub fn punch(&mut self, letter: u8) -> Result<(), ShieldError> {
        if !(letter.is_ascii_uppercase() || letter == b' ') {
            return Err(ShieldError::NotALatinCapital { letter });
        }
        let len = usize::from(self.len);
        if len >= MAX_PUNCH_LETTERS {
            return Err(ShieldError::InscriptionFull {
                cap: MAX_PUNCH_LETTERS,
            });
        }
        self.letters[len] = letter;
        self.len = u8::try_from(len + 1).unwrap_or(u8::MAX);
        Ok(())
    }
}

/// Take `count` bytes at the cursor, moving it; a cut-short image refuses.
fn take<'b>(bytes: &'b [u8], at: &mut usize, count: usize) -> Result<&'b [u8], ShieldError> {
    let end = at
        .checked_add(count)
        .ok_or(ShieldError::BadSave { at: *at })?;
    let slice = bytes
        .get(*at..end)
        .ok_or(ShieldError::BadSave { at: *at })?;
    *at = end;
    Ok(slice)
}

/// One little-endian `u32` field at the cursor.
fn word(bytes: &[u8], at: &mut usize) -> Result<u32, ShieldError> {
    let field = take(bytes, at, 4)?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(field);
    Ok(u32::from_le_bytes(buf))
}

/// One byte field at the cursor.
fn byte(bytes: &[u8], at: &mut usize) -> Result<u8, ShieldError> {
    Ok(take(bytes, at, 1)?[0])
}

impl Shield {
    /// Build a shield from the parts on hand.
    ///
    /// The parts are taken off the pile: a build that is short of a material
    /// takes nothing and is refused by the name of the material.
    ///
    /// # Errors
    ///
    /// Returns [`ShieldError::MissingPart`] when the pile is short of a
    /// material the build needs.
    pub fn build(spec: &Scutum<'_>, parts: &mut Parts) -> Result<Self, ShieldError> {
        check(parts, Part::Planks, spec.build.plank_layers)?;
        check(parts, Part::Linen, spec.build.linen_layers)?;
        check(parts, Part::Calfskin, spec.build.calfskin_layers)?;
        check(parts, Part::Rim, spec.rim.strips)?;
        parts.take(Part::Planks, spec.build.plank_layers);
        parts.take(Part::Linen, spec.build.linen_layers);
        parts.take(Part::Calfskin, spec.build.calfskin_layers);
        parts.take(Part::Rim, spec.rim.strips);
        Ok(Self {
            cover: WHOLE,
            boards: WHOLE,
            hits: 0,
            marks: Vec::new(),
            split: false,
            inscription: Punched::empty(),
        })
    }

    /// Put a new cover on the shield.
    ///
    /// A new cover is whole, so the tears in the old one are gone. The boards
    /// keep their wear. A shield whose boards are split is lost: no cover
    /// saves it.
    ///
    /// # Errors
    ///
    /// Returns [`ShieldError::Split`] when the boards are split, and
    /// [`ShieldError::MissingPart`] when the pile is short of linen or of
    /// calfskin.
    pub fn recover(&mut self, spec: &Scutum<'_>, parts: &mut Parts) -> Result<(), ShieldError> {
        if self.split {
            return Err(ShieldError::Split);
        }
        let linen = spec.build.linen_layers;
        let calfskin = spec.build.calfskin_layers;
        check(parts, Part::Linen, linen)?;
        check(parts, Part::Calfskin, calfskin)?;
        parts.take(Part::Linen, linen);
        parts.take(Part::Calfskin, calfskin);
        self.cover = WHOLE;
        self.marks.retain(|mark| mark.kind != MarkKind::Tear);
        Ok(())
    }

    /// Take one blow on the shield.
    ///
    /// A blow cuts the cover while the cover lasts. When the cover is gone the
    /// blow reaches the boards, and the blow that passes their last strength
    /// splits them: the shield is lost.
    pub fn hit(&mut self, spec: &Scutum<'_>) -> Hit {
        if self.split {
            return Hit::Lost;
        }
        let cell = self.hits % spec.cells();
        self.hits += 1;
        if self.cover > 0 {
            self.cover = self.cover.saturating_sub(spec.cover_wear);
            self.marks.push(Mark {
                cell,
                kind: MarkKind::Tear,
            });
            return Hit::Cover;
        }
        if self.boards == 0 {
            self.split = true;
            self.marks.push(Mark {
                cell,
                kind: MarkKind::Split,
            });
            return Hit::Split;
        }
        self.boards = self.boards.saturating_sub(spec.board_wear);
        self.marks.push(Mark {
            cell,
            kind: MarkKind::Gash,
        });
        Hit::Boards
    }

    /// The condition of the cover, in per mille.
    #[must_use]
    pub fn cover(&self) -> u32 {
        self.cover
    }

    /// The condition of the boards, in per mille.
    #[must_use]
    pub fn boards(&self) -> u32 {
        self.boards
    }

    /// How many blows the shield has taken.
    #[must_use]
    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// The boards are split: the shield is lost.
    #[must_use]
    pub fn is_split(&self) -> bool {
        self.split
    }

    /// The marks the shield carries, in the order they were made.
    #[must_use]
    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Punch one letter into the boss: one stroke per letter.
    ///
    /// # Errors
    ///
    /// As [`Punched::punch`]: a byte outside the Latin capitals and the space,
    /// or a stroke past [`MAX_PUNCH_LETTERS`].
    pub fn punch(&mut self, letter: u8) -> Result<(), ShieldError> {
        self.inscription.punch(letter)
    }

    /// The owner's name on the boss, in the order the strokes were made. The
    /// look draws it; this module holds it.
    #[must_use]
    pub fn inscription(&self) -> &str {
        self.inscription.as_str()
    }

    /// The shield as save bytes: magic, version, the wear and the hits, the
    /// split, the punched letters, then every mark. The same shield writes the
    /// same bytes, and [`Shield::from_bytes`] gives the shield back.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&SAVE_MAGIC);
        out.extend_from_slice(&SAVE_VERSION.to_le_bytes());
        out.extend_from_slice(&self.cover.to_le_bytes());
        out.extend_from_slice(&self.boards.to_le_bytes());
        out.extend_from_slice(&self.hits.to_le_bytes());
        out.push(u8::from(self.split));
        out.push(self.inscription.len);
        out.extend_from_slice(&self.inscription.letters[..self.inscription.len()]);
        let count = u32::try_from(self.marks.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&count.to_le_bytes());
        for mark in &self.marks {
            out.extend_from_slice(&mark.cell.to_le_bytes());
            out.push(match mark.kind {
                MarkKind::Tear => 0,
                MarkKind::Gash => 1,
                MarkKind::Split => 2,
            });
        }
        out
    }

    /// Read a shield back from [`Shield::to_bytes`].
    ///
    /// # Errors
    ///
    /// Returns [`ShieldError::BadSave`] when the bytes are no shield: a wrong
    /// magic or version, a field cut short, a stroke the punch would refuse, a
    /// mark kind it does not know, or bytes left over at the end.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ShieldError> {
        let mut at = 0usize;
        if take(bytes, &mut at, SAVE_MAGIC.len())? != SAVE_MAGIC {
            return Err(ShieldError::BadSave { at: 0 });
        }
        if take(bytes, &mut at, 2)? != SAVE_VERSION.to_le_bytes() {
            return Err(ShieldError::BadSave { at: 4 });
        }
        let cover = word(bytes, &mut at)?;
        let boards = word(bytes, &mut at)?;
        let hits = word(bytes, &mut at)?;
        let split = byte(bytes, &mut at)? != 0;
        let len = usize::from(byte(bytes, &mut at)?);
        if len > MAX_PUNCH_LETTERS {
            return Err(ShieldError::BadSave { at: at - 1 });
        }
        let mut inscription = Punched::empty();
        let strokes = at;
        for letter in take(bytes, &mut at, len)? {
            if inscription.punch(*letter).is_err() {
                return Err(ShieldError::BadSave { at: strokes });
            }
        }
        let count = word(bytes, &mut at)?;
        let mut marks = Vec::new();
        for _ in 0..count {
            let cell = word(bytes, &mut at)?;
            let kind = match byte(bytes, &mut at)? {
                0 => MarkKind::Tear,
                1 => MarkKind::Gash,
                2 => MarkKind::Split,
                _ => return Err(ShieldError::BadSave { at: at - 1 }),
            };
            marks.push(Mark { cell, kind });
        }
        if at != bytes.len() {
            return Err(ShieldError::BadSave { at });
        }
        Ok(Self {
            cover,
            boards,
            hits,
            marks,
            split,
            inscription,
        })
    }

    /// The drawn shield: the marks a renderer paints.
    #[must_use]
    pub fn drawing(&self, spec: &Scutum<'_>) -> Drawing {
        Drawing {
            height_mm: spec.height.mm,
            width_mm: spec.width.mm,
            cells_x: spec.cells_x,
            cells_y: spec.cells_y,
            marks: self.marks.clone(),
            cover: self.cover,
            boards: self.boards,
            split: self.split,
        }
    }

    /// A digest of the shield: the same blows give the same digest.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(b"engine-rome/scutum/shield/1");
        hasher.write_u32(self.cover);
        hasher.write_u32(self.boards);
        hasher.write_u32(self.hits);
        hasher.write_bool(self.split);
        hasher.write_len(self.inscription.len());
        hasher.write_str(self.inscription.as_str());
        for mark in &self.marks {
            hasher.write_u32(mark.cell);
            hasher.write_len(mark.kind.as_str().len());
            hasher.write_str(mark.kind.as_str());
        }
        hasher.finish()
    }
}

/// The build section: the plank layers and the cover.
fn build<'a>(table: &Table<'a>) -> Result<Build<'a>, LoadError<'a>> {
    let entry = table
        .entry("build")
        .ok_or(LoadError::MissingEntry { entry: "build" })?;
    only(
        entry,
        &["plank_layers", "joint", "linen_layers", "calfskin_layers"],
    )?;
    let joint = match entry.text("joint") {
        Some(word) => Joint::parse(word.trim()).ok_or_else(|| LoadError::BadValue {
            entry: entry.key(),
            key: "joint",
            value: word,
        })?,
        None => {
            return Err(LoadError::MissingKey {
                entry: entry.key(),
                key: "joint",
            })
        }
    };
    Ok(Build {
        plank_layers: count(entry, "plank_layers")?,
        joint,
        linen_layers: count(entry, "linen_layers")?,
        calfskin_layers: count(entry, "calfskin_layers")?,
        source: entry.source(),
    })
}

/// The rim section: the strips and what they are made of.
fn rim<'a>(table: &Table<'a>) -> Result<Rim<'a>, LoadError<'a>> {
    let entry = table
        .entry("rim")
        .ok_or(LoadError::MissingEntry { entry: "rim" })?;
    only(entry, &["strips", "material"])?;
    let material = match entry.text("material") {
        Some(word) => RimMaterial::parse(word.trim()).ok_or_else(|| LoadError::BadValue {
            entry: entry.key(),
            key: "material",
            value: word,
        })?,
        None => {
            return Err(LoadError::MissingKey {
                entry: entry.key(),
                key: "material",
            })
        }
    };
    Ok(Rim {
        strips: count(entry, "strips")?,
        material,
        source: entry.source(),
    })
}

/// One required length in millimetres, with the source that gave it.
fn measure<'a>(entry: &Entry<'a>, key: &'a str) -> Result<Measure<'a>, LoadError<'a>> {
    let mm = int(entry, key)?;
    let mm = u32::try_from(mm).map_err(|_| LoadError::NumberOverflow {
        entry: entry.key(),
        key,
    })?;
    Ok(Measure {
        mm,
        source: entry.source(),
    })
}

/// One required whole number from a section, above zero.
fn count<'a>(entry: &Entry<'a>, key: &'a str) -> Result<u32, LoadError<'a>> {
    let number = int(entry, key)?;
    let number = u32::try_from(number).map_err(|_| LoadError::NumberOverflow {
        entry: entry.key(),
        key,
    })?;
    if number == 0 {
        return Err(LoadError::Zero {
            entry: entry.key(),
            key,
        });
    }
    Ok(number)
}

/// Refuse a job that is short of a material.
fn check(parts: &Parts, part: Part, needed: u32) -> Result<(), ShieldError> {
    let held = parts.amount(part);
    if held < needed {
        return Err(ShieldError::MissingPart { part, needed, held });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped table, read once.
    fn spec() -> Scutum<'static> {
        Scutum::shipped().expect("shipped scutum table")
    }

    /// A fresh shield with the parts a build needs.
    fn shield(spec: &Scutum<'_>) -> (Shield, Parts) {
        let mut parts = Parts::for_build(spec);
        let shield = Shield::build(spec, &mut parts).expect("a build with the right parts");
        (shield, parts)
    }

    #[test]
    fn the_table_reads_the_build_of_polybius() {
        let scutum = spec();
        assert_eq!(scutum.build.plank_layers, 2, "two plank layers");
        assert_eq!(scutum.build.joint, Joint::Glued, "glued together");
        assert_eq!(scutum.build.linen_layers, 1, "one linen layer");
        assert_eq!(scutum.build.calfskin_layers, 1, "one calfskin layer");
        assert_eq!(scutum.rim.strips, 1, "one rim");
        assert_eq!(scutum.rim.material, RimMaterial::IronOrBronze);
        assert!(
            scutum.build.source.contains("Polybius 6.23"),
            "{}",
            scutum.build.source
        );
        assert!(
            scutum.rim.source.contains("Polybius 6.23"),
            "{}",
            scutum.rim.source
        );
    }

    #[test]
    fn a_build_takes_the_parts_it_needs() {
        let scutum = spec();
        let mut parts = Parts::for_build(&scutum);
        let built = Shield::build(&scutum, &mut parts).expect("a build with the right parts");
        assert_eq!(built.cover(), 1000, "the cover is whole");
        assert_eq!(built.boards(), 1000, "the boards are whole");
        assert_eq!(built.hits(), 0, "no blow has fallen on it");
        assert_eq!(parts.amount(Part::Planks), 0, "the planks are used");
        assert_eq!(parts.amount(Part::Linen), 0, "the linen is used");
        assert_eq!(parts.amount(Part::Calfskin), 0, "the calfskin is used");
        assert_eq!(parts.amount(Part::Rim), 0, "the rim is used");
    }

    #[test]
    fn a_build_without_a_material_is_refused_and_names_it() {
        let scutum = spec();
        let mut parts = Parts::for_build(&scutum);
        parts.linen = 0;
        let error = Shield::build(&scutum, &mut parts).expect_err("no linen");
        assert_eq!(
            error,
            ShieldError::MissingPart {
                part: Part::Linen,
                needed: 1,
                held: 0
            }
        );
        assert!(error.to_string().contains("linen"), "{error}");
        assert_eq!(
            parts.amount(Part::Planks),
            2,
            "a refused build takes nothing"
        );
    }

    #[test]
    fn a_shield_takes_its_hit_budget_before_the_boards_split() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        assert_eq!(
            scutum.hit_budget(),
            6,
            "four blows on the cover, two on the boards"
        );
        for _ in 0..scutum.hit_budget() {
            shield.hit(&scutum);
            assert!(!shield.is_split(), "the boards hold through the budget");
        }
        assert_eq!(shield.hit(&scutum), Hit::Split, "the next blow splits them");
        assert!(shield.is_split(), "the shield is lost");
        assert_eq!(
            shield.hit(&scutum),
            Hit::Lost,
            "a lost shield takes no more"
        );
    }

    #[test]
    fn a_blow_wears_the_cover_first_and_the_boards_after() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        assert_eq!(shield.hit(&scutum), Hit::Cover, "the cover is cut");
        assert_eq!(shield.cover(), 750, "a quarter of the cover is gone");
        assert_eq!(shield.boards(), 1000, "the boards are untouched");
        for _ in 0..3 {
            assert_eq!(shield.hit(&scutum), Hit::Cover, "the cover is cut");
        }
        assert_eq!(shield.cover(), 0, "the cover is gone");
        assert_eq!(
            shield.hit(&scutum),
            Hit::Boards,
            "the blow reaches the boards"
        );
        assert_eq!(shield.boards(), 500, "half the boards are gone");
        assert_eq!(shield.cover(), 0, "the cover stays gone");
    }

    #[test]
    fn a_re_cover_restores_the_shield() {
        let scutum = spec();
        let (mut shield, mut parts) = shield(&scutum);
        for _ in 0..4 {
            shield.hit(&scutum);
        }
        assert_eq!(shield.cover(), 0, "the cover is gone");
        assert_eq!(shield.marks().len(), 4, "four tears in the old cover");
        parts.add(Part::Linen, 1);
        parts.add(Part::Calfskin, 1);
        shield
            .recover(&scutum, &mut parts)
            .expect("a shield whose boards hold");
        assert_eq!(shield.cover(), 1000, "the new cover is whole");
        assert_eq!(shield.marks().len(), 0, "the tears are gone");
        assert_eq!(
            shield.hit(&scutum),
            Hit::Cover,
            "the blow cuts the new cover"
        );
        assert_eq!(shield.cover(), 750);
    }

    #[test]
    fn a_re_cover_needs_its_linen_and_its_calfskin() {
        let scutum = spec();
        let (mut shield, mut parts) = shield(&scutum);
        parts.add(Part::Linen, 1);
        let error = shield
            .recover(&scutum, &mut parts)
            .expect_err("no calfskin");
        assert_eq!(
            error,
            ShieldError::MissingPart {
                part: Part::Calfskin,
                needed: 1,
                held: 0
            }
        );
        assert!(error.to_string().contains("calfskin"), "{error}");
    }

    #[test]
    fn a_split_shield_cannot_be_re_covered() {
        let scutum = spec();
        let (mut shield, mut parts) = shield(&scutum);
        for _ in 0..=scutum.hit_budget() {
            shield.hit(&scutum);
        }
        assert!(shield.is_split(), "the boards are split");
        parts.add(Part::Linen, 1);
        parts.add(Part::Calfskin, 1);
        let error = shield
            .recover(&scutum, &mut parts)
            .expect_err("a lost shield");
        assert_eq!(error, ShieldError::Split);
        assert!(error.to_string().contains("split"), "{error}");
    }

    #[test]
    fn the_drawn_shield_shows_the_damage() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        shield.hit(&scutum);
        shield.hit(&scutum);
        let drawing = shield.drawing(&scutum);
        assert_eq!(drawing.marks.len(), 2, "two tears");
        assert_eq!(drawing.marks[0].kind, MarkKind::Tear);
        assert_eq!(drawing.marks[0].cell, 0, "the first cell of the face");
        assert_eq!(drawing.marks[1].cell, 1, "the next cell of the face");
        assert_eq!(drawing.cover, 500, "the cover shows its wear");
        assert_eq!(drawing.boards, 1000, "the boards are whole");
        assert!(!drawing.split, "the shield is not lost");
        assert_eq!(drawing.height_mm, 1200);
        assert_eq!(drawing.width_mm, 750);
        assert_eq!(drawing.cell_mm(), (250, 300), "a cell of the face");
        for _ in 0..5 {
            shield.hit(&scutum);
        }
        let drawing = shield.drawing(&scutum);
        let kinds: Vec<MarkKind> = drawing.marks.iter().map(|mark| mark.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MarkKind::Tear,
                MarkKind::Tear,
                MarkKind::Tear,
                MarkKind::Tear,
                MarkKind::Gash,
                MarkKind::Gash,
                MarkKind::Split,
            ],
            "the face shows the tears, the gashes and the split"
        );
        assert!(drawing.split, "the shield is lost");
    }

    #[test]
    fn the_same_blows_give_the_same_shield() {
        let scutum = spec();
        let (mut first, _parts) = shield(&scutum);
        let (mut second, _parts) = shield(&scutum);
        for _ in 0..5 {
            first.hit(&scutum);
            second.hit(&scutum);
        }
        assert_eq!(
            first.hash(),
            second.hash(),
            "the same blows, the same digest"
        );
        assert_eq!(first.drawing(&scutum), second.drawing(&scutum));
        assert_eq!(
            scutum.hash(),
            spec().hash(),
            "the same table, the same digest"
        );
    }

    #[test]
    fn a_number_with_no_ref_is_refused() {
        let text = "[shield]\nheight_mm = 1200\nwidth_mm = 750\ncells_x = 3\ncells_y = 4\n";
        let error = Scutum::parse(text).expect_err("a number with no source");
        assert!(error.to_string().contains("shield"), "{error}");
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let text = Scutum::SHIPPED.replace("plank_layers = 2", "plank_layer = 2");
        let error = Scutum::parse(&text).expect_err("a key the crate does not read");
        assert!(error.to_string().contains("plank_layer"), "{error}");
    }

    /// The owner's name on the Tyne boss of Junius Dubitatus, one stroke per letter.
    const TYNE_BOSS: &str = "IVL DVBITATI";

    #[test]
    fn punch_strokes_spell_the_tyne_boss_name() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        assert_eq!(shield.inscription(), "", "a new shield carries no mark");
        assert_eq!(
            TYNE_BOSS.len(),
            12,
            "the boss name is 12 strokes, the space one of them"
        );
        for (stroke, letter) in TYNE_BOSS.bytes().enumerate() {
            assert!(
                shield.punch(letter).is_ok(),
                "stroke {stroke} ({letter}) must take"
            );
        }
        assert_eq!(
            shield.inscription(),
            TYNE_BOSS,
            "the strokes spell the name"
        );
    }

    #[test]
    fn punch_refuses_a_letter_outside_latin_capitals() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        for letter in [b'i', b'1', b'.', 0xC3, b'@', b'\n'] {
            assert!(
                matches!(
                    shield.punch(letter),
                    Err(ShieldError::NotALatinCapital { letter: seen }) if seen == letter
                ),
                "{letter} is outside the Latin capitals"
            );
        }
        assert_eq!(shield.inscription(), "", "a refused stroke punches nothing");
        shield.punch(b'A').expect("a Latin capital takes");
        assert_eq!(shield.inscription(), "A", "the refusals were the letters");
    }

    #[test]
    fn punch_refuses_the_twenty_fifth_letter() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        for _ in 0..MAX_PUNCH_LETTERS {
            shield.punch(b'I').expect("the cap of letters fits");
        }
        assert_eq!(shield.inscription().len(), MAX_PUNCH_LETTERS, "24 strokes");
        assert!(
            matches!(
                shield.punch(b'I'),
                Err(ShieldError::InscriptionFull { cap }) if cap == MAX_PUNCH_LETTERS
            ),
            "the 25th stroke is refused by the cap"
        );
        assert_eq!(
            shield.inscription().len(),
            MAX_PUNCH_LETTERS,
            "the refused stroke punched nothing"
        );
    }

    #[test]
    fn punch_save_round_trips_byte_equal() {
        let scutum = spec();
        let (mut shield, _parts) = shield(&scutum);
        for letter in TYNE_BOSS.bytes() {
            shield.punch(letter).expect("the name takes");
        }
        shield.hit(&scutum);
        shield.hit(&scutum);
        let bytes = shield.to_bytes();
        let loaded = Shield::from_bytes(&bytes).expect("the save reads back");
        assert_eq!(
            loaded, shield,
            "the save keeps the wear, the marks and the name"
        );
        assert_eq!(loaded.inscription(), TYNE_BOSS, "the name rides the save");
        assert_eq!(loaded.to_bytes(), bytes, "the save round-trips byte equal");
        assert_eq!(loaded.hash(), shield.hash(), "the digest rides the save");
        assert!(
            Shield::from_bytes(&bytes[..bytes.len() - 1]).is_err(),
            "a truncated save is refused"
        );
    }
}
