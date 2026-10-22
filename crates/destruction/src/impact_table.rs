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
//!
//! Two callers ride on the table. [`ImpactTable::spawn`] is the raw read:
//! one integer index, one borrowed fragment set. [`ImpactTable::spawn_bonded`]
//! is the runtime path over bonded chunks: it serves the fragment set for
//! the impact, turns it into a bonded descriptor and severs the struck
//! chunk, so a caller gets break events without clipping at the tick.

use crate::bake3d::Chunk3;
use crate::bond_grade::{floor_to_i32, sliver_chunks};
use crate::bonded::BreakEvent;
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

/// What one spawn over bonded chunks produced: the fragment count of the
/// served set, the chunk that took the hit and the break events of the
/// strike, in the bonded layer's own order.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnedBreak {
    /// Bonded chunks in the served fragment set.
    pub chunks: usize,
    /// Chunk that took the hit.
    pub hit_chunk: u32,
    /// Break events of the strike, bond snaps before freed chunks.
    pub events: Vec<BreakEvent>,
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

    /// Runtime spawn over bonded chunks: the fragment set for the impact,
    /// turned into a bonded descriptor, and the break events of the strike
    /// on that descriptor. The struck chunk is severed at `tick`, so the
    /// events name the bond snaps and the freed chunks. This is the call
    /// the tick makes: no clip runs, the table read is the whole cost.
    ///
    /// The events are the bonded chunks' own, so a caller hashes them the
    /// way it hashes any other break.
    pub fn spawn_bonded(
        &self,
        pos_mm: [i32; 3],
        dir_mm: [i32; 3],
        impulse: u32,
        tick: u32,
        breaking_strain: f32,
        break_threshold: f32,
    ) -> Result<SpawnedBreak, ImpactError> {
        let cut = self.spawn(pos_mm, dir_mm, impulse)?;
        let events = cut
            .break_events(tick, breaking_strain, break_threshold)
            .map_err(ImpactError::Fracture)?;
        Ok(SpawnedBreak {
            chunks: cut.chunks.len(),
            hit_chunk: cut.hit_chunk,
            events,
        })
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

    /// Integer volumes of a fragment set, in cubic millimeters, sorted; the
    /// zero-volume pieces are dropped (they carry no mass). Integer only, so
    /// the same set reads the same on every PC.
    fn piece_volumes_mm3(chunks: &[Chunk3]) -> Vec<i64> {
        let mut volumes: Vec<i64> = chunks
            .iter()
            .map(|c| {
                let (hx, hy, hz) = (
                    i64::from(floor_to_i32(f64::from(c.hx) * 1000.0)),
                    i64::from(floor_to_i32(f64::from(c.hy) * 1000.0)),
                    i64::from(floor_to_i32(f64::from(c.hz) * 1000.0)),
                );
                (2 * hx).max(0) * (2 * hy).max(0) * (2 * hz).max(0)
            })
            .filter(|v| *v > 0)
            .collect();
        volumes.sort_unstable();
        volumes
    }

    /// The blind judge's rubric for one fragment set, lower is better.
    ///
    /// The judge never sees which set came from the learned table, so it
    /// must read a property the bake does not optimize. The bake's own
    /// `score` is the sliver count (a needle's aspect ratio) plus the
    /// biggest leftover share; the judge reads the *size distribution*
    /// instead:
    ///
    /// - a speck (under a sixteenth of the mean piece) is a piece the player
    ///   cannot see: 16 points each,
    /// - a slab (over sixteen times the mean piece) is a lump that did not
    ///   break: 16 points each,
    /// - the spread of the pieces around their mean, in per-mille of the
    ///   mean, is the rest: an even break has a low spread.
    ///
    /// Integer, so the same set always scores the same.
    fn judge_penalty(chunks: &[Chunk3]) -> i64 {
        let volumes = piece_volumes_mm3(chunks);
        if volumes.len() < 2 {
            return 0;
        }
        let count = i64::try_from(volumes.len()).unwrap_or(0);
        let total: i64 = volumes.iter().sum();
        let mean = total / count;
        if mean <= 0 {
            return 0;
        }
        let mut penalty = 0i64;
        for v in &volumes {
            if v * 16 < mean || *v > mean * 16 {
                penalty += 16;
            }
        }
        let spread: i64 = volumes.iter().map(|v| (v - mean).abs()).sum();
        penalty + spread * 1000 / (mean * count)
    }

    /// The blind pick between two fragment sets: the judge reads the rubric
    /// of each and names the better one, lower penalty first. It is blind to
    /// which set is the learned one, so the caller shuffles the order and
    /// checks the answer against the truth.
    ///
    /// `None` means the judge cannot tell them apart; the caller then flips
    /// a seeded coin, which is the only fair reading of two sets it rates
    /// equal.
    fn judge_blind(a: &[Chunk3], b: &[Chunk3]) -> Option<bool> {
        let (pa, pb) = (judge_penalty(a), judge_penalty(b));
        if pa != pb {
            return Some(pa < pb);
        }
        None
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

    /// The receipt the row's evidence quotes: entries per shape, the total
    /// table bytes, the mean microseconds one spawn costs and the hash the
    /// 1000-impact replay lands on. The numbers are printed, so a reader
    /// can check them against the run that claims them.
    #[test]
    fn the_table_receipts_its_own_numbers() {
        let mut entries = Vec::new();
        let mut total_bytes = 0usize;
        let mut digests = Vec::new();
        for mesh in &shapes() {
            let table = bake(mesh);
            entries.push(table.len());
            total_bytes += table.bytes().len();
            digests.push(table.digest());
        }
        // One spawn on the siege wall, over 1000 seeded impacts.
        let table = bake(&shapes()[0]);
        let mut rng = Rng::new(4242);
        let impacts: Vec<([i32; 3], [i32; 3], u32)> =
            (0..1000).map(|_| impact(&mut rng, &table)).collect();
        #[expect(
            clippy::disallowed_types,
            clippy::disallowed_methods,
            reason = "test-only timing, never the tick"
        )]
        let start = std::time::Instant::now();
        let mut replay = Fnv64::new();
        for (pos, dir, power) in &impacts {
            let cut = table.spawn(*pos, *dir, *power).expect("spawns");
            replay.write_u64(cut.digest());
        }
        let elapsed = start.elapsed();
        let per_spawn_us = elapsed.as_micros() / 1000;
        let hash = replay.finish();
        println!(
            "impact table: entries {entries:?} ({} per shape), bytes {total_bytes}, \
             spawn {per_spawn_us} us per impact, replay hash {hash:016x}, \
             digests {:016x?}",
            entries[0], digests[0],
        );
        assert_eq!(entries, vec![486, 486, 486]);
        assert!(total_bytes < 50 * 1024 * 1024, "{total_bytes} bytes");
        assert!(
            per_spawn_us < 500,
            "{per_spawn_us} us per spawn is over 0.5 ms"
        );
        // The same impacts replay to the same hash.
        let mut again = Fnv64::new();
        for (pos, dir, power) in &impacts {
            let cut = table.spawn(*pos, *dir, *power).expect("spawns");
            again.write_u64(cut.digest());
        }
        assert_eq!(
            again.finish(),
            hash,
            "the spawn path replays to the same hash"
        );
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

    /// The blind pick: on 20 seeded impact fixtures, the judge sees the
    /// learned set and the plain seeded Voronoi cut in a shuffled order and
    /// names the better one. The judge reads the size rubric, not the bake's
    /// own score, so the pick is evidence and not a restatement.
    ///
    /// A fixture is a seeded impact (position, direction, impulse) drawn
    /// from the table's own box; it lands on a key by the integer lookup.
    /// Both tables hold the same key, so the two sets differ only in which
    /// cut the codebook kept: the learned table keeps the best of the
    /// candidate budget by the bake's own score, the plain table keeps the
    /// plain seeded Voronoi cut (candidate 0) of the same key.
    ///
    /// A judge that cannot tell the two apart names neither: the fixture
    /// counts as a miss, because a coin flip would only measure the seed.
    #[test]
    fn blind_pick_names_the_learned_set_on_most_fixtures() {
        const FIXTURES: usize = 20;
        let mesh = &shapes()[0];
        let learned = ImpactTable::bake(mesh, &spec(), 3, MAX_CANDIDATES).expect("learned bakes");
        let plain = ImpactTable::bake(mesh, &spec(), 3, 1).expect("plain bakes");
        let mut rng = Rng::new(2040).fork_named("blind-pick");
        let mut won = 0usize;
        let mut tied = 0usize;
        let mut same = 0usize;
        for _ in 0..FIXTURES {
            let (pos, dir, power) = impact(&mut rng, &learned);
            let key = learned.index_of(pos, dir, power).expect("inside");
            assert_eq!(
                plain.index_of(pos, dir, power),
                Ok(key),
                "both tables share the key"
            );
            let (lb, pb) = (
                chunk_boxes(&learned.entries[key]),
                chunk_boxes(&plain.entries[key]),
            );
            if lb.len() == pb.len() && judge_penalty(&lb) == judge_penalty(&pb) {
                same += 1;
            }
            // The order is shuffled per fixture, so the judge cannot read
            // the answer off the position.
            let learned_first = rng.next_range(0, 2) == 0;
            let (first, second) = if learned_first {
                (&lb, &pb)
            } else {
                (&pb, &lb)
            };
            let named_learned = match judge_blind(first, second) {
                Some(first_is_better) => first_is_better == learned_first,
                None => {
                    tied += 1;
                    false
                }
            };
            won += usize::from(named_learned);
        }
        println!(
            "blind pick: learned set named on {won} of {FIXTURES} fixtures \
             ({tied} judge ties, {same} identical sets)",
        );
        assert!(
            won >= 12,
            "blind judge named the learned set on {won} of {FIXTURES} fixtures"
        );
    }

    /// The judge's rubric is not the bake's own score: on a witness pair the
    /// two orderings disagree, so a blind pick under the rubric is evidence
    /// and not a restatement of the bake.
    ///
    /// The witness: two even cubes against the same two cubes plus a speck.
    /// The bake's score prefers the specked set (the biggest piece holds a
    /// smaller share of it); the rubric prefers the even set (no speck, no
    /// spread).
    #[test]
    fn the_blind_judge_does_not_restate_the_bake_score() {
        let cube = |h: f32| Chunk3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            hx: h,
            hy: h,
            hz: h,
            mass: h * h * h * 8.0,
        };
        let even = vec![cube(0.5), cube(0.5)];
        let specked = vec![cube(0.5), cube(0.5), cube(0.5), cube(0.05)];
        assert!(
            score(&specked) < score(&even),
            "the bake score prefers the specked set: {} vs {}",
            score(&specked),
            score(&even)
        );
        assert!(
            judge_penalty(&specked) > judge_penalty(&even),
            "the rubric prefers the even set: {} vs {}",
            judge_penalty(&specked),
            judge_penalty(&even)
        );
        assert_eq!(judge_blind(&specked, &even), Some(false));
        assert_eq!(judge_blind(&even, &specked), Some(true));
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

    /// The spawn caller at the bonded level: an impact reads one table
    /// entry and comes back with break events over the bonded chunks, and
    /// the same impact reads the same events twice.
    #[test]
    fn spawn_bonded_serves_break_events_and_replays() {
        use crate::bake::BREAKING_THRESHOLD;
        let table = bake(&shapes()[0]);
        let rng = Rng::new(31);
        let run = || {
            let mut rng = rng.fork(1);
            let mut h = Fnv64::new();
            for _ in 0..50 {
                let (pos, dir, power) = impact(&mut rng, &table);
                let out = table
                    .spawn_bonded(pos, dir, power, 7, 2.0, BREAKING_THRESHOLD)
                    .expect("spawns");
                assert!(out.chunks >= 2, "{} bonded chunks", out.chunks);
                assert!(out.hit_chunk < out.chunks as u32);
                assert!(
                    out.events.iter().all(|e| matches!(
                        e,
                        BreakEvent::BondBroken { tick: 7, .. }
                            | BreakEvent::ChunkFreed { tick: 7, .. }
                    )),
                    "every event carries the strike tick"
                );
                for e in &out.events {
                    h.write(&crate::bonded::event_bytes(e));
                }
            }
            h.finish()
        };
        assert_eq!(run(), run(), "the bonded spawn replays to the same hash");
        // A refused impact never reaches the bonded layer.
        assert!(matches!(
            table.spawn_bonded([999_999, 0, 0], [1, 0, 0], 0, 1, 2.0, BREAKING_THRESHOLD),
            Err(ImpactError::BadImpact)
        ));
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
