//! impact_table: impact-keyed fracture codebook over one prop shape.
//!
//! Consumer: the bake path writes a table per shape; the spawn path reads
//! it with [`ImpactTable::spawn`] instead of clipping at runtime. Data
//! owner: `engine-destruction`. Units: integer millimeters for positions
//! and directions, integer impulse units for power. Determinism: the key is
//! integer math only (`i64` cell division, dominant-axis direction, power
//! edges); the offline bake draws candidates from named `Rng` forks and
//! keeps the best by a float score compared with `total_cmp`, so the same
//! shell and spec give the same table bytes on every PC. Errors:
//! [`ImpactError`]. Budget: a spawn is one index computation and a table
//! read, no allocation; the bake cost is `entries * candidates` cold cuts.
//!
//! Shape of the idea (a fragment-set codebook keyed by impact, as in
//! DeepFracture 2310.13344v2): the table is the whole "model", chosen
//! offline per key from several seeded Voronoi candidates by a fixed score
//! (no slivers, no big leftover slab). No network, weights or donor bytes
//! exist; the tick only reads the table.

use crate::bake3d::Chunk3;
use crate::bond_grade::{floor_to_i32, sliver_chunks};
use crate::csg::{mesh_aabb, validate_mesh, PropMesh};
use crate::fracture::{
    chunk_boxes, fracture_at, Fracture, FractureError, FractureSpec, MAX_FRACTURE_CELLS,
};
use engine_core::{Fnv64, Rng};

/// Most grid cells per axis; `4^3 * 6 * 3` is 1152 entries.
pub const MAX_TABLE_GRID: u32 = 4;
/// Most seeded candidates compared per entry.
pub const MAX_CANDIDATES: u32 = 8;
/// Impact directions: the dominant axis, positive or negative.
pub const DIRECTIONS: u32 = 6;
/// Power buckets: below the first edge, below the second, above.
pub const POWERS: u32 = 3;
/// Impulse edges between the power buckets, in integer impulse units.
pub const POWER_EDGES: [u32; 2] = [2_000, 8_000];
/// Wire version of the table bytes.
pub const IMPACT_TABLE_V1: u32 = 1;
/// How far the hit point moves along the impact direction, as a share of
/// the base radius in eighths: fragments pile up behind the strike.
const PUSH_EIGHTHS: f32 = 4.0;

/// Why a table bake or spawn was refused; input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImpactError {
    /// Grid zero or past [`MAX_TABLE_GRID`].
    BadGrid,
    /// Candidate count zero or past [`MAX_CANDIDATES`].
    BadCandidates,
    /// The impact is outside the shell box or its direction is zero.
    BadImpact,
    /// The shell or a cut was refused.
    Fracture(FractureError),
}

impl core::fmt::Display for ImpactError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ImpactError::BadGrid => write!(f, "impact table grid is out of bounds"),
            ImpactError::BadCandidates => {
                write!(f, "impact table candidate count is out of bounds")
            }
            ImpactError::BadImpact => write!(f, "impact is outside the shell or has no direction"),
            ImpactError::Fracture(e) => write!(f, "impact table cut refused: {e}"),
        }
    }
}

impl std::error::Error for ImpactError {}

impl From<FractureError> for ImpactError {
    fn from(e: FractureError) -> Self {
        ImpactError::Fracture(e)
    }
}

/// Impact direction bucket: dominant axis of `dir`, `0..6` as +x, -x, +y,
/// -y, +z, -z. Ties go to the lowest axis; a zero vector has no bucket.
#[must_use]
pub fn direction_bucket(dir_mm: [i32; 3]) -> Option<u32> {
    let mag = dir_mm.map(|v| i64::from(v).abs());
    if mag == [0, 0, 0] {
        return None;
    }
    let axis = if mag[0] >= mag[1] && mag[0] >= mag[2] {
        0
    } else if mag[1] >= mag[2] {
        1
    } else {
        2
    };
    Some(axis * 2 + u32::from(dir_mm[usize::try_from(axis).unwrap_or(0)] < 0))
}

/// Power bucket of an impulse: `0..POWERS`.
#[must_use]
pub fn power_bucket(impulse: u32) -> u32 {
    u32::from(impulse >= POWER_EDGES[0]) + u32::from(impulse >= POWER_EDGES[1])
}

/// Float score of a cut, lower is better: slivers dominate, then the share
/// of the biggest leftover chunk (a good break leaves no slab).
fn score(chunks: &[Chunk3]) -> f32 {
    let volume = |c: &Chunk3| c.hx * c.hy * c.hz * 8.0;
    let total: f32 = chunks.iter().map(volume).sum();
    let biggest = chunks.iter().map(volume).fold(0.0f32, f32::max);
    let slivers = sliver_chunks(chunks).len();
    f32::from(u16::try_from(slivers).unwrap_or(u16::MAX)) + biggest / total.max(1e-9)
}

/// The baked codebook of one shape: a fragment set per (cell, direction,
/// power) key, `grid^3 * 6 * 3` entries in cell, direction, power order.
#[derive(Clone, Debug, PartialEq)]
pub struct ImpactTable {
    grid: u32,
    min_mm: [i32; 3],
    max_mm: [i32; 3],
    entries: Vec<Fracture>,
}

impl ImpactTable {
    /// Cooks the table offline: for every key, `candidates` seeded cuts at
    /// the cell center pushed along the key's direction, keeping the lowest
    /// score (candidate 0 is the plain seeded Voronoi cut, kept on ties).
    pub fn bake(
        mesh: &PropMesh,
        spec: &FractureSpec,
        grid: u32,
        candidates: u32,
    ) -> Result<Self, ImpactError> {
        spec.validate()?;
        if grid == 0 || grid > MAX_TABLE_GRID {
            return Err(ImpactError::BadGrid);
        }
        if candidates == 0 || candidates > MAX_CANDIDATES {
            return Err(ImpactError::BadCandidates);
        }
        validate_mesh(mesh).map_err(FractureError::Csg)?;
        let (min, max) = mesh_aabb(mesh);
        // Ceil is the negated floor of the negation, so one helper serves both.
        let to_mm = |v: f32, up: bool| {
            let mm = f64::from(v) * 1000.0;
            if up {
                -floor_to_i32(-mm)
            } else {
                floor_to_i32(mm)
            }
        };
        let mut table = Self {
            grid,
            min_mm: min.map(|v| to_mm(v, false)),
            max_mm: max.map(|v| to_mm(v, true)),
            entries: Vec::new(),
        };
        let g = f32::from(u16::try_from(grid).unwrap_or(1));
        let root = Rng::new(spec.seed).fork_named("impact-table");
        for iz in 0..grid {
            for iy in 0..grid {
                for ix in 0..grid {
                    let idx = [ix, iy, iz];
                    let center: [f32; 3] = core::array::from_fn(|a| {
                        let t = (f32::from(u16::try_from(idx[a]).unwrap_or(0)) + 0.5) / g;
                        min[a] + (max[a] - min[a]) * t
                    });
                    for dir in 0..DIRECTIONS {
                        for power in 0..POWERS {
                            let slot = u64::try_from(table.entries.len()).unwrap_or(u64::MAX);
                            let cut = best_cut(
                                mesh,
                                spec,
                                &root,
                                slot,
                                center,
                                (min, max),
                                dir,
                                power,
                                candidates,
                            )?;
                            table.entries.push(cut);
                        }
                    }
                }
            }
        }
        Ok(table)
    }

    /// Entry count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the table holds nothing (never after a bake).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Integer key of an impact: position in mm inside the shell box, a
    /// direction in mm (any length) and an impulse in impulse units. The
    /// cell is `(pos - min) * grid / span` in `i64`, so no float is read.
    pub fn index_of(
        &self,
        pos_mm: [i32; 3],
        dir_mm: [i32; 3],
        impulse: u32,
    ) -> Result<usize, ImpactError> {
        let dir = direction_bucket(dir_mm).ok_or(ImpactError::BadImpact)?;
        let mut cell = [0i64; 3];
        for a in 0..3 {
            if pos_mm[a] < self.min_mm[a] || pos_mm[a] > self.max_mm[a] {
                return Err(ImpactError::BadImpact);
            }
            let span = (i64::from(self.max_mm[a]) - i64::from(self.min_mm[a])).max(1);
            let rel = i64::from(pos_mm[a]) - i64::from(self.min_mm[a]);
            cell[a] = (rel * i64::from(self.grid) / span).min(i64::from(self.grid) - 1);
        }
        let g = i64::from(self.grid);
        let cell_slot = (cell[2] * g + cell[1]) * g + cell[0];
        let slot = (cell_slot * i64::from(DIRECTIONS) + i64::from(dir)) * i64::from(POWERS)
            + i64::from(power_bucket(impulse));
        usize::try_from(slot).map_err(|_| ImpactError::BadImpact)
    }

    /// Spawn path: the fragment set for an impact, one table read.
    pub fn spawn(
        &self,
        pos_mm: [i32; 3],
        dir_mm: [i32; 3],
        impulse: u32,
    ) -> Result<&Fracture, ImpactError> {
        let slot = self.index_of(pos_mm, dir_mm, impulse)?;
        self.entries.get(slot).ok_or(ImpactError::BadImpact)
    }

    /// Table bytes: version, grid, box, then every entry's bytes
    /// length-prefixed in slot order.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&IMPACT_TABLE_V1.to_le_bytes());
        out.extend_from_slice(&self.grid.to_le_bytes());
        for v in self.min_mm.iter().chain(self.max_mm.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(
            &u64::try_from(self.entries.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for entry in &self.entries {
            let bytes = entry.bytes();
            out.extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// Digest of [`Self::bytes`]; equal bakes digest equal twice.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write(&self.bytes());
        h.finish()
    }
}

/// Best of `candidates` seeded cuts for one key. The hit point is the cell
/// center pushed along the direction and kept inside the box; the cell
/// count follows the power bucket (soft hits crack coarse, hard hits fine).
#[expect(
    clippy::too_many_arguments,
    reason = "one bake step, every input is a key part"
)]
fn best_cut(
    mesh: &PropMesh,
    spec: &FractureSpec,
    root: &Rng,
    slot: u64,
    center: [f32; 3],
    bounds: ([f32; 3], [f32; 3]),
    dir: u32,
    power: u32,
    candidates: u32,
) -> Result<Fracture, ImpactError> {
    let axis = usize::try_from(dir / 2).unwrap_or(0);
    let sign = if dir.is_multiple_of(2) { 1.0 } else { -1.0 };
    let mut hit = center;
    hit[axis] = (hit[axis] + sign * spec.radius_m * PUSH_EIGHTHS / 8.0)
        .clamp(bounds.0[axis], bounds.1[axis]);
    let cells = match power {
        0 => spec.cells / 2,
        1 => spec.cells,
        _ => spec.cells * 3 / 2,
    }
    .clamp(2, MAX_FRACTURE_CELLS);
    let mut best: Option<(f32, Fracture)> = None;
    for c in 0..candidates {
        let seed = root
            .fork(slot * u64::from(MAX_CANDIDATES) + u64::from(c))
            .next_u64();
        let cut = fracture_at(
            mesh,
            hit,
            &FractureSpec {
                cells,
                seed,
                ..*spec
            },
        )?;
        let s = score(&chunk_boxes(&cut));
        if best.as_ref().is_none_or(|(b, _)| s.total_cmp(b).is_lt()) {
            best = Some((s, cut));
        }
    }
    best.map(|(_, cut)| cut).ok_or(ImpactError::BadCandidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed box shell centered on x/z, standing on y = 0; 12 triangles.
    fn shape(hx: f32, h: f32, hz: f32) -> PropMesh {
        PropMesh {
            positions: vec![
                [-hx, 0.0, -hz],
                [hx, 0.0, -hz],
                [hx, h, -hz],
                [-hx, h, -hz],
                [-hx, 0.0, hz],
                [hx, 0.0, hz],
                [hx, h, hz],
                [-hx, h, hz],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        }
    }

    /// The three shapes of the row: a siege wall, a pillar and a slab.
    fn shapes() -> [PropMesh; 3] {
        [
            shape(2.0, 3.0, 0.2),
            shape(0.4, 3.0, 0.4),
            shape(1.5, 0.3, 1.5),
        ]
    }

    fn spec() -> FractureSpec {
        FractureSpec {
            cells: 12,
            radius_m: 0.8,
            seed: 5,
        }
    }

    fn bake(mesh: &PropMesh) -> ImpactTable {
        ImpactTable::bake(mesh, &spec(), 3, 3).expect("table bakes")
    }

    /// A seeded impact inside the table's box.
    fn impact(rng: &mut Rng, t: &ImpactTable) -> ([i32; 3], [i32; 3], u32) {
        let mut pos = [0i32; 3];
        let mut dir = [0i32; 3];
        for a in 0..3 {
            let span = u64::try_from(t.max_mm[a] - t.min_mm[a] + 1).expect("positive");
            pos[a] = t.min_mm[a] + i32::try_from(rng.next_range(0, span)).expect("fits");
            dir[a] = i32::try_from(rng.next_range(0, 2001)).expect("fits") - 1000;
        }
        if dir == [0, 0, 0] {
            dir[0] = 1;
        }
        (
            pos,
            dir,
            u32::try_from(rng.next_range(0, 12_000)).expect("fits"),
        )
    }

    #[test]
    fn three_shapes_bake_big_small_and_twice_the_same() {
        let mut total = 0usize;
        for mesh in &shapes() {
            let table = bake(mesh);
            assert!(table.len() >= 200, "{} entries", table.len());
            assert_eq!(table.len(), 3 * 3 * 3 * 6 * 3);
            assert_eq!(table.digest(), bake(mesh).digest());
            total += table.bytes().len();
        }
        assert!(total < 50 * 1024 * 1024, "{total} bytes");
    }

    #[test]
    fn keys_are_integer_buckets() {
        assert_eq!(direction_bucket([5, 1, -1]), Some(0));
        assert_eq!(direction_bucket([-5, 1, 1]), Some(1));
        assert_eq!(direction_bucket([0, -2, 1]), Some(3));
        assert_eq!(direction_bucket([0, 1, 4]), Some(4));
        assert_eq!(
            direction_bucket([3, 3, 3]),
            Some(0),
            "ties go to the lowest axis"
        );
        assert_eq!(direction_bucket([0, 0, 0]), None);
        assert_eq!(
            [0, 1_999, 2_000, 7_999, 8_000].map(power_bucket),
            [0, 0, 1, 1, 2]
        );
        let table = bake(&shapes()[0]);
        let (lo, hi) = (table.min_mm, table.max_mm);
        assert_eq!(table.index_of(lo, [1, 0, 0], 0), Ok(0));
        assert_eq!(table.index_of(hi, [0, 0, -1], 99_999), Ok(table.len() - 1));
        assert_eq!(
            table.index_of([hi[0] + 1, lo[1], lo[2]], [1, 0, 0], 0),
            Err(ImpactError::BadImpact)
        );
        assert_eq!(
            table.index_of(lo, [0, 0, 0], 0),
            Err(ImpactError::BadImpact)
        );
    }

    #[test]
    fn a_thousand_random_impacts_spawn_finite_and_replay() {
        for mesh in &shapes() {
            let table = bake(mesh);
            let run = || {
                let mut rng = Rng::new(99);
                let mut h = Fnv64::new();
                for _ in 0..1000 {
                    let (pos, dir, power) = impact(&mut rng, &table);
                    let cut = table.spawn(pos, dir, power).expect("spawns");
                    assert!(cut.sites.iter().flatten().all(|v| v.is_finite()));
                    for c in chunk_boxes(cut) {
                        assert!([c.x, c.y, c.z, c.hx, c.hy, c.hz, c.mass]
                            .iter()
                            .all(|v| v.is_finite()));
                    }
                    h.write_u64(cut.digest());
                }
                h.finish()
            };
            assert_eq!(run(), run());
        }
    }

    #[test]
    fn learned_sets_beat_the_plain_seeded_cut_on_most_keys() {
        let mesh = &shapes()[0];
        let learned = bake(mesh);
        let plain = ImpactTable::bake(mesh, &spec(), 3, 1).expect("plain bakes");
        let mut better = 0;
        for (l, p) in learned
            .entries
            .iter()
            .zip(&plain.entries)
            .step_by(24)
            .take(20)
        {
            let (sl, sp) = (score(&chunk_boxes(l)), score(&chunk_boxes(p)));
            assert!(sl <= sp, "best-of-3 never loses to its own candidate 0");
            better += usize::from(sl < sp);
        }
        assert!(
            better >= 12,
            "learned set scored better on {better} of 20 keys"
        );
    }

    #[test]
    fn spawn_is_a_table_read_under_half_a_millisecond() {
        let table = bake(&shapes()[0]);
        let mut rng = Rng::new(7);
        #[expect(
            clippy::disallowed_types,
            clippy::disallowed_methods,
            reason = "test-only timing, never the tick"
        )]
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let (pos, dir, power) = impact(&mut rng, &table);
            assert!(table.spawn(pos, dir, power).is_ok());
        }
        assert!(
            start.elapsed().as_micros() < 500 * 1000,
            "1000 spawns over 0.5 ms each"
        );
    }

    #[test]
    fn bad_bakes_refuse() {
        let mesh = &shapes()[0];
        assert_eq!(
            ImpactTable::bake(mesh, &spec(), 0, 3).unwrap_err(),
            ImpactError::BadGrid
        );
        assert_eq!(
            ImpactTable::bake(mesh, &spec(), MAX_TABLE_GRID + 1, 3).unwrap_err(),
            ImpactError::BadGrid
        );
        assert_eq!(
            ImpactTable::bake(mesh, &spec(), 2, 0).unwrap_err(),
            ImpactError::BadCandidates
        );
        assert_eq!(
            ImpactTable::bake(mesh, &spec(), 2, MAX_CANDIDATES + 1).unwrap_err(),
            ImpactError::BadCandidates
        );
    }
}
