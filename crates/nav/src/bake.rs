//! bake: tiled offline nav bake + flow cache + validator. Runtime reads only, never BFS.

use crate::flow::{bake_flow, flow_hash};

/// Tile edge in cells. Newtype so bare numbers never swap meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileEdge(pub usize);

/// Baked goal cell. Newtype so goal/pos args never swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Goal {
    /// Goal x cell.
    pub x: usize,
    /// Goal y cell.
    pub y: usize,
}

/// Query cell. Newtype so goal/pos args never swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Cell x.
    pub x: usize,
    /// Cell y.
    pub y: usize,
}

/// One baked tile: tile indices + occupancy (any covered cell blocked).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    /// Tile x index.
    pub tx: usize,
    /// Tile y index.
    pub ty: usize,
    /// True when any covered cell is blocked.
    pub occupied: bool,
}

/// Bounded flow cache: one distance field per baked goal, FIFO evict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowCache {
    goals: Vec<Goal>,
    fields: Vec<Vec<i32>>,
    cap: usize,
}

impl FlowCache {
    /// Empty cache holding at most `cap` fields (cap 0 becomes 1).
    pub fn new(cap: usize) -> Self {
        Self {
            goals: Vec::new(),
            fields: Vec::new(),
            cap: cap.max(1),
        }
    }
    /// Cached field for `goal`, if baked.
    pub fn get(&self, goal: &Goal) -> Option<&[i32]> {
        self.goals
            .iter()
            .position(|g| g == goal)
            .map(|i| &*self.fields[i])
    }
    /// Insert (or refresh) `goal`; evicts oldest past cap.
    pub fn insert(&mut self, goal: Goal, field: Vec<i32>) {
        if let Some(i) = self.goals.iter().position(|g| *g == goal) {
            self.fields[i] = field;
            return;
        }
        self.goals.push(goal);
        self.fields.push(field);
        while self.goals.len() > self.cap {
            self.goals.remove(0);
            self.fields.remove(0);
        }
    }
    /// Baked goal count.
    pub fn len(&self) -> usize {
        self.goals.len()
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
    }
}

/// Offline-baked nav: grid + tile occupancy + cached flow fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakedNav {
    w: usize,
    h: usize,
    edge: usize,
    blocked: Vec<bool>,
    tiles: Vec<Tile>,
    flows: FlowCache,
}

impl BakedNav {
    /// Bake tiles offline from row-major `blocked` (edge clamped >= 1).
    pub fn bake(blocked: &[bool], w: usize, h: usize, edge: &TileEdge) -> Self {
        let e = edge.0.max(1);
        Self {
            w,
            h,
            edge: e,
            blocked: blocked.to_vec(),
            tiles: bake_tiles(blocked, w, h, e),
            flows: FlowCache::new(8),
        }
    }
    /// Grid width in cells.
    pub fn width(&self) -> usize {
        self.w
    }
    /// Grid height in cells.
    pub fn height(&self) -> usize {
        self.h
    }
    /// Baked tile edge.
    pub fn edge(&self) -> usize {
        self.edge
    }
    /// Baked tiles (row-major over the tile grid).
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
    /// Flow cache (read-only at runtime).
    pub fn flows(&self) -> &FlowCache {
        &self.flows
    }
    /// Bake one goal flow field offline into the cache; runtime reads it.
    pub fn bake_goal(&mut self, goal: &Goal) -> &[i32] {
        let field = bake_flow(&self.blocked, self.w, self.h, (goal.x, goal.y));
        self.flows.insert(*goal, field);
        match self.flows.get(goal) {
            Some(f) => f,
            None => &[],
        }
    }
    /// Runtime-only step: next cell toward baked `goal`, never bakes.
    pub fn direction(&self, goal: &Goal, pos: &Cell) -> Option<Cell> {
        let flow = self.flows.get(goal)?;
        next_downhill(flow, self.w, self.h, pos)
    }
    /// Bake SHA gate: lane hash over dims + grid + goals + cached flows.
    pub fn bake_hash(&self) -> u64 {
        flow_hash(&self.bake_words())
    }
    /// Validator: dims + tile coverage + flow shape. Empty vec is green.
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        check_dims(self, &mut errs);
        check_tiles(self, &mut errs);
        check_flows(self, &mut errs);
        errs
    }
    /// Hash words: dims, edge, blocked bits, then each goal + its field.
    fn bake_words(&self) -> Vec<i32> {
        let mut words = Vec::new();
        words.push(self.w as i32);
        words.push(self.h as i32);
        words.push(self.edge as i32);
        for b in &self.blocked {
            words.push(i32::from(*b));
        }
        for g in &self.flows.goals {
            words.push(g.x as i32);
            words.push(g.y as i32);
        }
        for f in &self.flows.fields {
            words.extend_from_slice(f);
        }
        words
    }
}

/// Tile bake helper: one [`Tile`] per covered tile with occupancy OR.
fn bake_tiles(blocked: &[bool], w: usize, h: usize, edge: usize) -> Vec<Tile> {
    let mut out = Vec::new();
    if w == 0 || h == 0 {
        return out;
    }
    for ty in 0..h.div_ceil(edge) {
        for tx in 0..w.div_ceil(edge) {
            out.push(Tile {
                tx,
                ty,
                occupied: tile_occupied(blocked, w, h, edge, tx, ty),
            });
        }
    }
    out
}

/// True when any cell covered by tile `(tx, ty)` is blocked.
fn tile_occupied(blocked: &[bool], w: usize, h: usize, edge: usize, tx: usize, ty: usize) -> bool {
    for y in (ty * edge)..((ty + 1) * edge).min(h) {
        for x in (tx * edge)..((tx + 1) * edge).min(w) {
            if blocked[y * w + x] {
                return true;
            }
        }
    }
    false
}

/// Runtime downhill step: smallest non-negative neighbor (N,S,W,E wins).
fn next_downhill(flow: &[i32], w: usize, h: usize, pos: &Cell) -> Option<Cell> {
    if pos.x >= w || pos.y >= h {
        return None;
    }
    let cur = flow[pos.y * w + pos.x];
    if cur <= 0 {
        return None;
    }
    let mut best: Option<(i32, usize, usize)> = None;
    for (nx, ny) in [
        (pos.x, pos.y.wrapping_sub(1)),
        (pos.x, pos.y.wrapping_add(1)),
        (pos.x.wrapping_sub(1), pos.y),
        (pos.x.wrapping_add(1), pos.y),
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
    best.map(|(_, nx, ny)| Cell { x: nx, y: ny })
}

/// Validator part: dims sane and grid length matches.
fn check_dims(nav: &BakedNav, errs: &mut Vec<String>) {
    if nav.w == 0 || nav.h == 0 {
        errs.push("bake: zero grid dims".to_string());
    }
    if nav.edge == 0 {
        errs.push("bake: zero tile edge".to_string());
    }
    if nav.blocked.len() != nav.w * nav.h {
        errs.push("bake: blocked len != w*h".to_string());
    }
}

/// Validator part: tile count covers the grid and occupancy matches.
fn check_tiles(nav: &BakedNav, errs: &mut Vec<String>) {
    if nav.w == 0 || nav.h == 0 || nav.edge == 0 {
        return;
    }
    let expect = nav.w.div_ceil(nav.edge) * nav.h.div_ceil(nav.edge);
    if nav.tiles.len() != expect {
        errs.push("bake: tile count != coverage".to_string());
        return;
    }
    for t in &nav.tiles {
        let want = tile_occupied(&nav.blocked, nav.w, nav.h, nav.edge, t.tx, t.ty);
        if t.occupied != want {
            errs.push("bake: tile occupancy mismatch".to_string());
            return;
        }
    }
}

/// Validator part: every cached flow is shaped like a real BFS bake.
fn check_flows(nav: &BakedNav, errs: &mut Vec<String>) {
    for (i, goal) in nav.flows.goals.iter().enumerate() {
        check_one_flow(nav, goal, &nav.flows.fields[i], errs);
    }
}

/// Validator part: one cached flow — goal is 0, blocked is -1, downhill holds.
fn check_one_flow(nav: &BakedNav, goal: &Goal, flow: &[i32], errs: &mut Vec<String>) {
    if flow.len() != nav.w * nav.h {
        errs.push("bake: flow len != w*h".to_string());
        return;
    }
    if goal.x >= nav.w || goal.y >= nav.h {
        errs.push("bake: goal out of bounds".to_string());
        return;
    }
    if nav.blocked[goal.y * nav.w + goal.x] {
        errs.push("bake: goal on blocked cell".to_string());
        return;
    }
    if flow[goal.y * nav.w + goal.x] != 0 {
        errs.push("bake: goal dist != 0".to_string());
        return;
    }
    for (idx, cell_blocked) in nav.blocked.iter().enumerate() {
        if *cell_blocked && flow[idx] != -1 {
            errs.push("bake: blocked cell dist != -1".to_string());
            return;
        }
    }
    if !downhill_holds(flow, &nav.blocked, nav.w, nav.h) {
        errs.push("bake: flow not downhill-monotonic".to_string());
    }
}

/// True when every reachable non-goal cell has a neighbor with dist - 1.
fn downhill_holds(flow: &[i32], blocked: &[bool], w: usize, h: usize) -> bool {
    for y in 0..h {
        for x in 0..w {
            let d = flow[y * w + x];
            if d <= 0 || blocked[y * w + x] {
                continue;
            }
            if !has_downhill_neighbor(flow, blocked, w, h, x, y, d) {
                return false;
            }
        }
    }
    true
}

/// True when cell `(x, y)` has a 4-neighbor with distance `d - 1`.
fn has_downhill_neighbor(
    flow: &[i32],
    blocked: &[bool],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    d: i32,
) -> bool {
    for (nx, ny) in [
        (x.wrapping_sub(1), y),
        (x.wrapping_add(1), y),
        (x, y.wrapping_sub(1)),
        (x, y.wrapping_add(1)),
    ] {
        if nx >= w || ny >= h || blocked[ny * w + nx] {
            continue;
        }
        if flow[ny * w + nx] == d - 1 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_grid_and_flag_occupancy() {
        let mut blocked = vec![false; 5 * 5];
        blocked[6] = true;
        let nav = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        assert_eq!(nav.tiles().len(), 9);
        assert!(nav.tiles().iter().any(|t| t.occupied));
        assert!(nav.tiles().iter().any(|t| !t.occupied));
        assert!(nav.validate().is_empty());
    }

    #[test]
    fn bake_hash_stable_x2() {
        let blocked = vec![false; 5 * 5];
        let mut a = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        let mut b = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        a.bake_goal(&Goal { x: 4, y: 4 });
        b.bake_goal(&Goal { x: 4, y: 4 });
        assert_eq!(a.bake_hash(), b.bake_hash());
        assert_ne!(
            a.bake_hash(),
            BakedNav::bake(&blocked, 5, 5, &TileEdge(2)).bake_hash()
        );
    }

    #[test]
    fn flow_cache_hit_and_bounded_evict() {
        let blocked = vec![false; 4 * 4];
        let mut nav = BakedNav::bake(&blocked, 4, 4, &TileEdge(2));
        nav.flows = FlowCache::new(2);
        let g0 = Goal { x: 0, y: 0 };
        let g1 = Goal { x: 3, y: 0 };
        let g2 = Goal { x: 0, y: 3 };
        nav.bake_goal(&g0);
        nav.bake_goal(&g1);
        assert!(nav.flows().get(&g0).is_some());
        nav.bake_goal(&g2);
        assert_eq!(nav.flows().len(), 2);
        assert!(nav.flows().get(&g0).is_none());
        assert!(nav.flows().get(&g2).is_some());
    }

    #[test]
    fn validator_green_on_good_bake() {
        let blocked = vec![false; 5 * 5];
        let mut nav = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        nav.bake_goal(&Goal { x: 4, y: 4 });
        assert!(nav.validate().is_empty());
    }

    #[test]
    fn validator_red_on_blocked_goal() {
        let mut blocked = vec![false; 5 * 5];
        blocked[4 * 5 + 4] = true;
        let mut nav = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        nav.bake_goal(&Goal { x: 4, y: 4 });
        assert!(!nav.validate().is_empty());
    }

    #[test]
    fn runtime_query_reaches_goal() {
        let blocked = vec![false; 5 * 5];
        let mut nav = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        let goal = Goal { x: 4, y: 4 };
        nav.bake_goal(&goal);
        let mut p = Cell { x: 0, y: 0 };
        for _ in 0..16 {
            match nav.direction(&goal, &p) {
                Some(n) => p = n,
                None => break,
            }
        }
        assert_eq!(p, Cell { x: 4, y: 4 });
    }

    #[test]
    fn uncached_goal_returns_none() {
        let blocked = vec![false; 5 * 5];
        let nav = BakedNav::bake(&blocked, 5, 5, &TileEdge(2));
        let goal = Goal { x: 4, y: 4 };
        assert_eq!(nav.direction(&goal, &Cell { x: 0, y: 0 }), None);
    }
}
