//! quant: quantized nav runtime for floor queries (spike, 2026-10-19).
//!
//! Consumer: an offline advisory helper beside [`crate::bake`] and
//! [`crate::flow`]. The spike quantizes one baked flow field into a small
//! integer runtime and answers the same step and walkability queries from it:
//! occupancy drops to one bit per cell, the downhill direction to four bits,
//! and the coarse distance channel to one byte with a cap. Nothing here runs in
//! a tick, so no float, clock or map reaches an answer.
//!
//! Contract: the direction channel is **exact**. [`QuantNav::direction`] returns
//! the same cell [`crate::flow::follow`] and [`crate::bake::BakedNav::direction`]
//! return, cell for cell, on the same bake, at a fraction of the bytes. The
//! distance channel is lossy on purpose: a field deeper than [`LEVEL_MAX`]
//! steps reads [`LEVEL_MAX`] and sets [`QuantNav::saturated`], so a caller that
//! needs exact distance keeps the i32 field. A `blocked` slice whose length is
//! not `w * h` reads fully blocked: the runtime refuses instead of guessing.
//!
//! Determinism: integer only, one pass in row-major cell order, the fixed
//! N,S,W,E tie-break of [`crate::flow::follow`]. No `HashMap`, no clock.
//! Budget: the `quant_` tests print [`QuantNav::bytes`] against the i32 field
//! and the bool mask it replaces, and pin it under [`QUANT_BUDGET_BYTES`].

use crate::bake::{Cell, Goal};
use crate::flow;
use engine_core::Fnv64;

/// Cap of the coarse distance channel: a deeper step reads this and sets
/// [`QuantNav::saturated`].
pub const LEVEL_MAX: u8 = 255;
/// Largest runtime the row allows, in bytes (20 MB).
pub const QUANT_BUDGET_BYTES: usize = 20 * 1024 * 1024;

/// Direction nibble for "no step down": the goal, a blocked cell, an
/// unreachable cell, or a cell whose goal was never baked.
const DIR_NONE: u8 = 0;
/// Direction nibble for the neighbour at `y - 1`.
const DIR_NORTH: u8 = 1;
/// Direction nibble for the neighbour at `y + 1`.
const DIR_SOUTH: u8 = 2;
/// Direction nibble for the neighbour at `x - 1`.
const DIR_WEST: u8 = 3;
/// Direction nibble for the neighbour at `x + 1`.
const DIR_EAST: u8 = 4;

/// Quantized nav runtime over one row-major `w` by `h` grid: a 1-bit occupancy
/// mask, a 4-bit downhill direction per cell, and a 1-byte coarse distance.
///
/// Built by [`QuantNav::bake`] from the same `blocked` grid the tiled bake uses
/// and refreshed per goal by [`QuantNav::bake_goal`]. Read-only afterwards, so
/// a runtime is shareable and free of hidden state: the same bake and goal give
/// the same bytes, the same digest and the same step on every call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantNav {
    w: usize,
    h: usize,
    /// One bit per cell, row-major; set means blocked. 1/8 of a bool mask.
    blocked: Vec<u64>,
    /// Two cells per byte, low nibble first: [`DIR_NONE`] or a direction code.
    dirs: Vec<u8>,
    /// One byte per cell: the BFS level, capped at [`LEVEL_MAX`].
    levels: Vec<u8>,
    /// True when the baked goal field runs deeper than [`LEVEL_MAX`] steps.
    saturated: bool,
}

impl QuantNav {
    /// Pack `blocked` (row-major, `w * h` cells) and size an empty field.
    /// A length mismatch reads fully blocked, so `walkable` refuses.
    pub fn bake(blocked: &[bool], w: usize, h: usize) -> Self {
        let cells = w.saturating_mul(h);
        let words = if blocked.len() == cells {
            pack_mask(blocked)
        } else {
            vec![u64::MAX; cells.div_ceil(64)]
        };
        Self {
            w,
            h,
            blocked: words,
            dirs: vec![DIR_NONE; cells.div_ceil(2)],
            levels: vec![LEVEL_MAX; cells],
            saturated: false,
        }
    }

    /// Bake `goal`'s field and quantize it into this runtime. An out-of-bounds
    /// goal leaves the empty field in place, the way an unbaked bake reads.
    pub fn bake_goal(&mut self, goal: &Goal) {
        let cells = self.cells();
        if cells == 0 || goal.x >= self.w || goal.y >= self.h {
            self.clear_field();
            return;
        }
        let blocked = unpack_mask(&self.blocked, cells);
        let field = flow::bake_flow(&blocked, self.w, self.h, (goal.x, goal.y));
        let mut dirs = vec![DIR_NONE; cells.div_ceil(2)];
        let mut levels = vec![LEVEL_MAX; cells];
        let mut saturated = false;
        let cap = i32::from(LEVEL_MAX);
        for y in 0..self.h {
            for x in 0..self.w {
                let idx = y * self.w + x;
                let depth = field[idx];
                if depth < 0 {
                    continue;
                }
                levels[idx] = u8::try_from(depth.min(cap)).unwrap_or(LEVEL_MAX);
                if depth >= cap {
                    saturated = true;
                }
                if depth == 0 {
                    continue;
                }
                if let Some(code) = downhill_code(&field, self.w, self.h, x, y) {
                    set_nibble(&mut dirs, idx, code);
                }
            }
        }
        self.dirs = dirs;
        self.levels = levels;
        self.saturated = saturated;
    }

    /// Grid width in cells.
    pub fn width(&self) -> usize {
        self.w
    }

    /// Grid height in cells.
    pub fn height(&self) -> usize {
        self.h
    }

    /// Cells the runtime spans.
    pub fn cells(&self) -> usize {
        self.w.saturating_mul(self.h)
    }

    /// True when the baked goal field ran past [`LEVEL_MAX`] steps, so the
    /// coarse distance channel is capped (the direction channel stays exact).
    pub fn saturated(&self) -> bool {
        self.saturated
    }

    /// Bytes the runtime occupies: header + mask + nibbles + levels.
    pub fn bytes(&self) -> usize {
        core::mem::size_of::<usize>() * 2
            + core::mem::size_of::<u8>()
            + self.blocked.len() * core::mem::size_of::<u64>()
            + self.dirs.len()
            + self.levels.len()
    }

    /// True when `pos` is on the grid and not blocked; off-grid reads false.
    pub fn walkable(&self, pos: &Cell) -> bool {
        let Some(idx) = self.index(pos) else {
            return false;
        };
        !mask_bit(&self.blocked, idx)
    }

    /// Coarse BFS level of `pos`: 0 at the goal, [`LEVEL_MAX`] for a blocked,
    /// unreachable or off-grid cell, or any cell past the depth cap.
    pub fn level(&self, pos: &Cell) -> u8 {
        match self.index(pos) {
            Some(idx) => self.levels[idx],
            None => LEVEL_MAX,
        }
    }

    /// Next cell downhill toward the baked goal, or `None` at the goal, on a
    /// blocked or unreachable cell, and when no goal was baked. Exact: the same
    /// cell [`crate::flow::follow`] returns for the same grid and goal.
    pub fn direction(&self, pos: &Cell) -> Option<Cell> {
        let idx = self.index(pos)?;
        match nibble(&self.dirs, idx) {
            DIR_NORTH => pos.y.checked_sub(1).map(|y| Cell { x: pos.x, y }),
            DIR_SOUTH => Some(Cell {
                x: pos.x,
                y: pos.y + 1,
            }),
            DIR_WEST => pos.x.checked_sub(1).map(|x| Cell { x, y: pos.y }),
            DIR_EAST => Some(Cell {
                x: pos.x + 1,
                y: pos.y,
            }),
            _ => None,
        }
    }

    /// FNV-1a over the canonical runtime bytes: dims, saturation flag, mask,
    /// nibbles and levels. Same bake and goal give the same digest twice.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write(b"e2navquant1");
        hash.write_u64(u64::try_from(self.w).unwrap_or(u64::MAX));
        hash.write_u64(u64::try_from(self.h).unwrap_or(u64::MAX));
        hash.write_bool(self.saturated);
        for word in &self.blocked {
            hash.write_u64(*word);
        }
        for byte in &self.dirs {
            hash.write_u8(*byte);
        }
        for level in &self.levels {
            hash.write_u8(*level);
        }
        hash.finish()
    }

    /// Row-major cell index, or `None` off the grid or past the mask.
    fn index(&self, pos: &Cell) -> Option<usize> {
        if pos.x >= self.w || pos.y >= self.h {
            return None;
        }
        let idx = pos.y * self.w + pos.x;
        (idx < self.cells()).then_some(idx)
    }

    /// Drop the field, keeping the occupancy mask.
    fn clear_field(&mut self) {
        self.dirs = vec![DIR_NONE; self.cells().div_ceil(2)];
        self.levels = vec![LEVEL_MAX; self.cells()];
        self.saturated = false;
    }
}

/// Pack one bit per cell into little-endian `u64` words.
fn pack_mask(blocked: &[bool]) -> Vec<u64> {
    let mut words = vec![0u64; blocked.len().div_ceil(64)];
    for (idx, cell) in blocked.iter().enumerate() {
        if *cell {
            set_word_bit(&mut words, idx);
        }
    }
    words
}

/// Rebuild the row-major bool grid from the packed mask (bake-time only).
fn unpack_mask(words: &[u64], cells: usize) -> Vec<bool> {
    (0..cells).map(|idx| mask_bit(words, idx)).collect()
}

/// One mask bit; a bit past the mask reads clear.
fn mask_bit(words: &[u64], idx: usize) -> bool {
    let (word, bit) = (idx / 64, idx % 64);
    match words.get(word) {
        Some(value) => value & (1u64 << bit) != 0,
        None => false,
    }
}

/// Set one mask bit; a bit past the mask is dropped.
fn set_word_bit(words: &mut [u64], idx: usize) {
    let (word, bit) = (idx / 64, idx % 64);
    if let Some(value) = words.get_mut(word) {
        *value |= 1u64 << bit;
    }
}

/// One packed nibble; a cell past the nibble array reads [`DIR_NONE`].
fn nibble(bytes: &[u8], idx: usize) -> u8 {
    match bytes.get(idx / 2) {
        Some(byte) => {
            if idx.is_multiple_of(2) {
                byte & 0x0f
            } else {
                byte >> 4
            }
        }
        None => DIR_NONE,
    }
}

/// Write one packed nibble; a cell past the nibble array is dropped.
fn set_nibble(bytes: &mut [u8], idx: usize, value: u8) {
    if let Some(byte) = bytes.get_mut(idx / 2) {
        if idx.is_multiple_of(2) {
            *byte = (*byte & 0xf0) | (value & 0x0f);
        } else {
            *byte = (*byte & 0x0f) | ((value & 0x0f) << 4);
        }
    }
}

/// Direction code of the steepest downhill neighbour of `(x, y)`, in the
/// [`crate::flow::follow`] order (N, S, W, E; the first minimum wins).
fn downhill_code(field: &[i32], w: usize, h: usize, x: usize, y: usize) -> Option<u8> {
    let mut best: Option<(i32, u8)> = None;
    for (nx, ny, code) in [
        (x, y.wrapping_sub(1), DIR_NORTH),
        (x, y.wrapping_add(1), DIR_SOUTH),
        (x.wrapping_sub(1), y, DIR_WEST),
        (x.wrapping_add(1), y, DIR_EAST),
    ] {
        if nx >= w || ny >= h {
            continue;
        }
        let depth = field[ny * w + nx];
        if depth < 0 {
            continue;
        }
        if best.map(|(bd, _)| depth < bd).unwrap_or(true) {
            best = Some((depth, code));
        }
    }
    best.map(|(_, code)| code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{BakedNav, TileEdge};
    use crate::flow::{bake_flow, seeded_blocked};

    /// Grid edge of the measured fields, in cells.
    const W: usize = 48;
    const H: usize = 48;
    /// Seeded arenas: one open field, then four crowd densities up to packed.
    const GRIDS: [(u64, u32); 5] = [(11, 0), (12, 100), (13, 250), (14, 400), (15, 600)];

    /// Bytes the heuristic path costs for one field: an i32 per cell plus the
    /// one-byte bool mask both halves read.
    fn heuristic_bytes(cells: usize) -> usize {
        cells * core::mem::size_of::<i32>() + cells
    }

    fn cell(x: usize, y: usize) -> Cell {
        Cell { x, y }
    }

    #[test]
    fn quant_direction_matches_the_flow_heuristic_cell_for_cell() {
        let goal = Goal { x: W - 1, y: H - 1 };
        let mut total = 0usize;
        for (seed, density) in GRIDS {
            let blocked = seeded_blocked(seed, W, H, density);
            let field = bake_flow(&blocked, W, H, (goal.x, goal.y));
            let mut quant = QuantNav::bake(&blocked, W, H);
            quant.bake_goal(&goal);
            let mut baked = BakedNav::bake(&blocked, W, H, &TileEdge(2));
            baked.bake_goal(&goal);
            let mut agreed = 0usize;
            let mut steps = 0usize;
            for y in 0..H {
                for x in 0..W {
                    let want = flow::follow(&field, W, H, (x, y));
                    let got = quant.direction(&cell(x, y)).map(|c| (c.x, c.y));
                    assert_eq!(got, want, "seed {seed} cell ({x},{y})");
                    let bake_got = baked.direction(&goal, &cell(x, y)).map(|c| (c.x, c.y));
                    assert_eq!(bake_got, want, "seed {seed} bake cell ({x},{y})");
                    agreed += 1;
                    if want.is_some() {
                        steps += 1;
                    }
                }
            }
            total += agreed;
            let cells = W * H;
            println!(
                "quant nav seed {seed} density {density}: {agreed} cells agreed, {steps} steps; \
                 runtime {} bytes vs heuristic {} bytes (i32 field {} + bool mask {})",
                quant.bytes(),
                heuristic_bytes(cells),
                cells * core::mem::size_of::<i32>(),
                cells
            );
            assert!(
                quant.bytes() < heuristic_bytes(cells),
                "seed {seed}: the quantized runtime has to be smaller"
            );
            assert!(quant.bytes() < QUANT_BUDGET_BYTES, "seed {seed}: size");
            // One printed query side by side with the heuristic path.
            let probe = cell(10, 12);
            if let Some(step) = quant.direction(&probe) {
                let heuristic = flow::follow(&field, W, H, (probe.x, probe.y))
                    .expect("a stepped cell has a heuristic step");
                println!(
                    "quant nav seed {seed}: at ({},{}) quant -> ({},{}) = flow -> ({},{})",
                    probe.x, probe.y, step.x, step.y, heuristic.0, heuristic.1
                );
            }
            // A quantized walk from the open corner reaches the goal whenever
            // the heuristic can reach it at all.
            if field[0] >= 0 {
                let mut p = cell(0, 0);
                let mut hops = 0usize;
                while let Some(next) = quant.direction(&p) {
                    p = next;
                    hops += 1;
                    assert!(hops <= cells, "seed {seed}: walk left the grid");
                }
                assert_eq!(
                    (p.x, p.y),
                    (goal.x, goal.y),
                    "seed {seed}: walk reaches the goal"
                );
            }
        }
        assert_eq!(total, GRIDS.len() * W * H);
    }

    #[test]
    fn quant_walkability_follows_the_blocked_grid() {
        let blocked = seeded_blocked(21, W, H, 250);
        let quant = QuantNav::bake(&blocked, W, H);
        let mut blocked_cells = 0usize;
        for y in 0..H {
            for x in 0..W {
                assert_eq!(
                    quant.walkable(&cell(x, y)),
                    !blocked[y * W + x],
                    "({x},{y})"
                );
                blocked_cells += usize::from(blocked[y * W + x]);
            }
        }
        assert!(blocked_cells > 0, "density 250 blocks cells");
        assert!(!quant.walkable(&cell(W, 0)), "off-grid reads unwalkable");
        assert_eq!(quant.level(&cell(W, 0)), LEVEL_MAX, "off-grid level caps");
        assert_eq!(quant.cells(), W * H);
        assert_eq!(quant.width(), W);
        assert_eq!(quant.height(), H);
    }

    #[test]
    fn quant_levels_saturate_past_the_cap_and_direction_stays_exact() {
        // A one-row corridor 600 cells long: 599 steps is past the u8 cap.
        let (w, h) = (600usize, 1usize);
        let blocked = vec![false; w * h];
        let goal = Goal { x: w - 1, y: 0 };
        let field = bake_flow(&blocked, w, h, (goal.x, goal.y));
        assert_eq!(field[0], 599, "the corridor needs 599 steps");
        let mut quant = QuantNav::bake(&blocked, w, h);
        quant.bake_goal(&goal);
        assert!(quant.saturated(), "599 steps is past the {LEVEL_MAX} cap");
        assert_eq!(quant.level(&cell(0, 0)), LEVEL_MAX, "deep end caps");
        assert_eq!(quant.level(&cell(w - 1, 0)), 0, "the goal reads 0");
        assert_eq!(
            quant.level(&cell(300, 0)),
            LEVEL_MAX,
            "a deep cell reads the cap"
        );
        for x in 0..w {
            assert_eq!(
                quant.direction(&cell(x, 0)).map(|c| (c.x, c.y)),
                flow::follow(&field, w, h, (x, 0)),
                "the direction channel stays exact past the cap (x {x})"
            );
        }
        println!(
            "quant nav saturation: 599 steps cap at {LEVEL_MAX}; direction channel still exact"
        );
    }

    #[test]
    fn quant_runtime_repeats_and_refuses_a_bad_mask() {
        let blocked = seeded_blocked(31, 32, 32, 200);
        let goal = Goal { x: 31, y: 31 };
        let mut first = QuantNav::bake(&blocked, 32, 32);
        first.bake_goal(&goal);
        let mut second = QuantNav::bake(&blocked, 32, 32);
        second.bake_goal(&goal);
        assert_eq!(first, second, "same bake twice");
        assert_eq!(first.digest(), second.digest(), "digest x2");
        // Reordering the goal bakes leaves the last one in place, bytes stable.
        let mut third = QuantNav::bake(&blocked, 32, 32);
        third.bake_goal(&Goal { x: 0, y: 0 });
        third.bake_goal(&goal);
        assert_eq!(first, third, "the last goal wins");

        // A mask whose length is not `w * h` refuses every cell.
        let short = vec![false; 10];
        let broken = QuantNav::bake(&short, 32, 32);
        assert!(!broken.walkable(&cell(0, 0)), "a bad mask refuses");
        assert_eq!(broken.direction(&cell(0, 0)), None);
        let mut broken_goal = QuantNav::bake(&short, 32, 32);
        broken_goal.bake_goal(&goal);
        assert_eq!(broken_goal, broken, "no field over a bad mask");

        // An empty grid answers nothing and stays tiny.
        let empty = QuantNav::bake(&[], 0, 0);
        assert_eq!(empty.cells(), 0);
        assert_eq!(empty.direction(&cell(0, 0)), None);
        assert!(empty.bytes() < QUANT_BUDGET_BYTES);
        println!(
            "quant nav runtime: 32x32 field {} bytes, empty runtime {} bytes",
            first.bytes(),
            empty.bytes()
        );
    }
}
