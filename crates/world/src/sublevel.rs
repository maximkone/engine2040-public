//! sublevel: named groups of streamed cells, and which groups a mode keeps resident.
//!
//! A cell streams by coordinate ([`crate::cells`]); a sublevel is the author's
//! unit above it: `dungeon` owns a rectangle of cells, `overworld` owns the rest,
//! and a mode names the sublevels it wants loaded. [`Sublevels::resident`]
//! answers the cells to admit, sorted by coordinate, so two runs of the same
//! mode ask the loader for the same list.
//!
//! Owner: `engine-world`. Consumer: a loader that turns the list into
//! [`crate::cells::CellCoord`] requests; until one names it the tests below walk
//! the answers by hand. Units: cell coordinates, inclusive rectangle edges.
//! Determinism: a pure parse and a pure rectangle walk, no IO, no clock, sorted
//! output. Failure: a refused table names the line, the sublevel or the cell;
//! a wanted name the table does not carry is refused by name. Budget: at most
//! [`MAX_SUBLEVELS`] rectangles and [`MAX_CELLS`] cells in the answer.

use std::fmt;

use crate::cells::CellCoord;

/// Most sublevels one table may hold.
pub const MAX_SUBLEVELS: usize = 32;
/// Most cells one sublevel may own.
pub const MAX_CELLS: usize = 1_024;

/// An inclusive rectangle of cells: `min_x..=max_x` by `min_z..=max_z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSpan {
    /// Lowest cell x in the span.
    pub min_x: i32,
    /// Lowest cell z in the span.
    pub min_z: i32,
    /// Highest cell x in the span.
    pub max_x: i32,
    /// Highest cell z in the span.
    pub max_z: i32,
}

impl CellSpan {
    /// Cells the span holds when its edges are ordered, `None` when a low edge
    /// sits past its high edge.
    #[must_use]
    pub fn cells(self) -> Option<usize> {
        if self.min_x > self.max_x || self.min_z > self.max_z {
            return None;
        }
        let width = (self.max_x - self.min_x).checked_add(1)?;
        let depth = (self.max_z - self.min_z).checked_add(1)?;
        usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(depth).ok()?)
    }

    /// Whether the span holds this cell.
    #[must_use]
    pub fn holds(self, x: i32, z: i32) -> bool {
        (self.min_x..=self.max_x).contains(&x) && (self.min_z..=self.max_z).contains(&z)
    }

    /// The first cell two spans both hold, in `x` then `z` order.
    fn shared_cell(self, other: Self) -> Option<(i32, i32)> {
        let x0 = self.min_x.max(other.min_x);
        let x1 = self.max_x.min(other.max_x);
        let z0 = self.min_z.max(other.min_z);
        let z1 = self.max_z.min(other.max_z);
        if x0 > x1 || z0 > z1 {
            return None;
        }
        Some((x0, z0))
    }
}

/// One named group of cells. An `always` sublevel is resident in every mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sublevel {
    name: String,
    span: CellSpan,
    always: bool,
}

/// A whole sublevel table, in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sublevels {
    sublevels: Vec<Sublevel>,
}

/// Why a sublevel table, or a residency query, was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SublevelError {
    /// A row is not `sublevel <name> <min_x> <min_z> <max_x> <max_z> [always]`.
    BadRow {
        /// 1-based line of the row.
        line: usize,
        /// What the row should have been.
        reason: &'static str,
    },
    /// A coordinate does not parse.
    BadNumber {
        /// 1-based line of the row.
        line: usize,
        /// The text that did not parse.
        value: String,
    },
    /// The same sublevel name is listed twice.
    DuplicateName {
        /// 1-based line of the second row.
        line: usize,
        /// The name listed twice.
        name: String,
    },
    /// A span's low edge sits past its high edge.
    EmptySpan {
        /// The sublevel with the empty span.
        name: String,
    },
    /// A span holds more than [`MAX_CELLS`] cells.
    TooManyCells {
        /// The sublevel that is too big.
        name: String,
        /// How many cells it asks for.
        cells: usize,
    },
    /// An edge is past [`crate::cells::MAX_COORD`].
    OutOfBounds {
        /// The sublevel with the bad edge.
        name: String,
        /// The edge as written.
        edge: i32,
    },
    /// Two sublevels both hold a cell, so a loader could admit it twice.
    Overlap {
        /// The sublevel declared first.
        first: String,
        /// The sublevel that repeats a cell.
        second: String,
        /// The first shared cell, x.
        x: i32,
        /// The first shared cell, z.
        z: i32,
    },
    /// A file with no sublevel in it.
    NoSublevels,
    /// More sublevels than [`MAX_SUBLEVELS`].
    TooMany {
        /// How many the file holds.
        count: usize,
    },
    /// A mode asked for a name the table does not carry.
    UnknownSublevel {
        /// The name asked for.
        name: String,
    },
    /// A cell coordinate in the answer left the cell range.
    BadCell {
        /// The x that was refused.
        x: i32,
        /// The z that was refused.
        z: i32,
    },
}

impl fmt::Display for SublevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRow { line, reason } => write!(f, "line {line}: {reason}"),
            Self::BadNumber { line, value } => {
                write!(f, "line {line}: `{value}` is not a whole coordinate")
            }
            Self::DuplicateName { line, name } => {
                write!(f, "line {line}: sublevel `{name}` is listed twice")
            }
            Self::EmptySpan { name } => write!(
                f,
                "sublevel `{name}` has a low edge past its high edge; write min <= max"
            ),
            Self::TooManyCells { name, cells } => {
                write!(
                    f,
                    "sublevel `{name}` holds {cells} cells; at most {MAX_CELLS}"
                )
            }
            Self::OutOfBounds { name, edge } => write!(
                f,
                "sublevel `{name}` names cell edge {edge}; keep it within +-{}",
                crate::cells::MAX_COORD
            ),
            Self::Overlap {
                first,
                second,
                x,
                z,
            } => write!(
                f,
                "sublevel `{second}` claims cell ({x}, {z}), which `{first}` already owns"
            ),
            Self::NoSublevels => write!(f, "no sublevel row: a table needs at least one"),
            Self::TooMany { count } => {
                write!(f, "{count} sublevels; at most {MAX_SUBLEVELS}")
            }
            Self::UnknownSublevel { name } => write!(
                f,
                "`{name}` is not a sublevel this table carries; name one it lists"
            ),
            Self::BadCell { x, z } => write!(f, "cell ({x}, {z}) is out of range"),
        }
    }
}

impl std::error::Error for SublevelError {}

impl Sublevel {
    /// Sublevel name, as its row wrote it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The cells it owns.
    #[must_use]
    pub fn span(&self) -> CellSpan {
        self.span
    }

    /// True when every mode loads it.
    #[must_use]
    pub fn always(&self) -> bool {
        self.always
    }
}

/// Every cell of one span, in `x` then `z` order.
fn span_cells(span: CellSpan) -> Result<Vec<CellCoord>, SublevelError> {
    let mut cells = Vec::new();
    for x in span.min_x..=span.max_x {
        for z in span.min_z..=span.max_z {
            let coord = CellCoord::new(x, z).map_err(|_| SublevelError::BadCell { x, z })?;
            cells.push(coord);
        }
    }
    Ok(cells)
}

/// One integer edge of a `sublevel` row, refused with the row's line.
fn edge_of(line: usize, words: &mut std::str::SplitWhitespace<'_>) -> Result<i32, SublevelError> {
    let Some(value) = words.next() else {
        return Err(SublevelError::BadRow {
            line,
            reason: "sublevel needs four cell edges",
        });
    };
    value.parse().map_err(|_| SublevelError::BadNumber {
        line,
        value: value.to_owned(),
    })
}

/// One `sublevel` row on its own: shape, edge order, bounds and the cell cap.
fn parse_row(line: usize, body: &str) -> Result<Sublevel, SublevelError> {
    let shape = "expected `sublevel <name> <min_x> <min_z> <max_x> <max_z>`";
    let mut words = body.split_whitespace();
    let Some(key) = words.next() else {
        return Err(SublevelError::BadRow {
            line,
            reason: shape,
        });
    };
    if key != "sublevel" {
        return Err(SublevelError::BadRow {
            line,
            reason: shape,
        });
    }
    let Some(name) = words.next() else {
        return Err(SublevelError::BadRow {
            line,
            reason: "sublevel needs a name",
        });
    };
    let edges = [
        edge_of(line, &mut words)?,
        edge_of(line, &mut words)?,
        edge_of(line, &mut words)?,
        edge_of(line, &mut words)?,
    ];
    let always = match words.next() {
        None => false,
        Some("always") => true,
        Some(_) => {
            return Err(SublevelError::BadRow {
                line,
                reason: "the only word after the edges is `always`",
            });
        }
    };
    let span = CellSpan {
        min_x: edges[0],
        min_z: edges[1],
        max_x: edges[2],
        max_z: edges[3],
    };
    let Some(cells) = span.cells() else {
        return Err(SublevelError::EmptySpan {
            name: name.to_owned(),
        });
    };
    for edge in edges {
        if edge.abs() > crate::cells::MAX_COORD {
            return Err(SublevelError::OutOfBounds {
                name: name.to_owned(),
                edge,
            });
        }
    }
    if cells > MAX_CELLS {
        return Err(SublevelError::TooManyCells {
            name: name.to_owned(),
            cells,
        });
    }
    Ok(Sublevel {
        name: name.to_owned(),
        span,
        always,
    })
}

/// Add one row, refusing a repeated name or a cell another sublevel owns.
fn push_checked(
    sublevels: &mut Vec<Sublevel>,
    sublevel: Sublevel,
    line: usize,
) -> Result<(), SublevelError> {
    if sublevels.iter().any(|known| known.name == sublevel.name) {
        return Err(SublevelError::DuplicateName {
            line,
            name: sublevel.name,
        });
    }
    for known in sublevels.iter() {
        if let Some((x, z)) = sublevel.span.shared_cell(known.span) {
            return Err(SublevelError::Overlap {
                first: known.name.clone(),
                second: sublevel.name.clone(),
                x,
                z,
            });
        }
    }
    sublevels.push(sublevel);
    Ok(())
}

impl Sublevels {
    /// Parse a sublevel table: one `sublevel <name> <min_x> <min_z> <max_x> <max_z>
    /// [always]` row each, `#` starting a comment.
    pub fn parse(text: &str) -> Result<Self, SublevelError> {
        let mut sublevels: Vec<Sublevel> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let body = raw.split('#').next().unwrap_or(raw).trim();
            if body.is_empty() {
                continue;
            }
            let sublevel = parse_row(line, body)?;
            push_checked(&mut sublevels, sublevel, line)?;
        }
        if sublevels.is_empty() {
            return Err(SublevelError::NoSublevels);
        }
        if sublevels.len() > MAX_SUBLEVELS {
            return Err(SublevelError::TooMany {
                count: sublevels.len(),
            });
        }
        Ok(Self { sublevels })
    }

    /// The cells a mode loads: every `always` sublevel plus every named one,
    /// sorted by x then z, each cell once.
    pub fn resident(&self, wanted: &[&str]) -> Result<Vec<CellCoord>, SublevelError> {
        let mut cells: Vec<CellCoord> = Vec::new();
        for sublevel in &self.sublevels {
            let named = wanted.contains(&sublevel.name.as_str());
            if !sublevel.always && !named {
                continue;
            }
            cells.extend(span_cells(sublevel.span)?);
        }
        for name in wanted {
            if self.get(name).is_none() {
                return Err(SublevelError::UnknownSublevel {
                    name: (*name).to_owned(),
                });
            }
        }
        // `CellCoord` orders x-major then z, which is the residency order.
        cells.sort_unstable();
        Ok(cells)
    }

    /// Which sublevel owns one cell, or `None` when no row holds it.
    #[must_use]
    pub fn owner(&self, x: i32, z: i32) -> Option<&str> {
        self.sublevels
            .iter()
            .find(|sublevel| sublevel.span.holds(x, z))
            .map(|sublevel| sublevel.name.as_str())
    }

    /// The sublevel with this name, or `None`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Sublevel> {
        self.sublevels.iter().find(|sublevel| sublevel.name == name)
    }

    /// Names in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.sublevels
            .iter()
            .map(|sublevel| sublevel.name.as_str())
            .collect()
    }

    /// Sublevels in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sublevels.len()
    }

    /// True when the table holds no sublevel.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sublevels.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
# two groups: the hub is always loaded, the dungeon follows the mode
sublevel hub 0 0 1 0 always
sublevel dungeon 2 0 3 1
";

    /// F2P: no sublevel table existed before this module.
    #[test]
    fn resident_lists_always_groups_and_the_named_ones_sorted() {
        let table = Sublevels::parse(TABLE).expect("the table parses");
        assert_eq!(table.names(), ["hub", "dungeon"]);
        assert!(table.get("hub").expect("hub").always());
        assert!(!table.get("dungeon").expect("dungeon").always());
        // A mode that names nothing still loads the always group.
        let hub_only = table.resident(&[]).expect("a mode with no pick");
        assert_eq!(
            hub_only.iter().map(|c| (c.x, c.z)).collect::<Vec<_>>(),
            [(0, 0), (1, 0)]
        );
        // Naming the dungeon adds its cells; the answer is sorted by x then z.
        let both = table
            .resident(&["dungeon"])
            .expect("the dungeon is carried");
        assert_eq!(
            both.iter().map(|c| (c.x, c.z)).collect::<Vec<_>>(),
            [(0, 0), (1, 0), (2, 0), (2, 1), (3, 0), (3, 1)]
        );
        assert_eq!(both.len(), 6);
        // Which group owns a cell is a pure point query.
        assert_eq!(table.owner(0, 0), Some("hub"));
        assert_eq!(table.owner(3, 1), Some("dungeon"));
        assert_eq!(table.owner(9, 9), None);
        // A table is data: the same text builds the same table twice.
        assert_eq!(Sublevels::parse(TABLE), Ok(table));
    }

    #[test]
    fn an_unlisted_mode_name_refuses_by_name() {
        let table = Sublevels::parse(TABLE).expect("parses");
        assert_eq!(
            table.resident(&["dungeon", "crypt"]),
            Err(SublevelError::UnknownSublevel {
                name: "crypt".to_owned(),
            })
        );
    }

    #[test]
    fn refusals_name_the_line_the_group_or_the_shared_cell() {
        let cases: [(&str, SublevelError); 7] = [
            (
                "level hub 0 0 1 1\n",
                SublevelError::BadRow {
                    line: 1,
                    reason: "expected `sublevel <name> <min_x> <min_z> <max_x> <max_z>`",
                },
            ),
            (
                "sublevel hub 0 0 1 x\n",
                SublevelError::BadNumber {
                    line: 1,
                    value: "x".to_owned(),
                },
            ),
            (
                "sublevel hub 0 0 1 1\nsublevel hub 2 2 3 3\n",
                SublevelError::DuplicateName {
                    line: 2,
                    name: "hub".to_owned(),
                },
            ),
            (
                "sublevel hub 2 0 1 1\n",
                SublevelError::EmptySpan {
                    name: "hub".to_owned(),
                },
            ),
            (
                "sublevel hub 0 0 1 1\nsublevel dungeon 1 1 2 2\n",
                SublevelError::Overlap {
                    first: "hub".to_owned(),
                    second: "dungeon".to_owned(),
                    x: 1,
                    z: 1,
                },
            ),
            (
                "sublevel far 600 0 601 0\n",
                SublevelError::OutOfBounds {
                    name: "far".to_owned(),
                    edge: 600,
                },
            ),
            ("# nothing\n", SublevelError::NoSublevels),
        ];
        for (text, expected) in cases {
            assert_eq!(Sublevels::parse(text), Err(expected), "{text}");
        }
        // A span wider than the cell cap is refused with its count.
        assert_eq!(
            Sublevels::parse("sublevel wider 0 0 32 32\n"),
            Err(SublevelError::TooManyCells {
                name: "wider".to_owned(),
                cells: 1_089,
            })
        );
        // The word after the edges is `always` or nothing.
        assert_eq!(
            Sublevels::parse("sublevel hub 0 0 1 1 sometimes\n"),
            Err(SublevelError::BadRow {
                line: 1,
                reason: "the only word after the edges is `always`",
            })
        );
    }
}
