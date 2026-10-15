//! neighbors: uniform-grid shortlist for crowd steering (meters, deterministic).
use crate::crowd::{Agent, MAX_COORD_M, ORCA_MAX_NEIGHBORS};
/// Cell edge in meters for the neighbor index.
pub const NEIGHBOR_CELL_M: f32 = 2.0;
pub(crate) struct Shortlist {
    best_d2: [f32; ORCA_MAX_NEIGHBORS],
    best_id: [u32; ORCA_MAX_NEIGHBORS],
    best: [Option<Agent>; ORCA_MAX_NEIGHBORS],
    filled: usize,
    visited: usize,
}
impl Shortlist {
    pub(crate) fn new() -> Self {
        Self {
            best_d2: [f32::INFINITY; ORCA_MAX_NEIGHBORS],
            best_id: [u32::MAX; ORCA_MAX_NEIGHBORS],
            best: [None; ORCA_MAX_NEIGHBORS],
            filled: 0,
            visited: 0,
        }
    }
    pub(crate) fn rank(&mut self, dist2: f32, oid: u32, other: Agent) {
        let last = ORCA_MAX_NEIGHBORS - 1;
        if self.filled == ORCA_MAX_NEIGHBORS
            && !(dist2 < self.best_d2[last]
                || (dist2 == self.best_d2[last] && oid < self.best_id[last]))
        {
            return;
        }
        let mut slot = self.filled.min(last);
        while slot > 0
            && (dist2 < self.best_d2[slot - 1]
                || (dist2 == self.best_d2[slot - 1] && oid < self.best_id[slot - 1]))
        {
            self.best_d2[slot] = self.best_d2[slot - 1];
            self.best_id[slot] = self.best_id[slot - 1];
            self.best[slot] = self.best[slot - 1];
            slot -= 1;
        }
        self.best_d2[slot] = dist2;
        self.best_id[slot] = oid;
        self.best[slot] = Some(other);
        if self.filled < ORCA_MAX_NEIGHBORS {
            self.filled += 1;
        }
    }
    pub(crate) fn finish(mut self) -> [Option<Agent>; ORCA_MAX_NEIGHBORS] {
        self.best[..self.filled].sort_by_key(|o| o.map_or(u32::MAX, |a| a.id.0));
        self.best
    }
}
fn cell_of(pos_m: [f32; 2], cell_m: f32) -> (f32, f32) {
    let x = (pos_m[0] / cell_m).floor() + 0.0;
    let z = (pos_m[1] / cell_m).floor() + 0.0;
    (x, z)
}
/// Sorted uniform-grid index over one batch; build once, read per agent.
pub struct NeighborIndex {
    cell_m: f32,
    keys: Vec<(f32, f32, usize)>,
}
impl NeighborIndex {
    /// Index every lane by cell; bad cell falls back to [`NEIGHBOR_CELL_M`].
    pub fn build(agents: &[Agent], cell_m: f32) -> Self {
        let cell_m = if cell_m.is_finite() && cell_m > 0.0 {
            cell_m
        } else {
            NEIGHBOR_CELL_M
        };
        let mut keys = Vec::with_capacity(agents.len());
        for (lane, agent) in agents.iter().enumerate() {
            let (cx, cz) = cell_of(agent.pos_m, cell_m);
            keys.push((cx, cz, lane));
        }
        keys.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
        });
        Self { cell_m, keys }
    }
    fn rank_cell(&self, agents: &[Agent], agent: &Agent, short: &mut Shortlist, qx: f32, qz: f32) {
        let below =
            |k: &(f32, f32, usize)| k.0.total_cmp(&qx).then_with(|| k.1.total_cmp(&qz)).is_lt();
        let through =
            |k: &(f32, f32, usize)| k.0.total_cmp(&qx).then_with(|| k.1.total_cmp(&qz)).is_le();
        let lo = self.keys.partition_point(below);
        let hi = self.keys.partition_point(through);
        for key in &self.keys[lo..hi] {
            short.visited += 1;
            let other = &agents[key.2];
            if other.id == agent.id {
                continue;
            }
            let dx = agent.pos_m[0] - other.pos_m[0];
            let dz = agent.pos_m[1] - other.pos_m[1];
            short.rank(dx * dx + dz * dz, other.id.0, *other);
        }
    }
    /// Ten nearest lanes in id order, identical to the brute-force scan.
    pub fn shortlist_for(&self, all: &[Agent], me: &Agent) -> [Option<Agent>; ORCA_MAX_NEIGHBORS] {
        let mut short = Shortlist::new();
        let (ccx, ccz) = cell_of(me.pos_m, self.cell_m);
        let cap = MAX_COORD_M / self.cell_m + 2.0;
        let mut rad = 0.0f32;
        loop {
            let mut d = -rad;
            while d <= rad {
                self.rank_cell(all, me, &mut short, ccx + d, ccz - rad);
                if rad > 0.0 {
                    self.rank_cell(all, me, &mut short, ccx + d, ccz + rad);
                }
                d += 1.0;
            }
            let mut e = -rad + 1.0;
            while e < rad {
                self.rank_cell(all, me, &mut short, ccx - rad, ccz + e);
                self.rank_cell(all, me, &mut short, ccx + rad, ccz + e);
                e += 1.0;
            }
            if short.filled == ORCA_MAX_NEIGHBORS {
                let edge = ((me.pos_m[0] - (ccx - rad) * self.cell_m)
                    .min(((ccx + rad + 1.0) * self.cell_m) - me.pos_m[0]))
                .min(
                    (me.pos_m[1] - (ccz - rad) * self.cell_m)
                        .min(((ccz + rad + 1.0) * self.cell_m) - me.pos_m[1]),
                );
                if edge * edge > short.best_d2[ORCA_MAX_NEIGHBORS - 1] {
                    break;
                }
            }
            if short.visited >= all.len() || rad >= cap {
                break;
            }
            rad += 1.0;
        }
        short.finish()
    }
}
