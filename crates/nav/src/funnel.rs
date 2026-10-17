//! funnel: smooth paths over the tiled grid by portal string pulling.
//! Owner: nav crate; callers ask [`smooth_path`] for a walkable start/goal
//! pair and steer along [`SmoothPath::points`].
//! Units: half-cell lattice integers (x = 2 * cell + 1 is a cell centre, even
//! values are cell corners), tile versions, ticks via the nav.
//! Determinism: integer A* with a fixed neighbour order and a total-order
//! heap key, then an exact i64 cross-product funnel; no floats, no hash order,
//! no clock, so the same grid gives the same bytes on every machine.
//! Errors: typed [`TileError`]; a sealed goal is `Ok(None)`, never a guess.
//! Budgets: bake-time; one query allocates the A* lanes plus the corridor,
//! about 0.2 ms p50 on a 64x64 grid at release.
//! Clearance: the walker has zero radius, so a path may touch a cell corner.
//! PORT(recastnavigation/recastnavigation@main:DetourNavMeshQuery
//! findStraightPath, simple stupid funnel) and polyanya (Apache-2.0)
//! any-angle corridors.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::tiles::{StampedPath, TileError, TiledNav};

/// One lattice point `(x, y)` in half-cell units.
pub type Point = (i32, i32);

/// A smooth any-angle path: the corridor walk with its salted tile stamps
/// (tile id plus version) and the pulled-tight waypoints.
///
/// The stamps are the same salted tile references as [`StampedPath`], so a
/// rebake of any tile the corridor crosses flips [`SmoothPath::is_stale`] and
/// only those paths repath; a path elsewhere survives every rebake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmoothPath {
    walk: StampedPath,
    points: Vec<Point>,
}

impl SmoothPath {
    /// Waypoints from the start cell centre to the goal cell centre, in
    /// half-cell lattice units, with no collinear interior point.
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// The stamped corridor walk the funnel was pulled through.
    pub fn walk(&self) -> &StampedPath {
        &self.walk
    }

    /// True when a rebake moved under the corridor. Never allocates.
    /// Errors: as [`StampedPath::is_stale`].
    pub fn is_stale(&self, nav: &TiledNav) -> Result<bool, TileError> {
        self.walk.is_stale(nav)
    }

    /// Repaths exactly when stale: `Ok(None)` while fresh, else a new smooth
    /// path (or `Ok(None)` when the goal got sealed; ask [`SmoothPath::is_stale`]
    /// first to tell the two apart).
    /// Errors: as [`StampedPath::is_stale`] and [`smooth_path`].
    pub fn refresh(&self, nav: &TiledNav) -> Result<Option<Self>, TileError> {
        if !self.walk.is_stale(nav)? {
            return Ok(None);
        }
        smooth_path(nav, self.walk.start(), self.walk.goal())
    }

    /// Canonical bytes: the stamped corridor, then the waypoint count and
    /// each waypoint as little-endian `i32` pairs. Allocates the blob.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.walk.to_bytes();
        out.extend_from_slice(&(self.points.len() as u32).to_le_bytes());
        for (x, y) in &self.points {
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
        }
        out
    }
}

/// Smooth path from `start` to `goal` cells: an A* corridor that hugs the
/// straight line, string-pulled through its shared-edge portals.
///
/// The corridor is a shortest 4-dir walk (as long as [`TiledNav::find_path`]),
/// so the pulled path is never longer than the grid walk. `Ok(None)` when
/// sealed. Bake-time: allocates.
/// Errors: [`TileError::OutOfBounds`] (an endpoint past the grid),
/// [`TileError::BadGoal`] (an endpoint on a blocked cell, naming it).
pub fn smooth_path(
    nav: &TiledNav,
    start: (usize, usize),
    goal: (usize, usize),
) -> Result<Option<SmoothPath>, TileError> {
    for end in [start, goal] {
        if end.0 >= nav.width() || end.1 >= nav.height() {
            return Err(TileError::OutOfBounds);
        }
    }
    for end in [goal, start] {
        if nav.is_blocked(end.0, end.1) {
            return Err(TileError::BadGoal { x: end.0, y: end.1 });
        }
    }
    let Some(cells) = corridor(nav, start, goal) else {
        return Ok(None);
    };
    let points = pull(&cells);
    let walk = nav.stamp_walk(start, goal, cells)?;
    Ok(Some(SmoothPath { walk, points }))
}

/// Shortest 4-dir walk by A* with Manhattan distance. Equal-cost cells pop
/// in order of their distance from the start-goal line, so the walk is a
/// staircase around that line and the funnel can pull it nearly straight.
fn corridor(
    nav: &TiledNav,
    start: (usize, usize),
    goal: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    let (w, h) = (nav.width(), nav.height());
    let line = |x: usize, y: usize| {
        let (dx, dy) = (
            goal.0 as i64 - start.0 as i64,
            goal.1 as i64 - start.1 as i64,
        );
        ((x as i64 - start.0 as i64) * dy - (y as i64 - start.1 as i64) * dx).unsigned_abs()
    };
    let key = |x: usize, y: usize, g: u32| {
        let rest = x.abs_diff(goal.0) + y.abs_diff(goal.1);
        Reverse((g + rest as u32, line(x, y), y * w + x))
    };
    let mut cost = vec![u32::MAX; w * h];
    let mut parent = vec![usize::MAX; w * h];
    let mut closed = vec![false; w * h];
    let mut heap = BinaryHeap::new();
    cost[start.1 * w + start.0] = 0;
    heap.push(key(start.0, start.1, 0));
    while let Some(Reverse((_, _, at))) = heap.pop() {
        if std::mem::replace(&mut closed[at], true) {
            continue;
        }
        if at == goal.1 * w + goal.0 {
            break;
        }
        let (x, y) = (at % w, at / w);
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (Some(nx), Some(ny)) = (
                x.checked_add_signed(dx as isize),
                y.checked_add_signed(dy as isize),
            ) else {
                continue;
            };
            let next = ny * w + nx;
            if nx >= w
                || ny >= h
                || nav.is_blocked(nx, ny)
                || closed[next]
                || cost[at] + 1 >= cost[next]
            {
                continue;
            }
            cost[next] = cost[at] + 1;
            parent[next] = at;
            heap.push(key(nx, ny, cost[next]));
        }
    }
    let mut at = goal.1 * w + goal.0;
    if !closed[at] {
        return None;
    }
    let mut cells = vec![goal];
    while at != start.1 * w + start.0 {
        at = parent[at];
        cells.push((at % w, at / w));
    }
    cells.reverse();
    Some(cells)
}

/// `(b - o) x (c - o)`, exact in i64: positive when `c` lies left of `o -> b`.
fn cross(o: Point, b: Point, c: Point) -> i64 {
    (i64::from(b.0) - i64::from(o.0)) * (i64::from(c.1) - i64::from(o.1))
        - (i64::from(b.1) - i64::from(o.1)) * (i64::from(c.0) - i64::from(o.0))
}

/// Portal `(left, right)` for the step `from -> to` across their shared edge,
/// seen from the walker; left is the counter-clockwise side.
fn portal(from: (usize, usize), to: (usize, usize)) -> (Point, Point) {
    let (x, y) = (2 * from.0 as i32, 2 * from.1 as i32);
    match (to.0 as i32 - from.0 as i32, to.1 as i32 - from.1 as i32) {
        (1, 0) => ((x + 2, y + 2), (x + 2, y)),
        (-1, 0) => ((x, y), (x, y + 2)),
        (0, 1) => ((x, y + 2), (x + 2, y + 2)),
        _ => ((x + 2, y), (x, y)),
    }
}

/// Simple stupid funnel over the corridor's portals: the shortest path from
/// the first cell centre to the last inside the union of the cells.
fn pull(cells: &[(usize, usize)]) -> Vec<Point> {
    let centre = |c: (usize, usize)| (2 * c.0 as i32 + 1, 2 * c.1 as i32 + 1);
    let (first, last) = (centre(cells[0]), centre(cells[cells.len() - 1]));
    let mut portals = vec![(first, first)];
    portals.extend(cells.windows(2).map(|pair| portal(pair[0], pair[1])));
    portals.push((last, last));

    let mut out = vec![first];
    let (mut apex, mut left, mut right) = (first, first, first);
    let (mut left_i, mut right_i) = (0, 0);
    let mut i = 1;
    while i < portals.len() {
        let (new_left, new_right) = portals[i];
        if cross(apex, right, new_right) >= 0 {
            if apex == right || cross(apex, left, new_right) < 0 {
                (right, right_i) = (new_right, i);
            } else {
                out.push(left);
                (apex, right, right_i) = (left, left, left_i);
                i = left_i + 1;
                continue;
            }
        }
        if cross(apex, left, new_left) <= 0 {
            if apex == left || cross(apex, right, new_left) > 0 {
                (left, left_i) = (new_left, i);
            } else {
                out.push(right);
                (apex, left, left_i) = (right, right, right_i);
                i = right_i + 1;
                continue;
            }
        }
        i += 1;
    }
    out.push(last);
    simplify(out)
}

/// Drops repeated points and interior points on a straight run.
fn simplify(points: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for point in points {
        if out.last() == Some(&point) {
            continue;
        }
        while out.len() >= 2 && cross(out[out.len() - 2], out[out.len() - 1], point) == 0 {
            out.pop();
        }
        out.push(point);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::TICKS_PER_SECOND;

    fn open(w: usize, h: usize, edge: usize) -> TiledNav {
        TiledNav::bake(&vec![false; w * h], w, h, edge).expect("open grid bakes")
    }

    /// Wall column at `x` with one gap cell left open at `gap_y`.
    fn walled(w: usize, h: usize, wall_x: usize, gap_y: Option<usize>) -> TiledNav {
        let mut blocked = vec![false; w * h];
        for y in 0..h {
            if Some(y) != gap_y {
                blocked[y * w + wall_x] = true;
            }
        }
        TiledNav::bake(&blocked, w, h, 4).expect("walled grid bakes")
    }

    fn length(points: &[Point]) -> f64 {
        points
            .windows(2)
            .map(|p| {
                let (dx, dy) = (f64::from(p[1].0 - p[0].0), f64::from(p[1].1 - p[0].1));
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    /// SplitMix64 for seeded test queries (test-only, never in the sim).
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[test]
    fn open_field_pulls_to_one_straight_segment() {
        let nav = open(16, 16, 4);
        let path = smooth_path(&nav, (1, 1), (9, 5))
            .expect("endpoints fine")
            .expect("open");
        assert_eq!(
            path.points(),
            &[(3, 3), (19, 11)],
            "straight centre to centre"
        );
    }

    #[test]
    fn same_cell_is_one_point() {
        let nav = open(4, 4, 2);
        let path = smooth_path(&nav, (2, 2), (2, 2))
            .expect("fine")
            .expect("open");
        assert_eq!(path.points(), &[(5, 5)]);
    }

    /// Samples every segment: each sample must touch an open cell (a corner
    /// or an edge may touch a blocked one, the interior never crosses it).
    fn assert_inside_open_cells(nav: &TiledNav, points: &[Point]) {
        for pair in points.windows(2) {
            for k in 0..32i32 {
                let t = f64::from(2 * k + 1) / 64.0;
                let at = |a: i32, b: i32| f64::from(a) + t * f64::from(b - a);
                let (x, y) = (
                    at(pair[0].0, pair[1].0) / 2.0,
                    at(pair[0].1, pair[1].1) / 2.0,
                );
                let cells = [
                    (x.floor(), y.floor()),
                    (x.ceil() - 1.0, y.floor()),
                    (x.floor(), y.ceil() - 1.0),
                    (x.ceil() - 1.0, y.ceil() - 1.0),
                ];
                assert!(
                    cells.iter().any(|&(cx, cy)| cx >= 0.0
                        && cy >= 0.0
                        && !nav.is_blocked(cx as usize, cy as usize)),
                    "segment {pair:?} crosses a wall at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn wall_gap_path_runs_through_the_gap() {
        let nav = walled(24, 16, 12, Some(8));
        let path = smooth_path(&nav, (2, 2), (20, 14))
            .expect("fine")
            .expect("gap open");
        let points = path.points();
        assert_eq!((points[0], points[points.len() - 1]), ((5, 5), (41, 29)));
        assert!(points.len() > 2, "the wall forces a bend");
        assert_inside_open_cells(&nav, points);
    }

    #[test]
    fn sealed_goal_is_none_and_bad_ends_refuse() {
        let nav = walled(16, 8, 8, None);
        assert_eq!(smooth_path(&nav, (0, 0), (15, 7)), Ok(None));
        assert_eq!(
            smooth_path(&nav, (0, 0), (8, 3)),
            Err(TileError::BadGoal { x: 8, y: 3 })
        );
        assert_eq!(
            smooth_path(&nav, (0, 0), (16, 0)),
            Err(TileError::OutOfBounds)
        );
    }

    /// NV6 accept: never longer than the grid walk, and at least 10% shorter
    /// on 80% of seeded arena queries.
    #[test]
    fn smooth_paths_beat_the_grid_walk() {
        let nav = TiledNav::bake_seeded(2040, 48, 48, 8, 150).expect("arena bakes");
        let mut state = 7u64;
        let (mut asked, mut shorter) = (0u32, 0u32);
        while asked < 200 {
            let pick = |s: &mut u64| ((next(s) % 48) as usize, (next(s) % 48) as usize);
            let (start, goal) = (pick(&mut state), pick(&mut state));
            let Ok(Some(grid)) = nav.find_path(start, goal) else {
                continue;
            };
            let smooth = smooth_path(&nav, start, goal)
                .expect("same ends")
                .expect("reachable");
            assert_eq!(
                smooth.walk().cells().len(),
                grid.cells().len(),
                "corridor is a shortest walk"
            );
            let grid_len = 2.0 * (grid.cells().len() - 1) as f64;
            let smooth_len = length(smooth.points());
            assert!(
                smooth_len <= grid_len,
                "{start:?}->{goal:?}: {smooth_len} > {grid_len}"
            );
            assert_inside_open_cells(&nav, smooth.points());
            asked += 1;
            shorter += u32::from(smooth_len <= 0.9 * grid_len);
        }
        assert!(
            shorter * 10 >= asked * 8,
            "only {shorter}/{asked} queries 10% shorter"
        );
    }

    #[test]
    fn same_query_gives_the_same_bytes_twice() {
        let nav = TiledNav::bake_seeded(11, 32, 32, 8, 200).expect("arena bakes");
        let ask = || {
            smooth_path(&nav, (0, 0), (31, 31))
                .expect("ends fine")
                .map(|p| p.to_bytes())
        };
        assert_eq!(ask(), ask());
    }

    /// A breach opens a path within one second of ticks, and a rebake of a
    /// tile the path crosses flips it stale while a far path stays fresh.
    #[test]
    fn breach_opens_a_path_and_rebake_stales_only_crossing_paths() {
        let mut nav = walled(24, 16, 12, None);
        let (start, goal) = ((0, 8), (23, 8));
        assert_eq!(smooth_path(&nav, start, goal), Ok(None));
        let far = smooth_path(&nav, (0, 0), (5, 0))
            .expect("fine")
            .expect("open");

        nav.set_blocked(12, 8, false).expect("breach flips");
        let mut ticks = 0;
        let path = loop {
            nav.rebake_budget(1);
            ticks += 1;
            if let Some(path) = smooth_path(&nav, start, goal).expect("ends fine") {
                break path;
            }
            assert!(ticks < TICKS_PER_SECOND, "path must open within a second");
        };
        assert_eq!(length(path.points()), 2.0 * 23.0, "row 8 stays straight");
        assert_eq!(path.is_stale(&nav), Ok(false));
        assert_eq!(
            far.is_stale(&nav),
            Ok(false),
            "a far tile rebake leaves it alone"
        );

        nav.set_blocked(12, 8, true).expect("wall rebuilt");
        nav.set_blocked(12, 9, false).expect("new gap");
        nav.rebake_budget(usize::MAX);
        assert_eq!(path.is_stale(&nav), Ok(true));
        assert_eq!(far.is_stale(&nav), Ok(false));
        let again = path.refresh(&nav).expect("refreshes").expect("gap open");
        assert_eq!(again.is_stale(&nav), Ok(false));
        assert_eq!(
            path.refresh(&nav).expect("ok").map(|p| p.points().to_vec()),
            Some(again.points().to_vec())
        );
        assert_eq!(again.refresh(&nav), Ok(None), "a fresh path never repaths");
    }

    #[test]
    #[ignore = "timing gate: run under the release perf gate"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn query_p50_stays_under_a_fifth_of_a_millisecond() {
        let nav = TiledNav::bake_seeded(2040, 64, 64, 8, 150).expect("arena bakes");
        let mut state = 3u64;
        let mut times = Vec::new();
        while times.len() < 200 {
            let pick = |s: &mut u64| ((next(s) % 64) as usize, (next(s) % 64) as usize);
            let (start, goal) = (pick(&mut state), pick(&mut state));
            let begin = std::time::Instant::now();
            let Ok(Some(_)) = smooth_path(&nav, start, goal) else {
                continue;
            };
            times.push(begin.elapsed());
        }
        times.sort();
        assert!(
            times[times.len() / 2] <= std::time::Duration::from_micros(200),
            "p50 {:?}",
            times[times.len() / 2]
        );
    }
}
