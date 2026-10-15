//! broadphase: uniform-grid + SAP sweep. Integer cells, sorted output, std-only.
//! The grid sorts (cell, body) keys instead of hashing, so no per-process
//! hash seed exists anywhere on the path and equal inputs give equal pairs.

use crate::cast;

/// Candidate pair from overlapping cells (sorted, deduped).
pub fn candidate_pairs(bodies: &[(f32, f32, f32, f32)], cell: f32) -> Vec<(usize, usize)> {
    let cell = cell.max(0.001);
    let mut keys: Vec<((i32, i32), usize)> = Vec::with_capacity(bodies.len() * 4);
    for (i, (x, y, hx, hy)) in bodies.iter().enumerate() {
        let (x0, x1) = (
            cast::floor_to_i32((x - hx) / cell),
            cast::floor_to_i32((x + hx) / cell),
        );
        let (y0, y1) = (
            cast::floor_to_i32((y - hy) / cell),
            cast::floor_to_i32((y + hy) / cell),
        );
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                keys.push(((cx, cy), i));
            }
        }
    }
    // Sorted keys put each cell's bodies in one ascending run.
    keys.sort_unstable();
    let mut out: Vec<(usize, usize)> = vec![];
    for run in keys.chunk_by(|a, b| a.0 == b.0) {
        for (k, &(_, p)) in run.iter().enumerate() {
            for &(_, q) in &run[k + 1..] {
                out.push((p, q));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// SAP sweep on x-axis: sort by min-x, active window by max-x. Sorted output.
/// `total_cmp` gives NaN bounds a fixed place instead of a panic.
pub fn sweep_pairs(mins: &[f32], maxs: &[f32]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..mins.len()).collect();
    order.sort_by(|a, b| mins[*a].total_cmp(&mins[*b]).then(a.cmp(b)));
    let mut out = vec![];
    for (k, &i) in order.iter().enumerate() {
        for &j in &order[k + 1..] {
            if mins[j] > maxs[i] {
                break;
            }
            let (p, q) = if i < j { (i, j) } else { (j, i) };
            out.push((p, q));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

// Idea: jcornaz/bvh-arena (Unlicense, https://github.com/jcornaz/bvh-arena): arena-tree BVH,
// minimal-area-growth insert + for_each_overlapping_pair walk; integer-cm port, sweep stays fallback.
struct BvhN {
    lo: i32,
    hi: i32,
    l: usize,
    r: usize,
    id: Option<usize>,
}
/// BVH pair walk over integer-cm intervals; same pair set as sweep_pairs on valid inputs.
#[rustfmt::skip] // dense on purpose: holds the <=40-line port budget
pub fn bvh_pairs(mins: &[f32], maxs: &[f32]) -> Vec<(usize, usize)> {
    let n = mins.len().min(maxs.len()); if n < 2 { return vec![]; }
    let (cm_lo, cm_hi) = (|v: f32| cast::floor_to_i32(v * 100.0), |v: f32| cast::ceil_to_i32(v * 100.0));
    let mut t: Vec<BvhN> = Vec::with_capacity(2 * n); let mut root = 0;
    let (l0, h0) = (cm_lo(mins[0]), cm_hi(maxs[0]));
    t.push(BvhN { lo: l0.min(h0), hi: l0.max(h0), l: usize::MAX, r: usize::MAX, id: Some(0) });
    for i in 1..n {
        let (mut lo, mut hi) = (cm_lo(mins[i]), cm_hi(maxs[i])); if lo > hi { core::mem::swap(&mut lo, &mut hi); }
        let mut c = root; let mut path: Vec<usize> = vec![];
        while t[c].id.is_none() {
            path.push(c); let (l, r) = (t[c].l, t[c].r);
            let grow = |k: usize| (i64::from(hi.max(t[k].hi)) - i64::from(lo.min(t[k].lo))) - (i64::from(t[k].hi) - i64::from(t[k].lo));
            let (gl, gr) = (grow(l), grow(r));
            let llen = i64::from(t[l].hi) - i64::from(t[l].lo); let rlen = i64::from(t[r].hi) - i64::from(t[r].lo);
            c = if gl < gr || (gl == gr && llen <= rlen) { l } else { r };
        }
        let leaf = t.len(); t.push(BvhN { lo, hi, l: usize::MAX, r: usize::MAX, id: Some(i) });
        let node = t.len(); t.push(BvhN { lo: t[c].lo.min(lo), hi: t[c].hi.max(hi), l: c, r: leaf, id: None });
        if path.is_empty() { root = node; } else { let p = path[path.len() - 1]; if t[p].l == c { t[p].l = node; } else { t[p].r = node; } }
        for p in path.iter().rev() { t[*p].lo = t[*p].lo.min(lo); t[*p].hi = t[*p].hi.max(hi); }
    }
    let mut st = vec![(root, root)]; let mut out: Vec<(usize, usize)> = vec![];
    while let Some((x, y)) = st.pop() {
        if t[x].hi < t[y].lo || t[y].hi < t[x].lo { continue; }
        if x == y { if t[x].id.is_none() { let (l, r) = (t[x].l, t[x].r); st.push((l, l)); st.push((l, r)); st.push((r, r)); } }
        else if let (Some(a), Some(b)) = (t[x].id, t[y].id) { if mins[a] <= maxs[b] && mins[b] <= maxs[a] { out.push((a.min(b), a.max(b))); } }
        else if t[y].id.is_some() || (t[x].id.is_none() && i64::from(t[x].hi) - i64::from(t[x].lo) >= i64::from(t[y].hi) - i64::from(t[y].lo)) { let (l, r) = (t[x].l, t[x].r); st.push((l, y)); st.push((r, y)); }
        else { let (l, r) = (t[y].l, t[y].r); st.push((x, l)); st.push((x, r)); }
    }
    out.sort_unstable(); out.dedup(); out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_separates_far() {
        let b = [
            (0.0f32, 0.0, 0.5, 0.5),
            (0.4, 0.0, 0.5, 0.5),
            (50.0, 50.0, 0.5, 0.5),
        ];
        let p = candidate_pairs(&b, 2.0);
        assert!(p.contains(&(0, 1)), "{p:?}");
        assert!(!p.iter().any(|(a, b)| *a == 2 || *b == 2), "{p:?}");
    }

    #[test]
    fn sweep_window() {
        let (mins, maxs) = (vec![0.0, 0.5, 10.0], vec![1.0, 1.5, 11.0]);
        let p = sweep_pairs(&mins, &maxs);
        assert_eq!(p, vec![(0, 1)]);
    }

    #[test]
    fn determinism_x2() {
        let b = [
            (0.0f32, 0.0, 1.0, 1.0),
            (0.5, 0.5, 1.0, 1.0),
            (9.0, 9.0, 0.5, 0.5),
        ];
        assert_eq!(candidate_pairs(&b, 2.0), candidate_pairs(&b, 2.0));
        let (mins, maxs) = (vec![0.0, 0.5], vec![1.0, 2.0]);
        assert_eq!(sweep_pairs(&mins, &maxs), sweep_pairs(&mins, &maxs));
    }

    #[test]
    fn grid_pairs_match_brute_force_and_ignore_input_order() {
        // Scattered boxes, some spanning several cells, against an all-pairs
        // cell-overlap oracle; reversing the input must relabel, not reorder.
        let mut b = Vec::new();
        let mut s = 7u32;
        for _ in 0..40 {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (s >> 8) as f32 / 16_777_216.0 * 20.0;
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let y = (s >> 8) as f32 / 16_777_216.0 * 20.0;
            b.push((x, y, 0.3 + (s % 5) as f32 * 0.4, 0.5));
        }
        let cell = 2.0f32;
        let span = |v: f32, h: f32| {
            (
                ((v - h) / cell).floor() as i32,
                ((v + h) / cell).floor() as i32,
            )
        };
        let mut want = Vec::new();
        for i in 0..b.len() {
            for j in i + 1..b.len() {
                let (ax, bx) = (span(b[i].0, b[i].2), span(b[j].0, b[j].2));
                let (ay, by) = (span(b[i].1, b[i].3), span(b[j].1, b[j].3));
                if ax.0 <= bx.1 && bx.0 <= ax.1 && ay.0 <= by.1 && by.0 <= ay.1 {
                    want.push((i, j));
                }
            }
        }
        assert!(want.len() > 10, "fixture must overlap: {}", want.len());
        assert_eq!(candidate_pairs(&b, cell), want);
        let n = b.len() - 1;
        let reversed: Vec<_> = b.iter().rev().copied().collect();
        let mut relabeled: Vec<_> = candidate_pairs(&reversed, cell)
            .into_iter()
            .map(|(p, q)| (n - q, n - p))
            .collect();
        relabeled.sort_unstable();
        assert_eq!(relabeled, want);
    }

    #[test]
    fn nan_bounds_do_not_panic_the_sweep() {
        let pairs = sweep_pairs(&[0.0, f32::NAN, 0.5], &[1.0, 2.0, 1.5]);
        assert!(pairs.contains(&(0, 2)), "{pairs:?}");
    }
}

#[cfg(test)]
mod bvh_oracle_tests {
    use super::*;
    #[test]
    fn bvh_matches_sweep_5k_crowded() {
        let mut s = 0x2040u32;
        let mut next = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / 16_777_216.0
        };
        let (mut mins, mut maxs) = (Vec::with_capacity(5000), Vec::with_capacity(5000));
        for _ in 0..5000 {
            let m = next() * 50.0;
            mins.push(m);
            maxs.push(m + 0.5 + next() * 1.5);
        }
        let (a, b) = (bvh_pairs(&mins, &maxs), sweep_pairs(&mins, &maxs));
        assert!(
            a.len() > 10_000,
            "crowded fixture must overlap: {}",
            a.len()
        );
        assert_eq!(a, b);
    }
}
