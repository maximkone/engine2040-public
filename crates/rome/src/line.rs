//! line: the battle line's frontage per man and the gap between ranks.

use crate::table::Table;
use crate::{digest, length, mm_per_foot, only, LoadError, Measure};

/// One battle line as the drill books give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    /// One man's share of the front, in millimetres.
    pub frontage: Measure<'a>,
    /// The gap between one rank and the next, in millimetres.
    pub rank_gap: Measure<'a>,
}

impl Line<'static> {
    /// The line table shipped in `content/rome/line.toml`.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/line.toml");

    /// Read the shipped line table.
    ///
    /// # Errors
    ///
    /// Same as [`Line::parse`]; a test keeps the shipped table valid.
    pub fn shipped() -> Result<Self, LoadError<'static>> {
        Self::parse(Self::SHIPPED)
    }
}

impl<'a> Line<'a> {
    /// Read one line table into millimetres.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError::Table`] when the text is not a well-formed table or an entry
    /// has a number and no `ref`, and the typed variants when a section or key is
    /// missing, unknown, zero or too large for a millimetre length.
    pub fn parse(text: &'a str) -> Result<Self, LoadError<'a>> {
        let table = Table::parse(text)?;
        let mm_per_ft = mm_per_foot(&table)?;
        let entry = table
            .entry("line")
            .ok_or(LoadError::MissingEntry { entry: "line" })?;
        only(entry, &["frontage_ft", "rank_gap_ft"])?;
        Ok(Self {
            frontage: length(entry, "frontage_ft", mm_per_ft)?,
            rank_gap: length(entry, "rank_gap_ft", mm_per_ft)?,
        })
    }

    /// A digest of the loaded millimetre numbers: two loads of one table agree.
    #[must_use]
    pub fn hash(&self) -> u64 {
        digest(
            b"engine-rome/line/1",
            [self.frontage, self.rank_gap].into_iter(),
        )
    }
}
