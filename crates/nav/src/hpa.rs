//! hpa: hierarchical chunked A* beside flat astar. Pure query, no bake.
//! Owner nav; consumer NV6 funnel/legion walks (UNLOCKS NV6). Cells in/out, 10/14 costs.
//! Determinism integer-only fixed order, no wall clock. Errors: `None` on sealed/blocked/OOB.
//! PORT(JtotheThree/bevy_northstar@f4d85f6:src/hpa.rs+chunk.rs+graph.rs+astar.rs): abstract chunk search plus touched-chunk refine vs flat referee.
use std::collections::VecDeque;
/// Chunk edge in cells.
pub const HPA_CHUNK: usize = 8;
/// Cell path in walk order, inclusive of both ends.
pub type Walk = Vec<(usize, usize)>;
/// Scratch: chunk edge plus touched-chunk cache; start/goal virtualize as chunk members.
pub struct HpaScratch {
    /// Chunk edge in cells.
    pub chunk: usize,
    /// Touched chunks of the last abstract path in walk order.
    pub touched: Vec<usize>,
}
impl HpaScratch {
    /// Scratch for `chunk`-cell chunks; zero selects the default edge.
    pub fn new(chunk: usize) -> Self {
        Self {
            chunk: if chunk == 0 { HPA_CHUNK } else { chunk },
            touched: Vec::new(),
        }
    }
}
/// Chunk index of a cell.
fn chunk_at(x: usize, y: usize, ck: usize, cw: usize) -> usize {
    (y / ck).saturating_mul(cw).saturating_add(x / ck)
}
/// True when 4-neighbor chunks share one free orthogonal border pair.
fn edge_open(b: &[bool], w: usize, h: usize, ck: usize, a: usize, c: usize) -> bool {
    let cw = w.saturating_add(ck).saturating_sub(1) / ck;
    let (ax, ay) = (a % cw, a / cw);
    let (bx, by) = (c % cw, c / cw);
    if ay == by && ax.saturating_add(1) == bx {
        let xl = ax.saturating_add(1).saturating_mul(ck).saturating_sub(1);
        let xr = ax.saturating_add(1).saturating_mul(ck);
        for y in ay.saturating_mul(ck)..((ay.saturating_add(1).saturating_mul(ck)).min(h)) {
            if xl < w && xr < w {
                let li = y.saturating_mul(w).saturating_add(xl);
                let ri = y.saturating_mul(w).saturating_add(xr);
                if !b.get(li).copied().unwrap_or(true) && !b.get(ri).copied().unwrap_or(true) {
                    return true;
                }
            }
        }
    } else if ax == bx && ay.saturating_add(1) == by {
        let yt = ay.saturating_add(1).saturating_mul(ck).saturating_sub(1);
        let yb = ay.saturating_add(1).saturating_mul(ck);
        for x in ax.saturating_mul(ck)..((ax.saturating_add(1).saturating_mul(ck)).min(w)) {
            if yt < h && yb < h {
                let ti = yt.saturating_mul(w).saturating_add(x);
                let bi = yb.saturating_mul(w).saturating_add(x);
                if !b.get(ti).copied().unwrap_or(true) && !b.get(bi).copied().unwrap_or(true) {
                    return true;
                }
            }
        }
    } else {
        return false;
    }
    false
}
/// Breadth-first chunk path with fixed N,E,S,W order; uniform cost so BFS is optimal.
fn abstract_path(
    cw: usize,
    ch: usize,
    sc: usize,
    gc: usize,
    open: impl Fn(usize, usize) -> bool,
) -> Option<Vec<usize>> {
    if sc == gc {
        return Some(vec![sc]);
    }
    let mut par: Vec<Option<usize>> = vec![None; cw.saturating_mul(ch)];
    let mut seen = vec![false; cw.saturating_mul(ch)];
    let mut q = VecDeque::new();
    seen[sc] = true;
    q.push_back(sc);
    while let Some(ci) = q.pop_front() {
        if ci == gc {
            let mut rev = vec![gc];
            let mut c = gc;
            while let Some(p) = par[c] {
                rev.push(p);
                c = p;
            }
            rev.reverse();
            return Some(rev);
        }
        let (cx, cy) = (ci % cw, ci / cw);
        if cx + 1 < cw && !seen[ci + 1] && open(ci, ci + 1) {
            seen[ci + 1] = true;
            par[ci + 1] = Some(ci);
            q.push_back(ci + 1);
        }
        if cx > 0 && !seen[ci - 1] && open(ci, ci - 1) {
            seen[ci - 1] = true;
            par[ci - 1] = Some(ci);
            q.push_back(ci - 1);
        }
        if cy + 1 < ch && !seen[ci + cw] && open(ci, ci + cw) {
            seen[ci + cw] = true;
            par[ci + cw] = Some(ci);
            q.push_back(ci + cw);
        }
        if cy > 0 && !seen[ci - cw] && open(ci, ci - cw) {
            seen[ci - cw] = true;
            par[ci - cw] = Some(ci);
            q.push_back(ci - cw);
        }
    }
    None
}
/// Hierarchical chunked A* with caller scratch; refines touched chunks plus a one-ring.
/// Sealed corridors fall back to flat astar so length never differs from flat.
pub fn hpa_with_scratch(
    b: &[bool],
    w: usize,
    h: usize,
    s: (usize, usize),
    g: (usize, usize),
    sc: &mut HpaScratch,
) -> Option<Walk> {
    let (sx, sy) = s;
    let (gx, gy) = g;
    if w == 0 || h == 0 || b.len() < w.saturating_mul(h) || sx >= w || sy >= h || gx >= w || gy >= h
    {
        return None;
    }
    let ck = if sc.chunk == 0 { HPA_CHUNK } else { sc.chunk };
    let cw = w.saturating_add(ck).saturating_sub(1) / ck;
    let ch = h.saturating_add(ck).saturating_sub(1) / ck;
    if cw == 0 || ch == 0 {
        return crate::astar::astar(b, w, h, s, g);
    }
    let si = sy.saturating_mul(w).saturating_add(sx);
    let gi = gy.saturating_mul(w).saturating_add(gx);
    if b.get(si).copied().unwrap_or(true) || b.get(gi).copied().unwrap_or(true) {
        return None;
    }
    if si == gi {
        return Some(vec![s]);
    }
    let (ca, cb) = (chunk_at(sx, sy, ck, cw), chunk_at(gx, gy, ck, cw));
    if ca == cb {
        return crate::astar::astar(b, w, h, s, g);
    }
    let Some(path) = abstract_path(cw, ch, ca, cb, |a, c| edge_open(b, w, h, ck, a, c)) else {
        return crate::astar::astar(b, w, h, s, g);
    };
    sc.touched.clear();
    sc.touched.extend(path.iter().copied());
    let mut mask = b.to_vec();
    for (i, cell) in mask.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let c = chunk_at(x, y, ck, cw);
        let (cx, cy) = (c % cw, c / cw);
        let inside = sc
            .touched
            .iter()
            .copied()
            .any(|v| (v % cw).abs_diff(cx) <= 1 && (v / cw).abs_diff(cy) <= 1);
        if !inside {
            *cell = true;
        }
    }
    if let Some(p) = crate::astar::astar(&mask, w, h, s, g) {
        return Some(p);
    }
    crate::astar::astar(b, w, h, s, g)
}
/// Hierarchical chunked A* over a caller-owned blocked grid: abstract graph then touched refine.
pub fn hpa(b: &[bool], w: usize, h: usize, s: (usize, usize), g: (usize, usize)) -> Option<Walk> {
    hpa_with_scratch(b, w, h, s, g, &mut HpaScratch::new(HPA_CHUNK))
}
