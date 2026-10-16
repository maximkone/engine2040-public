//! scatter: seeded Poisson-disk points for bake-time scatter.
//! Board: WD15 (open: staged arena-ground bake, base -> wear -> scatter).
//! Owner: `engine-world`; bake-time only, never the tick or the hash.
//! Units: meters in (`w`, `h`, `min_dist`), `[x, y]` meters out.
//! Determinism: same `(seed, recipe, w, h, min_dist)` gives identical bytes.
//! Errors: [`ScatterErr`] refuses bad inputs before any allocation.
// PORT(Kromey/fast_poisson@5c8273cb790c600219fc32121f70458220ac9dfe:src/lib.rs Bridson sampler): reimplemented, zero bytes vendored.
use crate::blockout::effective_seed;
use engine_core::Rng;

/// Bridson tries per active point; grid ceiling refuses wild extents pre-alloc.
const TRIES: u32 = 30;
const MAX_CELLS: usize = 1_048_576;

/// Why a scatter request was refused. Nothing is allocated on refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScatterErr {
    /// `min_dist` is not finite positive.
    BadMinDist,
    /// `w` or `h` is not finite positive, or the grid would exceed [`MAX_CELLS`].
    BadExtent,
}

impl std::fmt::Display for ScatterErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScatterErr::BadMinDist => write!(f, "scatter bad min_dist"),
            ScatterErr::BadExtent => write!(f, "scatter bad extent"),
        }
    }
}

impl std::error::Error for ScatterErr {}

/// Deterministic `f32` in `[0, 1)` from the top 53 stream bits; bake-time only.
fn unit(rng: &mut Rng) -> f32 {
    ((rng.next_u64() >> 11) as f64 / 9_007_199_254_740_992.0) as f32
}

/// Grid index for `points.len()`: refuses values that do not fit `i32`.
/// Units: count in, grid index out. Errors: [`ScatterErr::BadExtent`] when
/// the count exceeds `i32::MAX`; bake-time only, never the tick.
fn point_index(len: usize) -> Result<i32, ScatterErr> {
    i32::try_from(len).map_err(|_| ScatterErr::BadExtent)
}

/// Bridson Poisson-disk points over `[0, w) x [0, h)` with pairwise distance
/// at least `min_dist`. Units: meters in and out. Seed: `Rng` at
/// `blockout::effective_seed(seed, recipe)`, so recipes split like blockout
/// bakes. Bake-time only: the annulus proposal uses only `+ - * /` and one
/// `sqrt` for the unit direction, never `sin`/`cos` and never in the tick.
/// Errors: [`ScatterErr::BadMinDist`] for non-finite or
/// non-positive `min_dist`, [`ScatterErr::BadExtent`] for non-finite or
/// non-positive extents, checked before any allocation.
pub fn poisson_disk(
    seed: u64,
    recipe: &str,
    w: f32,
    h: f32,
    min_dist: f32,
) -> Result<Vec<[f32; 2]>, ScatterErr> {
    if !min_dist.is_finite() || min_dist <= 0.0 {
        return Err(ScatterErr::BadMinDist);
    }
    if !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return Err(ScatterErr::BadExtent);
    }
    let cell = min_dist / core::f32::consts::SQRT_2;
    let (gw, gh) = ((w / cell).ceil() as usize, (h / cell).ceil() as usize);
    if gw == 0 || gh == 0 {
        return Err(ScatterErr::BadExtent);
    }
    let cells = gw.checked_mul(gh).ok_or(ScatterErr::BadExtent)?;
    if cells > MAX_CELLS {
        return Err(ScatterErr::BadExtent);
    }
    let mut rng = Rng::new(effective_seed(seed, recipe));
    let mut grid = vec![-1i32; cells];
    let mut points: Vec<[f32; 2]> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let r2 = min_dist * min_dist;
    let push = |p: [f32; 2],
                grid: &mut [i32],
                points: &mut Vec<[f32; 2]>,
                active: &mut Vec<usize>|
     -> Result<(), ScatterErr> {
        let gx = (p[0] / cell) as usize;
        let gy = (p[1] / cell) as usize;
        let idx = point_index(points.len())?;
        grid[gy * gw + gx] = idx;
        points.push(p);
        active.push(points.len() - 1);
        Ok(())
    };
    push(
        [unit(&mut rng) * w, unit(&mut rng) * h],
        &mut grid,
        &mut points,
        &mut active,
    )?;
    while !active.is_empty() {
        let pick = (rng.next_range(0, active.len() as u64) as usize).min(active.len() - 1);
        let anchor = points[active[pick]];
        let mut accepted: Option<[f32; 2]> = None;
        for _ in 0..TRIES {
            let radius = min_dist * (1.0 + unit(&mut rng));
            let ux = unit(&mut rng) * 2.0 - 1.0;
            let uy = unit(&mut rng) * 2.0 - 1.0;
            let len2 = ux * ux + uy * uy;
            if !(1.0e-12..=1.0).contains(&len2) {
                continue;
            }
            // `sqrt` rounds once per IEEE-754 on every platform, while
            // `sin`/`cos` vary by libm and would move the bake.
            let inv = 1.0 / len2.sqrt();
            let cand = [anchor[0] + radius * ux * inv, anchor[1] + radius * uy * inv];
            if cand[0] < 0.0 || cand[0] >= w || cand[1] < 0.0 || cand[1] >= h {
                continue;
            }
            let (gx, gy) = ((cand[0] / cell) as usize, (cand[1] / cell) as usize);
            let mut clear = true;
            for cy in gy.saturating_sub(2)..=(gy + 2).min(gh - 1) {
                for cx in gx.saturating_sub(2)..=(gx + 2).min(gw - 1) {
                    let at = grid[cy * gw + cx];
                    if at < 0 {
                        continue;
                    }
                    let q = points[at as usize];
                    let (dx, dy) = (q[0] - cand[0], q[1] - cand[1]);
                    if dx * dx + dy * dy < r2 {
                        clear = false;
                        break;
                    }
                }
                if !clear {
                    break;
                }
            }
            if clear {
                accepted = Some(cand);
                break;
            }
        }
        if let Some(p) = accepted {
            push(p, &mut grid, &mut points, &mut active)?;
        } else {
            active.swap_remove(pick);
        }
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spike() -> Vec<[f32; 2]> {
        poisson_disk(2040, "scatter-pine", 100.0, 100.0, 2.0).expect("spike fixture bakes")
    }

    fn min_gap(pts: &[[f32; 2]]) -> f32 {
        let mut gap = f32::INFINITY;
        for i in 0..pts.len() {
            for q in pts.iter().skip(i + 1) {
                let (dx, dy) = (pts[i][0] - q[0], pts[i][1] - q[1]);
                gap = gap.min((dx * dx + dy * dy).sqrt());
            }
        }
        gap
    }

    #[test]
    fn spike_count_gap_and_reseed() {
        let pts = spike();
        let n = pts.len();
        assert_eq!(n, 1550, "spike count drifted: count {n}");
        assert!(min_gap(&pts) >= 2.0, "pair closer than 2.0 m");
        assert_eq!(pts, spike(), "same seed must rerun byte-identical");
        let other = poisson_disk(7, "scatter-pine", 100.0, 100.0, 2.0).expect("seed 7 bakes");
        assert_ne!(pts, other, "seed 7 must give a different set");
        let recipe = poisson_disk(2040, "scatter-oak", 100.0, 100.0, 2.0).expect("recipe bakes");
        assert_ne!(pts, recipe, "recipe must split like blockout");
    }

    #[test]
    fn bad_inputs_refuse_before_allocating() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let got = poisson_disk(1, "r", 10.0, 10.0, bad);
            assert_eq!(got, Err(ScatterErr::BadMinDist));
        }
        for bad in [f32::NAN, 0.0, -4.0, f32::INFINITY] {
            assert_eq!(
                poisson_disk(1, "r", bad, 10.0, 1.0),
                Err(ScatterErr::BadExtent)
            );
            assert_eq!(
                poisson_disk(1, "r", 10.0, bad, 1.0),
                Err(ScatterErr::BadExtent)
            );
        }
        assert_eq!(ScatterErr::BadMinDist.to_string(), "scatter bad min_dist");
        assert_eq!(ScatterErr::BadExtent.to_string(), "scatter bad extent");
    }

    #[test]
    fn point_index_overflow_refuses() {
        assert_eq!(point_index(0), Ok(0));
        assert_eq!(point_index(1_048_576), Ok(1_048_576));
        assert_eq!(point_index(usize::MAX), Err(ScatterErr::BadExtent));
    }
}
