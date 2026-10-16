//! astar: 8-dir grid A* over a caller-owned blocked grid. Pure query, no bake.
//! ponytail: second A* copy lives in world/src/lib.rs; dedupe needs a decision note.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// 8-dir steps: (dx, dy, cost). Orthogonal 10, diagonal 14. Fixed order.
const DIRS: [(i32, i32, u32); 8] = [
    (1, 0, 10),
    (0, 1, 10),
    (-1, 0, 10),
    (0, -1, 10),
    (1, 1, 14),
    (-1, 1, 14),
    (-1, -1, 14),
    (1, -1, 14),
];

/// 8-dir A* over a row-major `blocked` grid.
///
/// Costs 10 orthogonal / 14 diagonal, Chebyshev x10 heuristic, no diagonal
/// corner-cutting (both orthogonal neighbors must be free). The open set is
/// ordered by (f, h, y, x), so ties break deterministically. Returns the
/// inclusive cell path, or `None` when unreachable (sealed, blocked or
/// out-of-bounds endpoints, empty grid).
pub fn astar(
    blocked: &[bool],
    w: usize,
    h: usize,
    start: (usize, usize),
    goal: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    let (sx, sy) = start;
    let (gx, gy) = goal;
    if w == 0 || h == 0 || blocked.len() < w.saturating_mul(h) {
        return None;
    }
    if sx >= w || sy >= h || gx >= w || gy >= h {
        return None;
    }
    let (si, gi) = (sy * w + sx, gy * w + gx);
    if blocked[si] || blocked[gi] {
        return None;
    }
    if si == gi {
        return Some(vec![start]);
    }
    // Admissible + consistent for 10/14 costs: never overestimates.
    let heur = |x: usize, y: usize| -> u32 {
        let (dx, dy) = (x.abs_diff(gx) as u32, y.abs_diff(gy) as u32);
        dx.max(dy) * 10
    };
    let mut g = vec![u32::MAX; w * h];
    let mut parent: Vec<Option<usize>> = vec![None; w * h];
    let mut closed = vec![false; w * h];
    // (f, h, idx): idx == y * w + x, so idx order == (y, x) order.
    let mut open: BinaryHeap<Reverse<(u32, u32, usize)>> = BinaryHeap::new();
    g[si] = 0;
    open.push(Reverse((heur(sx, sy), heur(sx, sy), si)));
    while let Some(Reverse((_, _, ci))) = open.pop() {
        if closed[ci] {
            continue;
        }
        closed[ci] = true;
        if ci == gi {
            let mut rev = vec![gi];
            let mut c = gi;
            while let Some(p) = parent[c] {
                rev.push(p);
                c = p;
            }
            rev.reverse();
            return Some(rev.into_iter().map(|i| (i % w, i / w)).collect());
        }
        let (cx, cy) = (ci % w, ci / w);
        for (dx, dy, cost) in DIRS {
            let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            let ni = ny * w + nx;
            if blocked[ni] || closed[ni] {
                continue;
            }
            if dx != 0 && dy != 0 && (blocked[cy * w + nx] || blocked[ny * w + cx]) {
                continue;
            }
            let ng = g[ci].saturating_add(cost);
            if ng < g[ni] {
                g[ni] = ng;
                parent[ni] = Some(ci);
                open.push(Reverse((ng.saturating_add(heur(nx, ny)), heur(nx, ny), ni)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_16_diagonal_is_16() {
        let blocked = vec![false; 16 * 16];
        let p = astar(&blocked, 16, 16, (0, 0), (15, 15)).expect("open grid must have path");
        assert_eq!(p.len(), 16);
        assert_eq!((p[0], p[p.len() - 1]), ((0, 0), (15, 15)));
        for w in p.windows(2) {
            assert_eq!(
                (w[0].0.abs_diff(w[1].0), w[0].1.abs_diff(w[1].1)),
                (1, 1),
                "open 8-dir path is all-diagonal: {p:?}"
            );
        }
        assert_eq!(astar(&blocked, 16, 16, (0, 0), (15, 15)), Some(p));
    }

    #[test]
    fn single_block_detours_without_corner_cut() {
        let mut blocked = vec![false; 16 * 16];
        blocked[7 * 16 + 7] = true;
        let p = astar(&blocked, 16, 16, (0, 0), (15, 15)).expect("one block must not seal 16x16");
        assert_eq!((p[0], p[p.len() - 1]), ((0, 0), (15, 15)));
        assert!(p.iter().all(|(x, y)| !blocked[y * 16 + x]));
        for w in p.windows(2) {
            let cuts = (w[1].0 as i32 - w[0].0 as i32) != 0 && (w[1].1 as i32 - w[0].1 as i32) != 0;
            if cuts {
                assert!(!blocked[w[0].1 * 16 + w[1].0], "cut corner: {w:?}");
                assert!(!blocked[w[1].1 * 16 + w[0].0], "cut corner: {w:?}");
            }
        }
        assert_eq!(astar(&blocked, 16, 16, (0, 0), (15, 15)), Some(p));
    }

    #[test]
    fn blocked_diagonal_forces_orthogonal_detour() {
        // 2x2 with (1,0) blocked: (0,0)->(1,1) would cut the corner.
        let mut blocked = vec![false; 2 * 2];
        blocked[1] = true;
        assert_eq!(
            astar(&blocked, 2, 2, (0, 0), (1, 1)),
            Some(vec![(0, 0), (0, 1), (1, 1)])
        );
    }

    #[test]
    fn sealed_wall_returns_none() {
        let mut blocked = vec![false; 5 * 5];
        for y in 0..5 {
            blocked[y * 5 + 2] = true;
        }
        assert_eq!(astar(&blocked, 5, 5, (0, 0), (4, 4)), None);
    }

    #[test]
    fn endpoints_and_degenerate() {
        let mut blocked = vec![false; 3 * 3];
        blocked[0] = true;
        assert_eq!(astar(&blocked, 3, 3, (0, 0), (2, 2)), None);
        blocked[0] = false;
        blocked[2 * 3 + 2] = true;
        assert_eq!(astar(&blocked, 3, 3, (0, 0), (2, 2)), None);
        let open = vec![false; 3 * 3];
        assert_eq!(astar(&open, 3, 3, (1, 1), (1, 1)), Some(vec![(1, 1)]));
        assert_eq!(astar(&open, 3, 3, (0, 0), (3, 3)), None);
        assert_eq!(astar(&[false; 4], 2, 3, (0, 0), (1, 1)), None);
    }
}
