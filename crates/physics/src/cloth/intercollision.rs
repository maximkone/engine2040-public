//! intercollision: the intercollision particle set of one cloth sheet and its spatial hash.
//!
//! Only the particles the caller names in the set collide with each other, the linked neighbours
//! (structure plus shear) are skipped so the links never fight the push, and the pass alternates
//! its sweep direction so the separation travels both ways along the sheet.

use super::convert::{cell_count, cell_index, index};
use super::{
    dot, sub, Cloth, ClothError, MAX_HASH_AXIS, MAX_HASH_AXIS_F32, MAX_SELF_PASSES, MIN_LEN2,
    SELF_RELAX,
};

impl Cloth {
    /// The intercollision particle set: the particles that collide with each other. Only the
    /// particles named here collide with each other, so a sheet can put a coarse set in the
    /// folded region and leave the rest out. An empty set is no self-collision at all.
    ///
    /// # Errors
    /// [`ClothError::BadIndex`] when a particle is out of range.
    pub fn set_self_collision_set(&mut self, particles: &[usize]) -> Result<(), ClothError> {
        for &p in particles {
            if p >= self.pos.len() {
                return Err(ClothError::BadIndex(p));
            }
        }
        for word in &mut self.self_set {
            *word = 0;
        }
        self.self_list.clear();
        for &p in particles {
            self.self_set[p / 64] |= 1u64 << (p % 64);
            self.self_list.push(p);
        }
        self.self_list.sort_unstable();
        self.self_list.dedup();
        self.wake();
        Ok(())
    }

    /// How many particles are in the intercollision set.
    pub fn self_collision_count(&self) -> usize {
        self.self_list.len()
    }

    /// True when the particle is in the intercollision set; false for an index out of range.
    pub fn is_self_colliding(&self, particle: usize) -> bool {
        let word = particle / 64;
        word < self.self_set.len() && self.self_set[word] & (1u64 << (particle % 64)) != 0
    }

    /// The separation two particles of the intercollision set keep, meters: half the smaller
    /// rest spacing. The rule is the one Unity writes down: the inter-collision distance is the
    /// diameter of a sphere around each particle and it stays smaller than the smallest distance
    /// between two particles, or the sheet jitters.
    fn self_target(&self) -> f32 {
        0.5 * self.spacing[0].min(self.spacing[1])
    }

    /// Separate the particles of the intercollision set so folded cloth never rests
    /// self-penetrated. The pair set skips the linked neighbours (structure plus shear), so the
    /// links never fight the push, and the sweep alternates direction so the separation travels
    /// both ways along the sheet. Index order with float math only, so the same call sequence
    /// replays exactly.
    ///
    /// The hash is rebuilt before every pass: a pass moves the particles, so the next pass must
    /// see where they are now, or the pairs it moved stay half-separated.
    pub(in crate::cloth) fn resolve_self(&mut self) {
        if self.self_list.len() < 2 {
            return;
        }
        let target = self.self_target();
        if target <= 0.0 {
            return;
        }
        for pass in 0..MAX_SELF_PASSES {
            self.build_hash();
            let mut clean = true;
            let n = self.self_list.len();
            if pass % 2 == 0 {
                for i in 0..n {
                    if !self.self_fix_pass(i, target) {
                        clean = false;
                    }
                }
            } else {
                for i in (0..n).rev() {
                    if !self.self_fix_pass(i, target) {
                        clean = false;
                    }
                }
            }
            if clean {
                break;
            }
        }
    }

    /// Rebuild the spatial hash of the intercollision set. The grid origin follows the sheet so
    /// the cells stay small; the axes and the cell edge are fixed at build time, so the pass
    /// never allocates.
    fn build_hash(&mut self) {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for k in 0..self.self_list.len() {
            let p = self.pos[self.self_list[k]];
            lo[0] = lo[0].min(p[0]);
            lo[1] = lo[1].min(p[1]);
            hi[0] = hi[0].max(p[0]);
            hi[1] = hi[1].max(p[1]);
        }
        self.hash_origin = [lo[0], lo[1]];
        let cells = self.hash_dims[0] * self.hash_dims[1];
        for cell in &mut self.hash_count {
            *cell = 0;
        }
        for k in 0..self.self_list.len() {
            let cell = self.cell_of(self.pos[self.self_list[k]]);
            self.hash_count[cell] += 1;
        }
        let mut sum = 0;
        for cell in 0..cells {
            self.hash_start[cell] = sum;
            sum += self.hash_count[cell];
        }
        self.hash_start[cells] = sum;
        for cell in 0..cells {
            self.hash_count[cell] = self.hash_start[cell];
        }
        for k in 0..self.self_list.len() {
            let particle = self.self_list[k];
            let cell = self.cell_of(self.pos[particle]);
            let at = index(self.hash_count[cell]);
            self.hash_items[at] = particle;
            self.hash_count[cell] += 1;
        }
        let _ = hi;
    }

    /// The hash cell of a point: the grid origin follows the sheet, so a point outside the grid
    /// lands in the edge cell and is still found.
    fn cell_of(&self, p: [f32; 3]) -> usize {
        let inv = 1.0 / self.hash_cell;
        let ix = cell_index((p[0] - self.hash_origin[0]) * inv).min(self.hash_dims[0] - 1);
        let iy = cell_index((p[1] - self.hash_origin[1]) * inv).min(self.hash_dims[1] - 1);
        ix + iy * self.hash_dims[0]
    }

    /// Fix every pair the `i`-th particle of the intercollision set shares with a later one.
    /// True when every pair already clears the separation.
    fn self_fix_pass(&mut self, i: usize, target: f32) -> bool {
        let a = self.self_list[i];
        let cell = self.cell_of(self.pos[a]);
        let width = self.hash_dims[0];
        let cx = cell % width;
        let cy = cell / width;
        let mut clean = true;
        // `cy + oy - 1` for `oy` in 0..3 is the same walk as `-1..=1`, without a signed
        // intermediate and without wrapping at the grid edge.
        for oy in 0..3 {
            let Some(ny) = cy.checked_add(oy).and_then(|row| row.checked_sub(1)) else {
                continue;
            };
            if ny >= self.hash_dims[1] {
                continue;
            }
            for ox in 0..3 {
                let Some(nx) = cx.checked_add(ox).and_then(|column| column.checked_sub(1)) else {
                    continue;
                };
                if nx >= width {
                    continue;
                }
                let neighbour = nx + ny * width;
                let start = index(self.hash_start[neighbour]);
                let end = index(self.hash_start[neighbour + 1]);
                for k in start..end {
                    let b = self.hash_items[k];
                    if b <= a || self.linked(a, b) {
                        continue;
                    }
                    if !self.self_fix(a, b, target) {
                        clean = false;
                    }
                }
            }
        }
        clean
    }

    /// True when two particles are neighbours on the sheet: a structural link (right or down) or
    /// one of the two shear diagonals. Those pairs are the ones the links hold, so the
    /// intercollision push never fights them.
    pub(in crate::cloth) fn linked(&self, a: usize, b: usize) -> bool {
        let (ac, ar) = (a % self.cols, a / self.cols);
        let (bc, br) = (b % self.cols, b / self.cols);
        let dc = bc.abs_diff(ac);
        let dr = br.abs_diff(ar);
        dc <= 1 && dr <= 1 && !(dc == 0 && dr == 0)
    }

    /// Push one pair of the intercollision set apart to the separation. A pinned end stays put
    /// and the free end takes the whole share; exactly coincident pairs escape along the
    /// index-sum axis, so the direction is the same on every run.
    fn self_fix(&mut self, a: usize, b: usize, target: f32) -> bool {
        if self.pinned[a] && self.pinned[b] {
            return true;
        }
        let pa = self.pos[a];
        let pb = self.pos[b];
        let between = sub(pb, pa);
        let d2 = dot(between, between);
        if d2 >= target * target {
            return true;
        }
        let wa = if self.pinned[a] {
            0.0
        } else if self.pinned[b] {
            1.0
        } else {
            0.5
        };
        let wb = if self.pinned[b] {
            0.0
        } else if self.pinned[a] {
            1.0
        } else {
            0.5
        };
        if d2 < MIN_LEN2 {
            let axis = (a + b) % 3;
            let mut na = pa;
            let mut nb = pb;
            na[axis] -= target * SELF_RELAX * wa;
            nb[axis] += target * SELF_RELAX * wb;
            self.pos[a] = na;
            self.pos[b] = nb;
            return false;
        }
        let dist = d2.sqrt();
        let k = (target - dist) * SELF_RELAX / dist;
        let mut na = pa;
        let mut nb = pb;
        for c in 0..3 {
            // `between` points from a to b, so a steps back along it and b
            // steps forward: the pair separates.
            na[c] = pa[c] - between[c] * k * wa;
            nb[c] = pb[c] + between[c] * k * wb;
        }
        self.pos[a] = na;
        self.pos[b] = nb;
        false
    }
}

/// The hash shape of a sheet: the cells per axis and the cell edge. The cell edge is at least
/// the intercollision separation, so every pair that can touch shares a cell or a neighbour.
pub(in crate::cloth) fn hash_shape(spacing: [f32; 2], size: [f32; 2]) -> ([usize; 2], f32) {
    let target = 0.5 * spacing[0].min(spacing[1]);
    let longest = size[0].max(size[1]);
    let cell = target.max(longest / MAX_HASH_AXIS_F32).max(1.0e-4);
    let dim = |e: f32| cell_count(e, cell).clamp(1, MAX_HASH_AXIS);
    ([dim(size[0]), dim(size[1])], cell)
}
