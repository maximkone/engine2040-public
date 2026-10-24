//! ruins: the tour's ruins station route. `content/maps/rewrite/ruined_wall.rules` is the map the
//! `world.rewrite` door generates from: a three-course wall broken by breaches, with rubble spilled
//! beside them. The station walks the human through one breach instead of round the wall.
//!
//! Owner: this module owns the walkable route over the generated grid; the generator and its cell
//! alphabet belong to `engine-world::rewrite`. Presentation and script only: nothing here is
//! written into the match, so no wall is a collider yet (the arena's obstacle set belongs to
//! `arena_kit`); the station grades the crossing from the drawn position.
//!
//! Determinism: the route is a breadth-first walk over the grid in a fixed neighbour order with a
//! fixed seed, integer indexes and no clock, so one seed gives one route. Budget: one grid and one
//! route per station entry, no work per tick past the next waypoint.

use engine_world::rewrite::{Grid, Program};
use std::collections::VecDeque;
use std::fmt;

/// The rules file the `world.rewrite` door runs for the ruins station.
pub const RUINED_WALL: &str = include_str!("../../../../content/maps/rewrite/ruined_wall.rules");
/// Seed the tour's ruins grid is generated with: the map must read the same on every replay of the
/// same tour.
pub const RUINS_SEED: u64 = 2040;
/// Size of one grid cell in meters: the rules grid is a 40 x 11 plan of a 20 x 5.5 m stretch.
pub const CELL_M: f32 = 0.5;

/// Why the ruins route was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuinsError {
    /// The rules file did not parse.
    Rules,
    /// The rules settled into no grid.
    Run,
    /// No breach in the wall reaches the far side: every path is blocked.
    NoBreach,
}

impl fmt::Display for RuinsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rules => f.write_str("the ruined-wall rules did not parse"),
            Self::Run => f.write_str("the ruined-wall rules settled into no grid"),
            Self::NoBreach => f.write_str("the ruined wall has no walkable breach"),
        }
    }
}

impl std::error::Error for RuinsError {}

/// One planar waypoint of the route, meters from the grid's centre (`x`, `z`).
pub type Waypoint = [f32; 2];

/// True for a cell a man walks through: open ground, a breach or the rubble that spilled off it.
/// Only the standing wall blocks.
fn walkable(symbol: Option<char>) -> bool {
    matches!(symbol, Some('G' | 'H' | 'R'))
}

/// The grid the ruins station plays on.
pub fn ruined_grid(seed: u64) -> Result<Grid, RuinsError> {
    Program::parse(RUINED_WALL)
        .map_err(|_| RuinsError::Rules)?
        .run(seed)
        .map_err(|_| RuinsError::Run)
}

/// One cell's centre in world meters, the grid centred on the origin.
fn cell_at(width: u16, height: u16, x: usize, y: usize, cell_m: f32) -> Waypoint {
    let half_w = f32::from(width) * 0.5;
    let half_h = f32::from(height) * 0.5;
    let step = 0.5f32;
    let x = u16::try_from(x).map_or(0.0, |x| f32::from(x) + step);
    let y = u16::try_from(y).map_or(0.0, |y| f32::from(y) + step);
    [(x - half_w) * cell_m, (y - half_h) * cell_m]
}

/// A walk from the ground south of the wall to the ground north of it, through a breach, as
/// waypoints in cell centres (meters). Breadth-first over walkable cells, neighbours in a fixed
/// order (north, east, south, west) so the route is the same on every run; the first route found
/// from the southern ground wins. `Err(NoBreach)` when the wall has no gap a man fits through.
pub fn breach_route(grid: &Grid, cell_m: f32) -> Result<Vec<Waypoint>, RuinsError> {
    let (w, h) = (usize::from(grid.width()), usize::from(grid.height()));
    if w == 0 || h == 0 {
        return Err(RuinsError::NoBreach);
    }
    // The wall is whatever rows carry the standing wall symbol; the walk runs from the ground
    // just south of the first of them to the ground just past the last.
    let mut band: Option<(usize, usize)> = None;
    for y in 0..h {
        for x in 0..w {
            if grid.symbol_at(u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0))
                == Some('W')
            {
                band = Some(match band {
                    Some((first, last)) => (first.min(y), last.max(y)),
                    None => (y, y),
                });
            }
        }
    }
    let Some((wall_first, wall_last)) = band else {
        return Err(RuinsError::NoBreach);
    };
    let south = wall_first.checked_sub(1).ok_or(RuinsError::NoBreach)?;
    let north = wall_last + 1;
    if north >= h {
        return Err(RuinsError::NoBreach);
    }
    let index = |x: usize, y: usize| y * w + x;
    let cell = |x: usize, y: usize| {
        let x = u16::try_from(x).unwrap_or(u16::MAX);
        let y = u16::try_from(y).unwrap_or(u16::MAX);
        grid.symbol_at(x, y)
    };
    let mut from = vec![usize::MAX; w * h];
    let mut queue = VecDeque::new();
    for x in 0..w {
        if walkable(cell(x, south)) {
            from[index(x, south)] = index(x, south);
            queue.push_back((x, south));
        }
    }
    if queue.is_empty() {
        return Err(RuinsError::NoBreach);
    }
    let mut end = None;
    while let Some((x, y)) = queue.pop_front() {
        if y >= north {
            end = Some((x, y));
            break;
        }
        let mut step = |nx: usize, ny: usize| {
            if nx >= w || ny >= h || !walkable(cell(nx, ny)) || from[index(nx, ny)] != usize::MAX {
                return;
            }
            from[index(nx, ny)] = index(x, y);
            queue.push_back((nx, ny));
        };
        if let Some(ny) = y.checked_sub(1) {
            step(x, ny);
        }
        step(x + 1, y);
        if y + 1 < h {
            step(x, y + 1);
        }
        if let Some(nx) = x.checked_sub(1) {
            step(nx, y);
        }
    }
    let Some((mut x, mut y)) = end else {
        return Err(RuinsError::NoBreach);
    };
    let mut cells = vec![(x, y)];
    loop {
        let at = index(x, y);
        let back = from[at];
        if back == at {
            break;
        }
        let (bx, by) = (back % w, back / w);
        cells.push((bx, by));
        (x, y) = (bx, by);
    }
    cells.reverse();
    Ok(cells
        .into_iter()
        .map(|(x, y)| cell_at(grid.width(), grid.height(), x, y, cell_m))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every waypoint sits on a walkable cell: the route never crosses the standing wall.
    fn walkable_at(grid: &Grid, at: Waypoint) -> bool {
        let half_w = f32::from(grid.width()) * 0.5;
        let half_h = f32::from(grid.height()) * 0.5;
        let x = (at[0] / CELL_M + half_w).floor();
        let y = (at[1] / CELL_M + half_h).floor();
        let (Ok(x), Ok(y)) = (u16::try_from(x as i32), u16::try_from(y as i32)) else {
            return false;
        };
        walkable(grid.symbol_at(x, y))
    }

    /// F2P: the generated ruined wall leaves a breach a body can walk, from the ground south of it
    /// to the ground north of it, without stepping onto the standing wall.
    #[test]
    fn the_ruined_wall_leaves_a_breach_a_body_can_walk() {
        for seed in [1u64, 2040] {
            let grid = ruined_grid(seed).expect("the rules settle");
            let route = breach_route(&grid, CELL_M).expect("a breach");
            assert!(
                route.len() >= 4,
                "seed {seed}: a breach a body fits through: {route:?}"
            );
            let first = route.first().copied().expect("a start");
            let last = route.last().copied().expect("an end");
            assert!(
                first[1] < 0.0,
                "seed {seed}: the walk starts south: {first:?}"
            );
            assert!(last[1] > 0.0, "seed {seed}: and ends north: {last:?}");
            for at in &route {
                assert!(walkable_at(&grid, *at), "seed {seed}: {at:?} is wall");
            }
            for pair in route.windows(2) {
                let step = (pair[1][0] - pair[0][0]).abs() + (pair[1][1] - pair[0][1]).abs();
                assert!(
                    (step - CELL_M).abs() < 1e-4,
                    "seed {seed}: {:?} -> {:?} is one cell",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    /// The same seed folds the same route twice: the station replays as it played.
    #[test]
    fn one_seed_gives_one_route() {
        let grid = ruined_grid(RUINS_SEED).expect("the rules settle");
        let a = breach_route(&grid, CELL_M).expect("a breach");
        let b = breach_route(&grid, CELL_M).expect("a breach again");
        assert_eq!(a, b, "the route is deterministic");
    }
}
