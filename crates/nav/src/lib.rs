//! nav: grid BFS path (bake) + gradient follow (flow-field-lite).

pub mod astar;
pub mod bake;
pub mod crowd;
mod cursor;
pub mod flow;
/// Portal string-pulling over tiled walks with salted refs (NV6).
pub mod funnel;
pub mod hpa;
pub mod links;
pub mod mesh;
pub mod neighbors;
pub mod rebake;
pub mod tiles;

pub use astar::astar;
pub use hpa::hpa;

use std::collections::VecDeque;

/// Typed grid failures; bad cells refuse, never panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridError {
    /// Cell is outside the grid; the grid keeps its cells.
    OutOfBounds {
        /// Cell x offered in cells.
        x: usize,
        /// Cell y offered in cells.
        y: usize,
    },
}

impl std::fmt::Display for GridError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfBounds { x, y } => write!(f, "grid: cell ({x}, {y}) outside the grid"),
        }
    }
}

impl std::error::Error for GridError {}

pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub blocked: Vec<bool>,
}

impl Grid {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            blocked: vec![false; w * h],
        }
    }
    /// Index of an in-bounds cell, or the typed refusal.
    fn index_of(&self, (x, y): (usize, usize)) -> Result<usize, GridError> {
        if x >= self.w || y >= self.h {
            return Err(GridError::OutOfBounds { x, y });
        }
        Ok(y * self.w + x)
    }
    /// Blocks one cell. Errors: [`GridError::OutOfBounds`] past the grid.
    pub fn set_blocked(&mut self, x: usize, y: usize) -> Result<(), GridError> {
        let cell = self.index_of((x, y))?;
        self.blocked[cell] = true;
        Ok(())
    }
    /// BFS shortest path, cell list inclusive. `Ok(None)` when sealed or on a
    /// blocked endpoint. Errors: [`GridError::OutOfBounds`] for an endpoint
    /// past the grid. Deterministic: fixed E,W,S,N order, same inputs give
    /// the same walk.
    pub fn path(
        &self,
        start: (usize, usize),
        goal: (usize, usize),
    ) -> Result<Option<Vec<(usize, usize)>>, GridError> {
        // Both endpoints validate before the blocked test, so an off-grid
        // goal is reported even when the start is blocked.
        let (s, g) = (self.index_of(start)?, self.index_of(goal)?);
        if self.blocked[s] || self.blocked[g] {
            return Ok(None);
        }
        let mut parent: Vec<Option<usize>> = vec![None; self.w * self.h];
        let mut q = VecDeque::new();
        parent[s] = Some(s);
        q.push_back(s);
        while let Some(c) = q.pop_front() {
            if c == g {
                break;
            }
            let (cx, cy) = (c % self.w, c / self.w);
            for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 {
                    continue;
                }
                let n = ny as usize * self.w + nx as usize;
                if parent[n].is_none() && !self.blocked[n] {
                    parent[n] = Some(c);
                    q.push_back(n);
                }
            }
        }
        if parent[g].is_none() {
            return Ok(None);
        }
        // The BFS parent chain from the goal always ends at the start.
        let mut rev = vec![];
        let mut at = Some(g);
        while let Some(c) = at {
            rev.push((c % self.w, c / self.w));
            at = if c == s { None } else { parent[c] };
        }
        rev.reverse();
        Ok(Some(rev))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_grid_finds_shortest_path() {
        let grid = Grid::new(5, 5);
        let p = grid
            .path((0, 0), (4, 4))
            .expect("endpoints are in bounds")
            .expect("open grid must have path");
        assert_eq!(p.len(), 9);
        assert_eq!((p[0], p[p.len() - 1]), ((0, 0), (4, 4)));
        for w in p.windows(2) {
            assert_eq!(w[0].0.abs_diff(w[1].0) + w[0].1.abs_diff(w[1].1), 1);
        }
    }

    #[test]
    fn blocked_wall_returns_none() {
        let mut grid = Grid::new(5, 5);
        for y in 0..5 {
            grid.set_blocked(2, y).expect("wall cell is in bounds");
        }
        assert_eq!(grid.path((0, 0), (4, 4)), Ok(None));
    }

    #[test]
    fn start_equals_goal_and_determinism() {
        let grid = Grid::new(5, 5);
        assert_eq!(grid.path((2, 2), (2, 2)), Ok(Some(vec![(2, 2)])));
        assert_eq!(grid.path((0, 0), (4, 4)), grid.path((0, 0), (4, 4)));
    }

    #[test]
    fn gate_16x16_single_block_len_31() {
        let mut grid = Grid::new(16, 16);
        grid.set_blocked(7, 7).expect("cell is in bounds");
        let p = grid
            .path((0, 0), (15, 15))
            .expect("endpoints are in bounds")
            .expect("one block must not seal 16x16");
        assert_eq!(p.len(), 31);
        assert_eq!((p[0], p[p.len() - 1]), ((0, 0), (15, 15)));
    }

    #[test]
    fn out_of_bounds_cells_refuse_without_panic() {
        let mut grid = Grid::new(5, 5);
        // (5, 0) is not on the 5-wide grid even though index 5 is in the
        // backing vec, so a flat index alone would hit the wrong cell.
        assert_eq!(
            grid.set_blocked(5, 0),
            Err(GridError::OutOfBounds { x: 5, y: 0 })
        );
        assert_eq!(
            grid.set_blocked(0, 5),
            Err(GridError::OutOfBounds { x: 0, y: 5 })
        );
        assert_eq!(
            grid.path((5, 0), (0, 0)),
            Err(GridError::OutOfBounds { x: 5, y: 0 })
        );
        assert_eq!(
            grid.path((0, 0), (4, 5)),
            Err(GridError::OutOfBounds { x: 4, y: 5 })
        );
        // The grid is untouched: the open-grid walk still lands.
        let p = grid
            .path((0, 0), (4, 4))
            .expect("endpoints are in bounds")
            .expect("refused sets must not half-apply");
        assert_eq!(p.len(), 9);
        // A blocked start does not hide an off-grid goal.
        grid.set_blocked(0, 0).expect("cell is in bounds");
        assert_eq!(
            grid.path((0, 0), (9, 9)),
            Err(GridError::OutOfBounds { x: 9, y: 9 })
        );
    }
}
