//! dig: the dig layer. An integer height grid in millimetres, cell 0.5 m.
//! `dig` and `pile` move earth and conserve its volume.
//! Walkers read height and slope with an integer slope limit and a ground
//! snap; nav reads the blocked-cell map this grid emits.
//! Units: millimetres for length and height, per mille of run for grade,
//! cell-millimetres for moved earth (one cell one millimetre deep).

use engine_core::Fnv64;

/// Cell side in millimetres: half a metre.
pub const CELL_MM: i32 = 500;

/// Ground area of one cell in square millimetres.
pub const CELL_AREA_MM2: i64 = 250_000;

/// Walkable slope cap, rise per mille of run; 1000 is 45 degrees.
pub const MAX_SLOPE_GRADE_Q: i32 = 1000;

/// Farthest the feet stick to falling ground while still grounded, millimetres.
pub const SNAP_MM: i32 = 400;

/// An axis-aligned ground rectangle in millimetres. The constructor sorts the
/// edges, so a caller never hands the grid an inverted rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectMm {
    /// Lower x edge, millimetres.
    pub x0_mm: i32,
    /// Lower z edge, millimetres.
    pub z0_mm: i32,
    /// Upper x edge, millimetres.
    pub x1_mm: i32,
    /// Upper z edge, millimetres.
    pub z1_mm: i32,
}

impl RectMm {
    /// New rect with every edge ordered low to high.
    pub fn new(x0_mm: i32, z0_mm: i32, x1_mm: i32, z1_mm: i32) -> Self {
        let (x_low, x_high) = (i32::min(x0_mm, x1_mm), i32::max(x0_mm, x1_mm));
        let (z_low, z_high) = (i32::min(z0_mm, z1_mm), i32::max(z0_mm, z1_mm));
        Self {
            x0_mm: x_low,
            z0_mm: z_low,
            x1_mm: x_high,
            z1_mm: z_high,
        }
    }
}

/// Earth one dig or one pile moved: the cells that took part and how much
/// earth moved, in cell-millimetres (one cell one millimetre deep).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EarthMove {
    /// Cells the move touched.
    pub cells: usize,
    /// Earth moved, in cell-millimetres.
    pub amount_cell_mm: i64,
}

impl EarthMove {
    /// The same earth in cubic millimetres.
    pub fn volume_mm3(&self) -> i64 {
        self.amount_cell_mm * CELL_AREA_MM2
    }

    /// No earth moved.
    fn empty() -> Self {
        Self {
            cells: 0,
            amount_cell_mm: 0,
        }
    }
}

/// What the ground under a walker looks like: the height, the grade of that
/// cell, and where the feet land when the ground is within the snap reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footing {
    /// Ground height under the walker, millimetres.
    pub ground_mm: i32,
    /// Steepest grade from that cell to a neighbour, rise per mille of run.
    pub grade_q: i32,
    /// Height the feet snap to, or `None` when the ground is out of reach.
    pub snapped_mm: Option<i32>,
}

/// The dig layer: an integer height grid in millimetres, one cell half a
/// metre wide. The tick owns it; `dig` and `pile` move earth between cells
/// and never lose a cubic millimetre.
/// Determinism: integer math only, fixed row-major cell order, no floats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeightGrid {
    cols: usize,
    rows: usize,
    heights: Vec<i32>,
}

impl HeightGrid {
    /// A flat grid of `cols` by `rows` cells, every cell at `base_mm`.
    pub fn new(cols: usize, rows: usize, base_mm: i32) -> Self {
        Self {
            cols,
            rows,
            heights: vec![base_mm; cols.saturating_mul(rows)],
        }
    }

    /// Grid width in cells.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Grid depth in cells.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Height of one cell in millimetres; `None` outside the grid.
    pub fn height_mm(&self, x: i32, z: i32) -> Option<i32> {
        let index = self.index(x, z)?;
        Some(self.heights[index])
    }

    /// Ground height under a point in millimetres; `None` outside the grid.
    pub fn ground_mm(&self, x_mm: i32, z_mm: i32) -> Option<i32> {
        let (cell_x, cell_z) = self.cell_of(x_mm, z_mm);
        self.height_mm(cell_x, cell_z)
    }

    /// The cell under a point in millimetres. A point below the origin floors
    /// into the cell below zero, like every other grid in the engine.
    pub fn cell_of(&self, x_mm: i32, z_mm: i32) -> (i32, i32) {
        (x_mm.div_euclid(CELL_MM), z_mm.div_euclid(CELL_MM))
    }

    /// The cells a rect covers, as the first and last cell of each axis:
    /// `(x_first, x_last, z_first, z_last)`, both ends included. A cell
    /// belongs to the rect when its centre lies inside it. `None` when the
    /// rect misses the grid.
    pub fn covered(&self, rect: RectMm) -> Option<(i32, i32, i32, i32)> {
        let first_x = Self::first_cell_at_or_after(i64::from(rect.x0_mm));
        let last_x = Self::last_cell_at_or_before(i64::from(rect.x1_mm));
        let first_z = Self::first_cell_at_or_after(i64::from(rect.z0_mm));
        let last_z = Self::last_cell_at_or_before(i64::from(rect.z1_mm));
        let x_first = first_x.max(0);
        let x_last = last_x.min(self.last_cell_x());
        let z_first = first_z.max(0);
        let z_last = last_z.min(self.last_cell_z());
        if x_first > x_last || z_first > z_last {
            return None;
        }
        Some((
            i32::try_from(x_first).unwrap_or(i32::MAX),
            i32::try_from(x_last).unwrap_or(i32::MIN),
            i32::try_from(z_first).unwrap_or(i32::MAX),
            i32::try_from(z_last).unwrap_or(i32::MIN),
        ))
    }

    /// Dig the rect down to `floor_mm`. The floor is an absolute height: a
    /// cell already at or below it keeps its ground, and the earth that comes
    /// out is the caller's to pile somewhere else.
    pub fn dig(&mut self, rect: RectMm, floor_mm: i32) -> EarthMove {
        let Some((x_first, x_last, z_first, z_last)) = self.covered(rect) else {
            return EarthMove::empty();
        };
        let mut amount_cell_mm = 0i64;
        let mut cells = 0usize;
        for cell_z in z_first..=z_last {
            for cell_x in x_first..=x_last {
                let Some(index) = self.index(cell_x, cell_z) else {
                    continue;
                };
                let current = self.heights[index];
                if i64::from(current) <= i64::from(floor_mm) {
                    continue;
                }
                amount_cell_mm += i64::from(current) - i64::from(floor_mm);
                cells += 1;
                self.heights[index] = floor_mm;
            }
        }
        EarthMove {
            cells,
            amount_cell_mm,
        }
    }

    /// Pile earth onto the rect. The amount is in cell-millimetres and is
    /// spread over the covered cells: every cell takes an equal share and the
    /// first cells in row-major order take the leftover millimetre. A pile
    /// never lowers ground, and a negative amount moves no earth.
    pub fn pile(&mut self, rect: RectMm, amount_cell_mm: i64) -> EarthMove {
        let amount = amount_cell_mm.max(0);
        let Some((x_first, x_last, z_first, z_last)) = self.covered(rect) else {
            return EarthMove::empty();
        };
        let cells = i64::from(x_last - x_first + 1) * i64::from(z_last - z_first + 1);
        let share = amount / cells;
        let mut leftover = amount % cells;
        for cell_z in z_first..=z_last {
            for cell_x in x_first..=x_last {
                let Some(index) = self.index(cell_x, cell_z) else {
                    continue;
                };
                let mut give = share;
                if leftover > 0 {
                    give += 1;
                    leftover -= 1;
                }
                let raised = i64::from(self.heights[index]) + give;
                self.heights[index] = i32::try_from(raised).unwrap_or(i32::MAX);
            }
        }
        EarthMove {
            cells: usize::try_from(cells).unwrap_or(0),
            amount_cell_mm: amount,
        }
    }

    /// The whole grid's earth, in cell-millimetres: the sum of every cell's
    /// height. Digging lowers it, piling raises it by the same amount.
    pub fn total_cell_mm(&self) -> i64 {
        self.heights.iter().map(|height| i64::from(*height)).sum()
    }

    /// The steepest grade from a cell to one of its neighbours, rise per
    /// mille of run. `None` outside the grid.
    pub fn grade_q(&self, x: i32, z: i32) -> Option<i32> {
        let height = self.height_mm(x, z)?;
        let mut worst = 0i32;
        for (step_x, step_z) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let neighbour_x = x.saturating_add(step_x);
            let neighbour_z = z.saturating_add(step_z);
            let Some(neighbour) = self.height_mm(neighbour_x, neighbour_z) else {
                continue;
            };
            worst = worst.max(Self::grade_between(height, neighbour, CELL_MM));
        }
        Some(worst)
    }

    /// True when a walker can walk from one point to another: the points sit
    /// in the same cell or in neighbouring cells, and the grade between them
    /// stays inside the slope limit.
    pub fn can_step(&self, from_x_mm: i32, from_z_mm: i32, to_x_mm: i32, to_z_mm: i32) -> bool {
        let (from_x, from_z) = self.cell_of(from_x_mm, from_z_mm);
        let (to_x, to_z) = self.cell_of(to_x_mm, to_z_mm);
        let (Some(from_mm), Some(to_mm)) =
            (self.height_mm(from_x, from_z), self.height_mm(to_x, to_z))
        else {
            return false;
        };
        let run_x = (to_x - from_x).abs();
        let run_z = (to_z - from_z).abs();
        if run_x + run_z == 0 {
            return true;
        }
        if run_x + run_z > 1 {
            return false;
        }
        Self::grade_between(from_mm, to_mm, CELL_MM) <= MAX_SLOPE_GRADE_Q
    }

    /// What the ground under a walker looks like: the height, the grade of
    /// that cell, and the height the feet land on when the ground is at or
    /// below them and within the snap reach. `None` outside the grid.
    pub fn footing(&self, feet_mm: i32, x_mm: i32, z_mm: i32) -> Option<Footing> {
        let (cell_x, cell_z) = self.cell_of(x_mm, z_mm);
        let ground_mm = self.height_mm(cell_x, cell_z)?;
        let grade_q = self.grade_q(cell_x, cell_z).unwrap_or(0);
        let drop = i64::from(feet_mm) - i64::from(ground_mm);
        let snapped_mm = if (0..=i64::from(SNAP_MM)).contains(&drop) {
            Some(ground_mm)
        } else {
            None
        };
        Some(Footing {
            ground_mm,
            grade_q,
            snapped_mm,
        })
    }

    /// The nav map: one flag per cell in row-major order. A cell is blocked
    /// when the ground to any neighbour is steeper than the slope limit,
    /// which keeps a cell of margin around a ditch.
    pub fn blocked(&self) -> Vec<bool> {
        let mut map = vec![false; self.cols.saturating_mul(self.rows)];
        for cell_z in 0..self.rows {
            for cell_x in 0..self.cols {
                let (Ok(x), Ok(z)) = (i32::try_from(cell_x), i32::try_from(cell_z)) else {
                    continue;
                };
                let steep = self.grade_q(x, z).unwrap_or(0) > MAX_SLOPE_GRADE_Q;
                map[cell_z * self.cols + cell_x] = steep;
            }
        }
        map
    }

    /// True when a cell is blocked for nav. Ground outside the grid is
    /// blocked too: the walker waits, it never guesses.
    pub fn is_blocked(&self, x: i32, z: i32) -> bool {
        match self.grade_q(x, z) {
            Some(grade) => grade > MAX_SLOPE_GRADE_Q,
            None => true,
        }
    }

    /// Digest of the whole grid: shape plus every height. The same grid
    /// always hashes the same.
    pub fn state_hash(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u64(u64::try_from(self.cols).unwrap_or(u64::MAX));
        hash.write_u64(u64::try_from(self.rows).unwrap_or(u64::MAX));
        for height in &self.heights {
            hash.write_i32(*height);
        }
        hash.finish()
    }

    /// Index of one cell in the height row-major array; `None` outside.
    fn index(&self, x: i32, z: i32) -> Option<usize> {
        if x < 0 || z < 0 {
            return None;
        }
        let x = usize::try_from(x).ok()?;
        let z = usize::try_from(z).ok()?;
        if x >= self.cols || z >= self.rows {
            return None;
        }
        Some(z * self.cols + x)
    }

    /// Last cell column of the grid.
    fn last_cell_x(&self) -> i64 {
        i64::try_from(self.cols)
            .unwrap_or(i64::MAX)
            .saturating_sub(1)
    }

    /// Last cell row of the grid.
    fn last_cell_z(&self) -> i64 {
        i64::try_from(self.rows)
            .unwrap_or(i64::MAX)
            .saturating_sub(1)
    }

    /// The first cell whose centre lies at or past `mm`: the cell that holds
    /// the low edge of a rect that starts at `mm`. Signed division rounds
    /// toward zero, so the ceiling comes from the floor of the negation.
    fn first_cell_at_or_after(mm: i64) -> i64 {
        let cell = i64::from(CELL_MM);
        let half = cell / 2;
        -(-(mm - half)).div_euclid(cell)
    }

    /// The last cell whose centre lies at or before `mm`: the cell that holds
    /// the high edge of a rect that ends at `mm`.
    fn last_cell_at_or_before(mm: i64) -> i64 {
        let cell = i64::from(CELL_MM);
        let half = cell / 2;
        (mm - half).div_euclid(cell)
    }

    /// Grade between two heights over a run of `run_mm`, rise per mille of run.
    fn grade_between(from_mm: i32, to_mm: i32, run_mm: i32) -> i32 {
        let rise = (i64::from(to_mm) - i64::from(from_mm)).abs();
        let grade = rise * 1000 / i64::from(run_mm.max(1));
        i32::try_from(grade).unwrap_or(i32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat camp ground: 24 by 24 cells, twelve metres a side.
    fn camp() -> HeightGrid {
        HeightGrid::new(24, 24, 0)
    }

    /// The ditch: 1480 mm wide, ten metres long.
    fn ditch() -> RectMm {
        RectMm::new(1000, 0, 2480, 10_000)
    }

    /// The rampart beside the ditch: four metres wide, ten metres long.
    fn rampart() -> RectMm {
        RectMm::new(2500, 0, 6500, 10_000)
    }

    #[test]
    fn ditch_volume_moves_into_the_rampart() {
        let mut grid = camp();
        let before = grid.total_cell_mm();
        let dug = grid.dig(ditch(), -888);
        assert_eq!(dug.cells, 60, "three cells wide, twenty long");
        assert_eq!(dug.amount_cell_mm, 53_280, "60 cells 888 mm deep");
        assert_eq!(dug.volume_mm3(), 13_320_000_000, "13.32 cubic metres");
        // The ditch floor is 888 mm down and the plain around it is untouched.
        assert_eq!(grid.height_mm(3, 10), Some(-888));
        assert_eq!(grid.height_mm(1, 10), Some(0), "the plain is untouched");
        assert_eq!(
            grid.height_mm(5, 10),
            Some(0),
            "the rampart ground is untouched"
        );
        // The grid can only hold whole cells, so the 1480 mm asked for comes
        // out as 1500 mm of ground: never less than the request.
        const { assert!(3 * CELL_MM >= 1480) };
        // The same earth goes into the rampart beside the ditch.
        let piled = grid.pile(rampart(), dug.amount_cell_mm);
        assert_eq!(piled.amount_cell_mm, dug.amount_cell_mm, "the same volume");
        assert_eq!(piled.cells, 160, "eight cells wide, twenty long");
        assert_eq!(grid.height_mm(6, 10), Some(333), "the rampart rises evenly");
        assert_eq!(grid.total_cell_mm(), before, "the earth is conserved");
    }

    #[test]
    fn walker_cannot_climb_the_ditch_side() {
        let mut grid = camp();
        grid.dig(ditch(), -888);
        // 888 mm down over one 500 mm cell is 1776 per mille of run, past the
        // 1000 the limit allows.
        assert!(
            !grid.can_step(750, 500, 1250, 500),
            "the ditch side is too steep to climb"
        );
        assert!(
            !grid.can_step(1250, 500, 750, 500),
            "and too steep to climb out of"
        );
        assert!(grid.can_step(250, 500, 750, 500), "the plain is flat");
        let grade = grid.grade_q(2, 1).expect("the ditch cell has a grade");
        assert_eq!(grade, 1776);
        assert!(grade > MAX_SLOPE_GRADE_Q);
    }

    #[test]
    fn the_ground_snap_holds_the_walker_on_the_ground() {
        let mut grid = camp();
        grid.dig(ditch(), -888);
        let rest = grid
            .footing(0, 750, 500)
            .expect("the plain is inside the grid");
        assert_eq!(rest.ground_mm, 0);
        assert_eq!(rest.snapped_mm, Some(0), "the feet rest on the ground");
        let near = grid
            .footing(300, 750, 500)
            .expect("the plain is inside the grid");
        assert_eq!(near.snapped_mm, Some(0), "300 mm is inside the reach");
        let air = grid
            .footing(500, 750, 500)
            .expect("the plain is inside the grid");
        assert_eq!(air.snapped_mm, None, "500 mm is past the 400 mm reach");
        let down = grid
            .footing(-888, 1250, 500)
            .expect("the ditch is inside the grid");
        assert_eq!(down.ground_mm, -888);
        assert_eq!(down.snapped_mm, Some(-888));
    }

    #[test]
    fn the_grid_hashes_the_same_twice() {
        let a = camp();
        let b = camp();
        assert_eq!(a.state_hash(), a.state_hash());
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "the same grid hashes the same"
        );
        let mut dug = camp();
        dug.dig(ditch(), -888);
        assert_ne!(
            dug.state_hash(),
            a.state_hash(),
            "a dug grid is another grid"
        );
    }

    #[test]
    fn nav_reads_the_blocked_cells() {
        let mut grid = camp();
        grid.dig(ditch(), -888);
        let blocked = grid.blocked();
        assert_eq!(blocked.len(), 24 * 24);
        assert!(blocked[10 * 24 + 2], "the ditch cell is blocked");
        assert!(blocked[10 * 24 + 1], "the rim keeps a cell of margin");
        assert!(!blocked[10 * 24], "the open plain is walkable");
        assert!(grid.is_blocked(2, 10));
        assert!(!grid.is_blocked(0, 10));
    }

    #[test]
    fn a_rect_covers_the_cells_whose_centre_lies_inside_it() {
        let grid = camp();
        let covered = grid.covered(ditch()).expect("the ditch is inside the grid");
        assert_eq!(covered, (2, 4, 0, 19));
        assert!(grid
            .covered(RectMm::new(-4000, -4000, -3000, -3000))
            .is_none());
    }

    #[test]
    fn dig_stops_at_the_floor_and_pile_needs_a_cell() {
        let mut grid = camp();
        let dug = grid.dig(RectMm::new(0, 0, 500, 500), -100);
        assert_eq!(dug.amount_cell_mm, 100);
        let again = grid.dig(RectMm::new(0, 0, 500, 500), -100);
        assert_eq!(again.amount_cell_mm, 0, "the floor is already there");
        let empty = grid.pile(RectMm::new(-4000, -4000, -3000, -3000), 1000);
        assert_eq!(empty.amount_cell_mm, 0, "no cell, no earth");
        assert_eq!(grid.total_cell_mm(), -100);
    }

    #[test]
    fn a_pile_splits_the_earth_exactly() {
        let mut grid = camp();
        let one = grid.pile(RectMm::new(0, 0, 500, 500), 10);
        assert_eq!(one.amount_cell_mm, 10);
        assert_eq!(grid.height_mm(0, 0), Some(10));
        // A pile spreads over the cells it covers: two cells take five each.
        let two = grid.pile(RectMm::new(0, 0, 1000, 500), 10);
        assert_eq!(two.amount_cell_mm, 10);
        assert_eq!(
            grid.height_mm(0, 0),
            Some(15),
            "the pile adds to the ground"
        );
        assert_eq!(grid.height_mm(1, 0), Some(5));
        // A split that does not divide leaves the extra millimetre on the
        // first cells in row-major order.
        let three = grid.pile(RectMm::new(0, 0, 1500, 500), 11);
        assert_eq!(three.amount_cell_mm, 11);
        assert_eq!(grid.height_mm(0, 0), Some(19));
        assert_eq!(grid.height_mm(1, 0), Some(9));
        assert_eq!(grid.height_mm(2, 0), Some(3));
    }
}
