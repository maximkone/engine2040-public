//! flow: tile flow-field bake + follow. One BFS bake serves N agents; 1 goal = 1 field.

use std::collections::VecDeque;

use engine_core::Fnv64;

/// Seeded arena walls for deterministic bakes: `seed` + size decides every wall.
///
/// `density_permille` is blocked cells per thousand (0 open, 1000 full);
/// values past 1000 saturate. Start `(0, 0)` and goal `(w - 1, h - 1)` stay
/// open so fighter follow and legion batches always have endpoints. Empty or
/// overflowing dims bake empty. Deterministic: `CounterRng` domain
/// `nav-grid`, one sample per cell in row-major order; same inputs give the
/// same walls on every platform. Bake-time: allocates the grid once.
pub fn seeded_blocked(seed: u64, w: usize, h: usize, density_permille: u32) -> Vec<bool> {
    let Some(cells) = w.checked_mul(h) else {
        return Vec::new();
    };
    if cells == 0 {
        return Vec::new();
    }
    let density = density_permille.min(1000);
    if density == 0 {
        return vec![false; cells];
    }
    let rng = engine_core::CounterRng::new(seed).fork_named("nav-grid");
    let cap = u64::from(density);
    let mut blocked = Vec::with_capacity(cells);
    for index in 0..cells {
        let x = index % w;
        let y = index / w;
        if (x == 0 && y == 0) || (x + 1 == w && y + 1 == h) {
            blocked.push(false);
            continue;
        }
        // Widening conversion: cell indices always fit u64, so the fallback
        // never runs; `unwrap_or` keeps the bake total without a panic site.
        let counter = u64::try_from(index).unwrap_or(u64::MAX);
        let roll = rng.sample(counter) % 1000;
        blocked.push(roll < cap);
    }
    blocked
}

/// Bake a distance field from `goal`: goal=0, neighbors+1, unreachable=-1.
/// 4-dir, deterministic neighbor order (E,W,S,N). `blocked` is row-major.
pub fn bake_flow(blocked: &[bool], w: usize, h: usize, goal: (usize, usize)) -> Vec<i32> {
    let mut dist = vec![-1i32; w * h];
    let (gx, gy) = goal;
    if gx >= w || gy >= h || blocked[gy * w + gx] {
        return dist;
    }
    let mut q = VecDeque::new();
    dist[gy * w + gx] = 0;
    q.push_back((gx, gy));
    while let Some((x, y)) = q.pop_front() {
        let d = dist[y * w + x];
        for (nx, ny) in [
            (x.wrapping_add(1), y),
            (x.wrapping_sub(1), y),
            (x, y.wrapping_add(1)),
            (x, y.wrapping_sub(1)),
        ] {
            if nx >= w || ny >= h {
                continue;
            }
            let ni = ny * w + nx;
            if blocked[ni] || dist[ni] != -1 {
                continue;
            }
            dist[ni] = d + 1;
            q.push_back((nx, ny));
        }
    }
    dist
}

/// Follow one step downhill: neighbor with smallest non-negative dist.
/// Tie-break order: N,S,W,E for determinism. None at goal or trapped.
pub fn follow(flow: &[i32], w: usize, h: usize, pos: (usize, usize)) -> Option<(usize, usize)> {
    let (x, y) = pos;
    if x >= w || y >= h {
        return None;
    }
    let cur = flow[y * w + x];
    if cur <= 0 {
        return None;
    }
    let mut best: Option<(i32, usize, usize)> = None;
    // N,S,W,E tie-break (fixed order, first wins on equal dist)
    for (nx, ny) in [
        (x, y.wrapping_sub(1)),
        (x, y.wrapping_add(1)),
        (x.wrapping_sub(1), y),
        (x.wrapping_add(1), y),
    ] {
        if nx >= w || ny >= h {
            continue;
        }
        let d = flow[ny * w + nx];
        if d < 0 {
            continue;
        }
        if best.map(|(bd, _, _)| d < bd).unwrap_or(true) {
            best = Some((d, nx, ny));
        }
    }
    best.map(|(_, nx, ny)| (nx, ny))
}

/// FNV-1a64 over the distance field (bake SHA gate).
pub fn flow_hash(flow: &[i32]) -> u64 {
    let mut h = Fnv64::new();
    for v in flow {
        h.write_i32(*v);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_bake_manhattan() {
        let blocked = vec![false; 5 * 5];
        let f = bake_flow(&blocked, 5, 5, (4, 4));
        assert_eq!(f[0], 8);
        assert_eq!(f[4 * 5 + 4], 0);
        assert_eq!(flow_hash(&f), flow_hash(&bake_flow(&blocked, 5, 5, (4, 4))));
    }

    #[test]
    fn flow_hash_is_fnv1a_over_little_endian_cells() {
        // Oracle: FNV-1a 64 over the little-endian i32 bytes of these cells,
        // computed outside this crate.
        let cells = [0, 1, -1, 258, i32::MIN];
        assert_eq!(flow_hash(&cells), 0x5199_0395_dfa0_b16d);
    }

    #[test]
    fn wall_blocks_flow() {
        let mut blocked = vec![false; 5 * 5];
        for y in 0..5 {
            blocked[y * 5 + 2] = true;
        }
        let f = bake_flow(&blocked, 5, 5, (4, 4));
        assert_eq!(f[0], -1);
        assert_eq!(f[4 * 5 + 4], 0);
    }

    #[test]
    fn follow_reaches_goal() {
        let blocked = vec![false; 5 * 5];
        let f = bake_flow(&blocked, 5, 5, (4, 4));
        let mut p = (0usize, 0usize);
        for _ in 0..16 {
            if let Some(n) = follow(&f, 5, 5, p) {
                p = n;
            } else {
                break;
            }
        }
        assert_eq!(p, (4, 4));
    }
}
