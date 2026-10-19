//! stance: the stance clash matrix as data — three bits decide who wins a
//! blade clash, and one source publishes the bits.
//! Owner: engine-combat (code and data).
//! Consumer: the arena match step reads a [`ClashVerdict`] for the two clash
//! kinds and hands it to `exchange`'s clash resolver; the world-space UI draws
//! the same matrix, because a matrix players must read has to be shown.
//! Units: none — a cell is one of three small integers and a kind is an index.
//! Determinism: pure lookup into a fixed `4x4` `u8` table, no heap and no
//! clock; the same two kinds always give the same verdict.
//! Errors: [`StanceError`] for a matrix that failed [`StanceMatrix::validate`]
//! (a cell out of range, a diagonal that is not a draw, or a pair that is not
//! antisymmetric); lookups never panic.
//! Budget: [`StanceMatrix::verdict`] is `O(1)`; the table is `Copy` and 16
//! bytes, and nothing here allocates.
//!
//! Source: `web: Blade Symphony official guide + ModDB MotY posts 4 and 7,
//! read 2026-10-19 (ideas only)`.
//!
//! Single source, on purpose: the official guide and the community Steam guide
//! contradict each other on Air vs Heavy (the guide says Air always loses its
//! tracer parries; the Steam guide says Air beats Heavy), and their Fast/
//! Balanced/Heavy triangle disagrees too. Two published matrices make a
//! mechanic unlearnable, so V1 ships the official guide's matrix and this
//! module is its only home. If the numbers ever change, they change here and
//! every caller reads the new row through [`stance_matrix_for_version`].

use crate::combo::Stance;

/// Wire version of the shipped clash matrix: 1 is the official guide's row.
pub const STANCE_MATRIX_VERSION_V1: u32 = 1;

/// How many clash kinds the matrix covers (Fast, Balanced, Heavy, Air).
const CLASH_COUNT: usize = 4;
/// Cell value: the row kind loses the clash.
const LOSE: u8 = 0;
/// Cell value: both kinds bind, nobody wins.
const DRAW: u8 = 1;
/// Cell value: the row kind wins the clash.
const WIN: u8 = 2;

/// The three stances plus the airborne kind, which is not a stance but clashes
/// like one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClashExt {
    /// Fast (reverse) stance.
    Fast,
    /// Balanced stance.
    Balanced,
    /// Heavy stance.
    Heavy,
    /// Airborne: a stance-independent kind, forced while off the ground.
    Air,
}

impl ClashExt {
    /// Canonical index in matrix order: Fast, Balanced, Heavy, Air.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Fast => 0,
            Self::Balanced => 1,
            Self::Heavy => 2,
            Self::Air => 3,
        }
    }

    /// The clash kind of a fighter: airborne forces [`ClashExt::Air`],
    /// otherwise the kind follows the stance.
    #[must_use]
    pub fn from_stance(stance: Stance, airborne: bool) -> ClashExt {
        if airborne {
            return Self::Air;
        }
        match stance {
            Stance::Fast => Self::Fast,
            Stance::Balanced => Self::Balanced,
            Stance::Heavy => Self::Heavy,
        }
    }
}

/// What a clash between two kinds did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClashVerdict {
    /// The row kind lost the clash.
    Lose,
    /// Both bound; nobody won.
    Draw,
    /// The row kind won the clash.
    Win,
}

/// The clash matrix: row is mine, column is theirs, order Fast, Balanced,
/// Heavy, Air. Cells are `0` lose, `1` draw or `2` win.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StanceMatrix {
    cells: [[u8; CLASH_COUNT]; CLASH_COUNT],
}

impl StanceMatrix {
    /// The shipped V1 row, straight from the official guide: balanced beats
    /// fast, heavy beats both, air loses to all three, a kind never beats
    /// itself, and any grounded kind beats air.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            cells: [
                [DRAW, LOSE, LOSE, WIN],  // fast: loses to balanced and heavy, wins vs air
                [WIN, DRAW, LOSE, WIN],   // balanced: beats fast, loses to heavy
                [WIN, WIN, DRAW, WIN],    // heavy: beats both, wins vs air
                [LOSE, LOSE, LOSE, DRAW], // air: loses to every grounded kind
            ],
        }
    }

    /// The verdict for one pair: row is `mine`, column is `theirs`.
    ///
    /// Total: a cell past `2` (only reachable through a matrix that failed
    /// [`StanceMatrix::validate`]) reads as [`ClashVerdict::Lose`].
    #[must_use]
    pub fn verdict(&self, mine: ClashExt, theirs: ClashExt) -> ClashVerdict {
        match self.cells[mine.index()][theirs.index()] {
            DRAW => ClashVerdict::Draw,
            WIN => ClashVerdict::Win,
            _ => ClashVerdict::Lose,
        }
    }

    /// Range-check the matrix: every cell in range, every diagonal a draw, and
    /// every off-diagonal pair antisymmetric (a win one way is a loss back).
    ///
    /// Determinism: fixed row-then-column walk; the first bad cell wins.
    pub fn validate(&self) -> Result<(), StanceError> {
        let mut row = 0;
        while row < CLASH_COUNT {
            let mut col = 0;
            while col < CLASH_COUNT {
                let cell = self.cells[row][col];
                if !self.cell_is_valid(row, col, cell) {
                    return Err(self.bad_cell(row, col));
                }
                col += 1;
            }
            row += 1;
        }
        Ok(())
    }

    /// One cell's rule: in range, diagonal draw, off-diagonal antisymmetric.
    fn cell_is_valid(&self, row: usize, col: usize, cell: u8) -> bool {
        if cell > WIN {
            return false;
        }
        if row == col {
            return cell == DRAW;
        }
        mirrors(cell, self.cells[col][row])
    }

    /// The error for one bad cell: a symmetry break names itself, everything
    /// else is a bad cell.
    fn bad_cell(&self, row: usize, col: usize) -> StanceError {
        let mine = clash_at(row);
        let theirs = clash_at(col);
        let cell = self.cells[row][col];
        if row != col && cell <= WIN && !mirrors(cell, self.cells[col][row]) {
            return StanceError::NotAntisymmetric { mine, theirs };
        }
        StanceError::BadCell { mine, theirs }
    }
}

/// `a` and `b` are mirror cells: a win one way must be a loss the other.
const fn mirrors(a: u8, b: u8) -> bool {
    match a {
        WIN => b == LOSE,
        LOSE => b == WIN,
        _ => b == DRAW,
    }
}

/// Matrix index to kind, in table order.
const fn clash_at(index: usize) -> ClashExt {
    match index {
        0 => ClashExt::Fast,
        1 => ClashExt::Balanced,
        2 => ClashExt::Heavy,
        _ => ClashExt::Air,
    }
}

/// Short lowercase name for one clash kind, for error text.
const fn clash_name(kind: ClashExt) -> &'static str {
    match kind {
        ClashExt::Fast => "fast",
        ClashExt::Balanced => "balanced",
        ClashExt::Heavy => "heavy",
        ClashExt::Air => "air",
    }
}

/// Why a clash matrix was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StanceError {
    /// Unknown matrix version.
    VersionUnknown(u32),
    /// `mine` beat `theirs`, but `theirs` did not lose to `mine`.
    NotAntisymmetric {
        /// The row kind of the offending cell.
        mine: ClashExt,
        /// The column kind of the offending cell.
        theirs: ClashExt,
    },
    /// A cell was out of range, or a diagonal was not a draw.
    BadCell {
        /// The row kind of the offending cell.
        mine: ClashExt,
        /// The column kind of the offending cell.
        theirs: ClashExt,
    },
}

impl core::fmt::Display for StanceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VersionUnknown(version) => {
                write!(f, "stance matrix version {version} is not valid")
            }
            Self::NotAntisymmetric { mine, theirs } => write!(
                f,
                "stance matrix cell {} vs {} is not antisymmetric",
                clash_name(*mine),
                clash_name(*theirs)
            ),
            Self::BadCell { mine, theirs } => write!(
                f,
                "stance matrix cell {} vs {} is out of range",
                clash_name(*mine),
                clash_name(*theirs)
            ),
        }
    }
}

impl std::error::Error for StanceError {}

/// The shipped clash matrix.
pub const STANCE_MATRIX_V1: StanceMatrix = StanceMatrix::v1();

/// Resolve a replay version tag to its static matrix: 1 gives V1, else unknown.
///
/// Determinism scope: pure lookup; the same tag gives the same matrix.
#[must_use]
pub fn stance_matrix_for_version(version: u32) -> Option<&'static StanceMatrix> {
    match version {
        STANCE_MATRIX_VERSION_V1 => Some(&STANCE_MATRIX_V1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ClashExt; CLASH_COUNT] = [
        ClashExt::Fast,
        ClashExt::Balanced,
        ClashExt::Heavy,
        ClashExt::Air,
    ];

    /// A copy of the shipped row with one cell rewritten.
    fn with_cell(mine: ClashExt, theirs: ClashExt, value: u8) -> StanceMatrix {
        let mut cells = STANCE_MATRIX_V1.cells;
        cells[mine.index()][theirs.index()] = value;
        StanceMatrix { cells }
    }

    #[test]
    fn v1_covers_all_sixteen_pairs_antisymmetric() {
        assert_eq!(STANCE_MATRIX_V1.validate(), Ok(()));
        for mine in ALL {
            for theirs in ALL {
                if mine == theirs {
                    continue;
                }
                let out = STANCE_MATRIX_V1.verdict(mine, theirs);
                let back = STANCE_MATRIX_V1.verdict(theirs, mine);
                match out {
                    ClashVerdict::Win => assert_eq!(back, ClashVerdict::Lose),
                    ClashVerdict::Lose => assert_eq!(back, ClashVerdict::Win),
                    ClashVerdict::Draw => assert_eq!(back, ClashVerdict::Draw),
                }
            }
        }
        assert_eq!(ALL.map(ClashExt::index), [0, 1, 2, 3]);
    }

    #[test]
    fn diagonals_draw() {
        for kind in ALL {
            assert_eq!(STANCE_MATRIX_V1.verdict(kind, kind), ClashVerdict::Draw);
        }
    }

    #[test]
    fn fast_loses_to_balanced_which_loses_to_heavy() {
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Fast, ClashExt::Balanced),
            ClashVerdict::Lose
        );
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Balanced, ClashExt::Fast),
            ClashVerdict::Win
        );
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Balanced, ClashExt::Heavy),
            ClashVerdict::Lose
        );
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Heavy, ClashExt::Balanced),
            ClashVerdict::Win
        );
    }

    #[test]
    fn heavy_beats_fast_and_balanced_and_air_loses_to_all() {
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Heavy, ClashExt::Fast),
            ClashVerdict::Win
        );
        assert_eq!(
            STANCE_MATRIX_V1.verdict(ClashExt::Heavy, ClashExt::Balanced),
            ClashVerdict::Win
        );
        for grounded in [ClashExt::Fast, ClashExt::Balanced, ClashExt::Heavy] {
            assert_eq!(
                STANCE_MATRIX_V1.verdict(grounded, ClashExt::Air),
                ClashVerdict::Win
            );
            assert_eq!(
                STANCE_MATRIX_V1.verdict(ClashExt::Air, grounded),
                ClashVerdict::Lose
            );
        }
    }

    #[test]
    fn from_stance_reaches_every_kind() {
        assert_eq!(ClashExt::from_stance(Stance::Fast, false), ClashExt::Fast);
        assert_eq!(
            ClashExt::from_stance(Stance::Balanced, false),
            ClashExt::Balanced
        );
        assert_eq!(ClashExt::from_stance(Stance::Heavy, false), ClashExt::Heavy);
        for stance in Stance::all() {
            assert_eq!(ClashExt::from_stance(stance, true), ClashExt::Air);
        }
    }

    #[test]
    fn validate_rejects_hand_broken_matrices() {
        // A symmetry break: both directions now say the row wins.
        let both_win = with_cell(ClashExt::Fast, ClashExt::Balanced, WIN);
        assert_eq!(
            both_win.validate(),
            Err(StanceError::NotAntisymmetric {
                mine: ClashExt::Fast,
                theirs: ClashExt::Balanced,
            })
        );

        // A cell out of range.
        let out_of_range = with_cell(ClashExt::Air, ClashExt::Air, 9);
        assert_eq!(
            out_of_range.validate(),
            Err(StanceError::BadCell {
                mine: ClashExt::Air,
                theirs: ClashExt::Air,
            })
        );

        // A diagonal that is not a draw.
        let bad_diagonal = with_cell(ClashExt::Heavy, ClashExt::Heavy, WIN);
        assert_eq!(
            bad_diagonal.validate(),
            Err(StanceError::BadCell {
                mine: ClashExt::Heavy,
                theirs: ClashExt::Heavy,
            })
        );

        assert!(with_cell(ClashExt::Fast, ClashExt::Air, LOSE)
            .validate()
            .is_err());
    }

    #[test]
    fn version_lookup() {
        assert_eq!(
            stance_matrix_for_version(STANCE_MATRIX_VERSION_V1),
            Some(&STANCE_MATRIX_V1)
        );
        assert_eq!(stance_matrix_for_version(2), None);
        assert_eq!(stance_matrix_for_version(0), None);
        assert_eq!(STANCE_MATRIX_V1, StanceMatrix::v1());
    }

    #[test]
    fn errors_display() {
        assert_eq!(
            StanceError::VersionUnknown(3).to_string(),
            "stance matrix version 3 is not valid"
        );
        assert_eq!(
            StanceError::NotAntisymmetric {
                mine: ClashExt::Fast,
                theirs: ClashExt::Balanced,
            }
            .to_string(),
            "stance matrix cell fast vs balanced is not antisymmetric"
        );
        assert_eq!(
            StanceError::BadCell {
                mine: ClashExt::Air,
                theirs: ClashExt::Air,
            }
            .to_string(),
            "stance matrix cell air vs air is out of range"
        );
    }
}
