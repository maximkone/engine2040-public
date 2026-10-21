//! camp: the ditch, the rampart and the street widths a marching camp is laid out with.

use crate::table::Table;
use crate::{digest, length, mm_per_foot, only, LoadError, Measure};

/// How much trouble the camp expects before morning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Threat {
    /// One night on the march: the smallest works the sources give.
    OneNight,
    /// A light threat: a deeper ditch, the same rampart.
    Light,
    /// A serious threat: the deepest ditch the sources give.
    Serious,
}

impl Threat {
    /// Every threat, in table order.
    pub const ALL: [Self; 3] = [Self::OneNight, Self::Light, Self::Serious];

    /// The table entry this threat reads, for example `ditch.one_night`.
    #[must_use]
    pub const fn entry(self) -> &'static str {
        match self {
            Self::OneNight => "ditch.one_night",
            Self::Light => "ditch.light",
            Self::Serious => "ditch.serious",
        }
    }

    /// Position in the camp's ditch list.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::OneNight => 0,
            Self::Light => 1,
            Self::Serious => 2,
        }
    }
}

/// A ditch: how wide it opens and how deep the century digs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ditch<'a> {
    /// Width at the top, in millimetres.
    pub width: Measure<'a>,
    /// Depth from the ground, in millimetres.
    pub depth: Measure<'a>,
}

/// The turf rampart raised behind the ditch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rampart<'a> {
    /// Width at the base, in millimetres.
    pub width: Measure<'a>,
    /// Height above the ground, in millimetres.
    pub height: Measure<'a>,
}

/// The camp streets that stay clear of tents and packs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Streets<'a> {
    /// The via principalis, the main street across the camp.
    pub via_principalis: Measure<'a>,
    /// The via quintana, the second street behind it.
    pub via_quintana: Measure<'a>,
    /// The via sagularis, the broad street down the middle of the camp.
    pub via_sagularis: Measure<'a>,
    /// The via sagularis with five legions under it, which is wider.
    pub via_sagularis_five_legions: Measure<'a>,
}

/// The camp works one table describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camp<'a> {
    ditches: [Ditch<'a>; Threat::ALL.len()],
    /// The rampart every camp of any threat raises.
    pub rampart: Rampart<'a>,
    /// The streets every camp keeps clear.
    pub streets: Streets<'a>,
    /// The clear ground left outside the ditch at the gates.
    pub gate_clear: Measure<'a>,
    /// The table's own millimetres to a Roman foot.
    foot: u32,
}

impl Camp<'static> {
    /// The camp table shipped in `content/rome/camp.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/camp.toml");

    /// Read the shipped camp table.
    ///
    /// # Errors
    ///
    /// Same as [`Camp::parse`]; a test keeps the shipped table valid, so this doubles as
    /// a build check on the content file.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Camp<'a> {
    /// Read one camp table into millimetres.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or an entry
    /// has a number and no `ref`, and the typed variants when a section or key is
    /// missing, unknown, zero or too large for a millimetre length.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let mm_per_ft = mm_per_foot(&table)?;
        let ditches = [
            ditch(&table, Threat::OneNight, mm_per_ft)?,
            ditch(&table, Threat::Light, mm_per_ft)?,
            ditch(&table, Threat::Serious, mm_per_ft)?,
        ];
        Ok(Self {
            ditches,
            rampart: rampart(&table, mm_per_ft)?,
            streets: streets(&table, mm_per_ft)?,
            gate_clear: gate_clear(&table, mm_per_ft)?,
            foot: mm_per_ft,
        })
    }

    /// The table's own millimetres to a Roman foot: every length turns through it.
    #[must_use]
    pub fn foot(&self) -> u32 {
        self.foot
    }

    /// The ditch this threat's century digs.
    #[must_use]
    pub fn ditch(&self, threat: Threat) -> Ditch<'a> {
        self.ditches[threat.index()]
    }

    /// A digest of the loaded millimetre numbers: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let [one_night, light, serious] = self.ditches;
        let lengths = [
            one_night.width,
            one_night.depth,
            light.width,
            light.depth,
            serious.width,
            serious.depth,
            self.rampart.width,
            self.rampart.height,
            self.streets.via_principalis,
            self.streets.via_quintana,
            self.streets.via_sagularis,
            self.streets.via_sagularis_five_legions,
            self.gate_clear,
        ];
        digest(b"engine-rome/camp/1", lengths.into_iter())
    }
}

/// One threat's ditch entry.
fn ditch<'a>(
    table: &Table<'a>,
    threat: Threat,
    mm_per_ft: u32,
) -> Result<Ditch<'a>, LoadError<'a>> {
    let entry = table
        .entry(threat.entry())
        .ok_or_else(|| LoadError::MissingEntry {
            entry: threat.entry(),
        })?;
    only(entry, &["width_ft", "depth_ft"])?;
    Ok(Ditch {
        width: length(entry, "width_ft", mm_per_ft)?,
        depth: length(entry, "depth_ft", mm_per_ft)?,
    })
}

/// The rampart entry.
fn rampart<'a>(table: &Table<'a>, mm_per_ft: u32) -> Result<Rampart<'a>, LoadError<'a>> {
    let entry = table
        .entry("rampart")
        .ok_or(LoadError::MissingEntry { entry: "rampart" })?;
    only(entry, &["width_ft", "height_ft"])?;
    Ok(Rampart {
        width: length(entry, "width_ft", mm_per_ft)?,
        height: length(entry, "height_ft", mm_per_ft)?,
    })
}

/// The street entry.
fn streets<'a>(table: &Table<'a>, mm_per_ft: u32) -> Result<Streets<'a>, LoadError<'a>> {
    let entry = table
        .entry("streets")
        .ok_or(LoadError::MissingEntry { entry: "streets" })?;
    only(entry, &["via_principalis_ft", "via_quintana_ft"])?;
    let sagularis = table
        .entry("streets.sagularis")
        .ok_or(LoadError::MissingEntry {
            entry: "streets.sagularis",
        })?;
    only(sagularis, &["width_ft", "width_five_legions_ft"])?;
    Ok(Streets {
        via_principalis: length(entry, "via_principalis_ft", mm_per_ft)?,
        via_quintana: length(entry, "via_quintana_ft", mm_per_ft)?,
        via_sagularis: length(sagularis, "width_ft", mm_per_ft)?,
        via_sagularis_five_legions: length(sagularis, "width_five_legions_ft", mm_per_ft)?,
    })
}

/// The clear ground outside the ditch at the gates.
fn gate_clear<'a>(table: &Table<'a>, mm_per_ft: u32) -> Result<Measure<'a>, LoadError<'a>> {
    let entry = table
        .entry("gates")
        .ok_or(LoadError::MissingEntry { entry: "gates" })?;
    only(entry, &["clear_space_ft"])?;
    length(entry, "clear_space_ft", mm_per_ft)
}
