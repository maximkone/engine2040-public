//! fracture: runtime Voronoi fracture at an impact point, plus the cook
//! cache that serves the same cut without the cold clip.
//!
//! Consumer: siege walls and big props in the arena match (a hit asks for
//! one [`fracture_at`] or one [`FractureCache::cut`]); the break verbs call
//! the same functions through the door's break pack. Data owner:
//! `engine-destruction`. Units: meters for geometry and sites, cell counts
//! as `u32`. Determinism: the sites are integer `Rng` draws turned into
//! floats with `+ - * /` only, each cell is the prop shell clipped by the
//! bisector half-spaces of its site against every other site (nearest
//! first, ties by site index), so the same shell, hit and spec give the
//! same chunk bytes on every PC. Errors: [`FractureError`] for a bad spec,
//! hit or mesh; a cut that leaves no cell refuses instead of returning the
//! prop. Budget: at most [`MAX_FRACTURE_CELLS`] cells per event, one clip
//! per (cell, neighbor) pair over the shell triangles, so a fixture wall
//! costs far under 1 ms (see the budget test). The cook cache moves the
//! clip out of the tick: [`FractureCache::bake`] runs once offline and a
//! runtime cut is a table read.
//!
//! Voronoi idea: the cell of a site is the intersection of the half-spaces
//! closer to it than to any other site; the bisector planes feed the plane
//! split already in `csg` (shells stay open along cuts).

use crate::bake3d::Chunk3;
use crate::bonded::{BondedCollection, BondedDesc, BreakError, BreakEvent};
use crate::crack::{fold_state, spread_for_desc, CrackError, CrackOutcome};
use crate::csg::{
    clip_back_reused, csg_bake_bytes, dist2, frac_name, mesh_aabb, plane_between, to_bonded_desc,
    ClipScratch, CsgError, FractureChunk, PropMesh,
};
use engine_core::{hash_bytes, Fnv64, Rng};

/// Most cells one fracture event emits.
pub const MAX_FRACTURE_CELLS: u32 = 64;
/// Most cells per grid axis of the cook cache.
pub const MAX_CACHE_GRID: u32 = 8;
/// Largest accepted cluster radius; meters.
pub const MAX_RADIUS_M: f32 = 1000.0;
/// Wire version of the cache bytes.
pub const FRACTURE_CACHE_V1: u32 = 1;
/// Draw span for site offsets: offsets are `[-1, 1)` in 1/1024 steps.
const OFFSET_SPAN: u64 = 2048;
/// Share of the sites that cluster near the hit, in eighths; the rest
/// spread over the whole shell so no cell swallows the far side.
const NEAR_EIGHTHS: u32 = 5;

/// Why a fracture request was refused; user input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractureError {
    /// Cell count below two or past [`MAX_FRACTURE_CELLS`].
    BadCells,
    /// Cluster radius non-finite, non-positive or past [`MAX_RADIUS_M`].
    BadRadius,
    /// The hit point is non-finite or outside the shell box.
    BadHit,
    /// Cache grid zero or past [`MAX_CACHE_GRID`] per axis.
    BadGrid,
    /// Fewer than two cells kept triangles: the cut split nothing.
    NoSplit,
    /// The shell was refused by the CSG kernel.
    Csg(CsgError),
    /// The bonded descriptor was refused.
    Bonded(BreakError),
}

impl core::fmt::Display for FractureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FractureError::BadCells => write!(f, "fracture cell count is out of bounds"),
            FractureError::BadRadius => write!(f, "fracture radius is out of bounds"),
            FractureError::BadHit => write!(f, "fracture hit is outside the shell"),
            FractureError::BadGrid => write!(f, "fracture cache grid is out of bounds"),
            FractureError::NoSplit => write!(f, "fracture split the shell into one piece"),
            FractureError::Csg(e) => write!(f, "fracture shell refused: {e}"),
            FractureError::Bonded(e) => write!(f, "fracture descriptor refused: {e}"),
        }
    }
}

impl std::error::Error for FractureError {}

impl From<CsgError> for FractureError {
    fn from(e: CsgError) -> Self {
        FractureError::Csg(e)
    }
}

impl From<BreakError> for FractureError {
    fn from(e: BreakError) -> Self {
        FractureError::Bonded(e)
    }
}

/// What one fracture event asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FractureSpec {
    /// Sites, so at most this many chunks; `2..=MAX_FRACTURE_CELLS`.
    pub cells: u32,
    /// Radius of the fine cluster around the hit; meters.
    pub radius_m: f32,
    /// Seed of the site draws.
    pub seed: u64,
}

impl FractureSpec {
    /// Range-checks the spec without cutting.
    pub fn validate(&self) -> Result<(), FractureError> {
        if !(2..=MAX_FRACTURE_CELLS).contains(&self.cells) {
            return Err(FractureError::BadCells);
        }
        if !self.radius_m.is_finite() || self.radius_m <= 0.0 || self.radius_m > MAX_RADIUS_M {
            return Err(FractureError::BadRadius);
        }
        Ok(())
    }
}

/// One fracture event: the chunks, the sites they grew from and which
/// chunk took the hit.
#[derive(Clone, Debug, PartialEq)]
pub struct Fracture {
    /// Cell patches in site order; empty cells are dropped.
    pub chunks: Vec<FractureChunk>,
    /// The site of every kept chunk, same order; meters.
    pub sites: Vec<[f32; 3]>,
    /// Index into `chunks` of the cell holding the hit point.
    pub hit_chunk: u32,
}

impl Fracture {
    /// Bonded descriptor for the pieces: contact bonds at `breaking_strain`,
    /// bottom layer anchored (see `csg::to_bonded_desc`).
    pub fn to_desc(
        &self,
        breaking_strain: f32,
        break_threshold: f32,
    ) -> Result<BondedDesc, FractureError> {
        Ok(to_bonded_desc(
            &self.chunks,
            breaking_strain,
            break_threshold,
        )?)
    }

    /// The break events of the hit on the bonded-collection shape: the struck chunk loses
    /// every bond at `tick`, so `BondBroken` events (ascending neighbor)
    /// come first and `ChunkFreed` for whatever no longer touches an
    /// anchor follows, in index order.
    pub fn break_events(
        &self,
        tick: u32,
        breaking_strain: f32,
        break_threshold: f32,
    ) -> Result<Vec<BreakEvent>, FractureError> {
        let desc = self.to_desc(breaking_strain, break_threshold)?;
        let mut field = BondedCollection::new(&desc)?;
        let hit = usize::try_from(self.hit_chunk).unwrap_or(usize::MAX);
        field.sever_chunk(hit, tick)?;
        Ok(field.drain_events())
    }

    /// Chunk bytes: the CSG bake bytes, then the sites bit-exact, then the
    /// hit chunk.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = csg_bake_bytes(&self.chunks);
        out.extend_from_slice(&len64(self.sites.len()).to_le_bytes());
        for site in &self.sites {
            for f in site {
                out.extend_from_slice(&f.to_bits().to_le_bytes());
            }
        }
        out.extend_from_slice(&self.hit_chunk.to_le_bytes());
        out
    }

    /// Digest of [`Self::bytes`]; equal inputs digest equal twice.
    #[must_use]
    pub fn digest(&self) -> u64 {
        hash_bytes(&self.bytes())
    }
}

/// Seeded site list: site 0 is the hit, then `NEAR_EIGHTHS / 8` of the
/// rest within `radius_m` of it (denser near the hit), the remainder
/// spread over the shell box. Every site stays inside the box.
fn scatter_sites(
    min: &[f32; 3],
    max: &[f32; 3],
    hit: [f32; 3],
    spec: &FractureSpec,
    stream: u64,
) -> Vec<[f32; 3]> {
    let mut rng = Rng::new(spec.seed).fork_named("fracture").fork(stream);
    let count = usize::try_from(spec.cells).unwrap_or(usize::MAX);
    let near = (count - 1) * usize::try_from(NEAR_EIGHTHS).unwrap_or(0) / 8;
    let mut sites = Vec::with_capacity(count);
    sites.push(hit);
    for k in 1..count {
        let mut p = [0.0f32; 3];
        for axis in 0..3 {
            let draw = rng.next_range(0, OFFSET_SPAN);
            let unit = small(draw) / 1024.0 - 1.0;
            p[axis] = if k <= near {
                // Squared offset pulls the cluster toward the hit.
                let fall = unit * unit.abs();
                (hit[axis] + fall * spec.radius_m).clamp(min[axis], max[axis])
            } else {
                min[axis] + (unit * 0.5 + 0.5) * (max[axis] - min[axis])
            };
        }
        sites.push(p);
    }
    sites
}

/// Draw value as an exact `f32` (spans are far below `u16::MAX`).
fn small(draw: u64) -> f32 {
    f32::from(u16::try_from(draw).unwrap_or(u16::MAX))
}

/// Work one fracture call did in the Voronoi cell loop. The counts belong to
/// the call that produced them: two fractures running at once, on any
/// threads, each report their own and nothing is shared between them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClipCounts {
    /// Bisector clips run, one per (cell, neighbor) pair reached.
    pub clips: usize,
    /// Neighbors the early exit skipped without a clip.
    pub skipped: usize,
}

/// Working state of one fracture call, owned by the caller of the cell loop
/// and reused across its cells: the neighbor order, the clip scratch and the
/// counts. Every call builds its own, so no fracture reads another's state.
#[derive(Default)]
struct CellWork {
    order: Vec<usize>,
    clip: ClipScratch,
    counts: ClipCounts,
}

/// Absolute slack of the Voronoi early exit; meters.
const EXIT_SLACK_M: f32 = 1e-4;
/// Slack of the Voronoi early exit per meter of coordinate magnitude, which
/// covers the `f32` rounding of a plane distance far from the origin.
const EXIT_SLACK_PER_M: f32 = 1e-5;

/// Squared distance from `site` to the farthest vertex of `mesh`.
fn farthest_dist2(site: &[f32; 3], mesh: &PropMesh) -> f32 {
    mesh.positions
        .iter()
        .fold(0.0f32, |far, v| far.max(dist2(site, v)))
}

/// Squared neighbor distance past which no bisector can touch `cell`. Every
/// cell point lies within the farthest cell vertex of `site` (clips only add
/// points on shell edges), so a bisector farther than `slack` beyond that
/// radius leaves the whole cell on its back side and the clip would be a miss.
fn exit_dist2(site: &[f32; 3], cell: &PropMesh, slack: f32) -> f32 {
    let reach = farthest_dist2(site, cell).sqrt() * 2.0 + slack;
    reach * reach
}

/// Squared distances between all site pairs, row-major `sites * sites`.
fn pair_dist2(sites: &[[f32; 3]]) -> Vec<f32> {
    let mut table = Vec::with_capacity(sites.len() * sites.len());
    for a in sites {
        table.extend(sites.iter().map(|b| dist2(a, b)));
    }
    table
}

/// Shared inputs of one fracture event's cell loop.
struct CellLoop<'a> {
    mesh: &'a PropMesh,
    sites: &'a [[f32; 3]],
    /// Row-major squared site distances (see [`pair_dist2`]).
    pair_d2: &'a [f32],
    slack: f32,
}

impl CellLoop<'_> {
    /// Cell of site `i`: the shell clipped by every other site's bisector,
    /// nearest site first (ties by index). `work` is reused across cells and
    /// collects the counts. Neighbors run nearest first and the cell only
    /// shrinks, so the loop stops at the first neighbor past the cell's
    /// reach; later clips are misses and the bytes match a full clip.
    fn clip_cell(&self, i: usize, work: &mut CellWork) -> PropMesh {
        let (sites, row) = (self.sites, i * self.sites.len());
        let site = &sites[i];
        let order = &mut work.order;
        order.clear();
        order.extend((0..sites.len()).filter(|j| *j != i));
        order.sort_by(|a, b| {
            self.pair_d2[row + *a]
                .total_cmp(&self.pair_d2[row + *b])
                .then(a.cmp(b))
        });
        let mut cell = self.mesh.clone();
        let mut cutoff2 = exit_dist2(site, &cell, self.slack);
        for (rank, &j) in order.iter().enumerate() {
            if self.pair_d2[row + j] > cutoff2 {
                work.counts.skipped += order.len() - rank;
                break;
            }
            work.counts.clips += 1;
            let Some(plane) = plane_between(site, &sites[j]) else {
                // Coincident sites share one cell: the lower index keeps it.
                if j < i {
                    cell.indices.clear();
                    break;
                }
                continue;
            };
            let changed = clip_back_reused(&mut cell, &plane, &mut work.clip);
            if cell.indices.is_empty() {
                break;
            }
            if changed {
                cutoff2 = exit_dist2(site, &cell, self.slack);
            }
        }
        cell
    }
}

/// Voronoi cells of `sites` over the shell. Cell `i` is the shell clipped
/// by the half-space of every other site, nearest site first (ties by
/// index); an empty cell is dropped. Returns kept chunks, their sites and
/// the kept index of the cell closest to `hit`, plus the counts of this call.
fn clip_cells(
    mesh: &PropMesh,
    sites: &[[f32; 3]],
    hit: &[f32; 3],
) -> Result<(Fracture, ClipCounts), FractureError> {
    let (min, max) = mesh_aabb(mesh);
    let reach = min.iter().chain(&max).fold(0.0f32, |m, c| m.max(c.abs()));
    let pair_d2 = pair_dist2(sites);
    let cells = CellLoop {
        mesh,
        sites,
        pair_d2: &pair_d2,
        slack: EXIT_SLACK_M + reach * EXIT_SLACK_PER_M,
    };
    let mut chunks = Vec::with_capacity(sites.len());
    let mut kept_sites = Vec::with_capacity(sites.len());
    let mut hit_chunk = 0u32;
    let mut best = f32::INFINITY;
    let mut work = CellWork {
        order: Vec::with_capacity(sites.len()),
        ..CellWork::default()
    };
    for (i, site) in sites.iter().enumerate() {
        let cell = cells.clip_cell(i, &mut work);
        if cell.indices.is_empty() {
            continue;
        }
        let (min, max) = mesh_aabb(&cell);
        let d = dist2(site, hit);
        if d < best {
            best = d;
            hit_chunk = u32::try_from(chunks.len()).unwrap_or(u32::MAX);
        }
        chunks.push(FractureChunk {
            name: frac_name(chunks.len()),
            mesh: cell,
            min,
            max,
        });
        kept_sites.push(*site);
    }
    if chunks.len() < 2 {
        return Err(FractureError::NoSplit);
    }
    let fracture = Fracture {
        chunks,
        sites: kept_sites,
        hit_chunk,
    };
    Ok((fracture, work.counts))
}

/// Box of the shell, `(min, max)`.
fn shell_box(mesh: &PropMesh) -> Result<([f32; 3], [f32; 3]), FractureError> {
    crate::csg::validate_mesh(mesh)?;
    Ok(mesh_aabb(mesh))
}

/// Cold fracture: sites around `hit`, one clip per (cell, neighbor).
/// `stream` salts the seed so each cache key draws its own pattern; direct
/// callers pass 0.
pub fn fracture_at(
    mesh: &PropMesh,
    hit: [f32; 3],
    spec: &FractureSpec,
) -> Result<Fracture, FractureError> {
    fracture_counted(mesh, hit, spec).map(|(fracture, _)| fracture)
}

/// [`fracture_at`] plus the [`ClipCounts`] of this call: the same bytes, and
/// how much clipping the early exit saved. The counts are returned, not
/// stored, so concurrent calls never mix them.
pub fn fracture_counted(
    mesh: &PropMesh,
    hit: [f32; 3],
    spec: &FractureSpec,
) -> Result<(Fracture, ClipCounts), FractureError> {
    fracture_stream(mesh, hit, spec, 0)
}

fn fracture_stream(
    mesh: &PropMesh,
    hit: [f32; 3],
    spec: &FractureSpec,
    stream: u64,
) -> Result<(Fracture, ClipCounts), FractureError> {
    spec.validate()?;
    let (min, max) = shell_box(mesh)?;
    if hit.iter().any(|f| !f.is_finite()) || (0..3).any(|a| hit[a] < min[a] || hit[a] > max[a]) {
        return Err(FractureError::BadHit);
    }
    let sites = scatter_sites(&min, &max, hit, spec, stream);
    clip_cells(mesh, &sites, &hit)
}

/// Cache key: cell of the quantized hit grid, each axis `0..grid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheKey {
    /// Grid index along x.
    pub ix: u32,
    /// Grid index along y.
    pub iy: u32,
    /// Grid index along z.
    pub iz: u32,
}

/// Pre-cooked fractures for one shell: one entry per grid cell the shell
/// box holds, each the cold fracture at that cell's center. A runtime hit
/// quantizes to its grid cell, so a cache hit and a cold cut at the same
/// cell center agree byte for byte.
#[derive(Clone, Debug, PartialEq)]
pub struct FractureCache {
    grid: u32,
    spec: FractureSpec,
    min: [f32; 3],
    max: [f32; 3],
    entries: Vec<Fracture>,
}

impl FractureCache {
    /// Cooks the cache: `grid^3` cold fractures in z, y, x index order.
    /// Offline; the cost is `grid^3` cold cuts.
    pub fn bake(mesh: &PropMesh, spec: &FractureSpec, grid: u32) -> Result<Self, FractureError> {
        spec.validate()?;
        if grid == 0 || grid > MAX_CACHE_GRID {
            return Err(FractureError::BadGrid);
        }
        let (min, max) = shell_box(mesh)?;
        let mut cache = Self {
            grid,
            spec: *spec,
            min,
            max,
            entries: Vec::new(),
        };
        for iz in 0..grid {
            for iy in 0..grid {
                for ix in 0..grid {
                    let key = CacheKey { ix, iy, iz };
                    let fracture = cache.cold(mesh, key)?;
                    cache.entries.push(fracture);
                }
            }
        }
        Ok(cache)
    }

    /// Grid cells per axis.
    #[must_use]
    pub fn grid(&self) -> u32 {
        self.grid
    }

    /// Cached entries, `grid^3`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the cache holds nothing (never after a successful bake).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Grid key of a hit point; refuses a hit outside the shell box.
    pub fn key_of(&self, hit: [f32; 3]) -> Result<CacheKey, FractureError> {
        if hit.iter().any(|f| !f.is_finite())
            || (0..3).any(|a| hit[a] < self.min[a] || hit[a] > self.max[a])
        {
            return Err(FractureError::BadHit);
        }
        let mut idx = [0u32; 3];
        for axis in 0..3 {
            let span = self.max[axis] - self.min[axis];
            let t = if span > 0.0 {
                (hit[axis] - self.min[axis]) / span
            } else {
                0.0
            };
            // Cell index = grid lines at or below `t`; the max edge lands in
            // the last cell and no float-to-int cast is involved.
            let g = small_grid(self.grid);
            let below = (1..self.grid).filter(|i| t >= small_grid(*i) / g).count();
            idx[axis] = u32::try_from(below).unwrap_or(0);
        }
        Ok(CacheKey {
            ix: idx[0],
            iy: idx[1],
            iz: idx[2],
        })
    }

    /// Center of a grid cell; meters.
    #[must_use]
    pub fn center_of(&self, key: CacheKey) -> [f32; 3] {
        let g = small_grid(self.grid);
        let at = |axis: usize, i: u32| {
            let span = self.max[axis] - self.min[axis];
            self.min[axis] + span * ((small_grid(i) + 0.5) / g)
        };
        [at(0, key.ix), at(1, key.iy), at(2, key.iz)]
    }

    fn slot(&self, key: CacheKey) -> usize {
        let g = usize::try_from(self.grid).unwrap_or(usize::MAX);
        let (x, y, z) = (
            usize::try_from(key.ix).unwrap_or(usize::MAX),
            usize::try_from(key.iy).unwrap_or(usize::MAX),
            usize::try_from(key.iz).unwrap_or(usize::MAX),
        );
        (z * g + y) * g + x
    }

    /// Cold path for one key: fracture the shell at the cell center with
    /// the key's own stream. The cache stores exactly this.
    pub fn cold(&self, mesh: &PropMesh, key: CacheKey) -> Result<Fracture, FractureError> {
        let stream = u64::try_from(self.slot(key)).unwrap_or(u64::MAX);
        fracture_stream(mesh, self.center_of(key), &self.spec, stream).map(|(fracture, _)| fracture)
    }

    /// Runtime cut: the cached fracture of the hit's grid cell. No clip.
    pub fn cut(&self, hit: [f32; 3]) -> Result<&Fracture, FractureError> {
        let key = self.key_of(hit)?;
        self.entries
            .get(self.slot(key))
            .ok_or(FractureError::BadGrid)
    }

    /// Siege-wall strike through the baked crack path: the cached slice at
    /// `hit`, its bonded conversion and the fixed-point spread, folded with
    /// a zero base. Caller: `crate::crack::CrackWall::strike` owns the wall
    /// tick and the integer hash state; this leg keeps the cache behavior
    /// covered beside `cut` so cache and spread agree in one place.
    pub fn crack_strike(
        &self,
        hit: [f32; 3],
        breaking_strain: f32,
        break_threshold: f32,
        seed_strain_q16: i64,
        tick: u32,
    ) -> Result<CrackOutcome, CrackError> {
        let fracture = self.cut(hit).map_err(CrackError::Fracture)?;
        let desc = fracture
            .to_desc(breaking_strain, break_threshold)
            .map_err(CrackError::Fracture)?;
        let fracture_digest = fracture.digest();
        // A fracture emits at most MAX_FRACTURE_CELLS chunks, far below u32::MAX;
        // the saturating fallback below is unreachable on supported targets.
        let chunks = u32::try_from(fracture.chunks.len()).unwrap_or(u32::MAX);
        let hit_chunk = fracture.hit_chunk;
        let (events, spread_digest) = spread_for_desc(&desc, hit, seed_strain_q16, tick)?;
        let digest = fold_state(0, tick, fracture_digest, spread_digest);
        Ok(CrackOutcome {
            events,
            hit_chunk,
            chunks,
            fracture_digest,
            spread_digest,
            digest,
            state: digest,
        })
    }

    /// Cache bytes: version, grid, spec, box, then every entry's bytes
    /// length-prefixed in slot order.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&FRACTURE_CACHE_V1.to_le_bytes());
        out.extend_from_slice(&self.grid.to_le_bytes());
        out.extend_from_slice(&self.spec.cells.to_le_bytes());
        out.extend_from_slice(&self.spec.radius_m.to_bits().to_le_bytes());
        out.extend_from_slice(&self.spec.seed.to_le_bytes());
        for f in self.min.iter().chain(self.max.iter()) {
            out.extend_from_slice(&f.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&len64(self.entries.len()).to_le_bytes());
        for entry in &self.entries {
            let bytes = entry.bytes();
            out.extend_from_slice(&len64(bytes.len()).to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }

    /// Digest of [`Self::bytes`].
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write(&self.bytes());
        h.finish()
    }
}

/// Length as its `u64` wire value.
fn len64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Grid count as an exact `f32` (grids are capped at 8).
fn small_grid(count: u32) -> f32 {
    f32::from(u16::try_from(count).unwrap_or(u16::MAX))
}

/// Box chunk of a fracture piece, for callers that only need boxes.
#[must_use]
pub fn chunk_boxes(fracture: &Fracture) -> Vec<Chunk3> {
    fracture
        .chunks
        .iter()
        .map(FractureChunk::to_chunk3)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;

    /// Siege wall shell: a closed box `[-2, 2] x [0, 3] x [-0.2, 0.2]`, 12
    /// triangles, meters.
    fn wall() -> PropMesh {
        let (x, y, z) = (2.0f32, 3.0f32, 0.2f32);
        PropMesh {
            positions: vec![
                [-x, 0.0, -z],
                [x, 0.0, -z],
                [x, y, -z],
                [-x, y, -z],
                [-x, 0.0, z],
                [x, 0.0, z],
                [x, y, z],
                [-x, y, z],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        }
    }

    fn spec(cells: u32, seed: u64) -> FractureSpec {
        FractureSpec {
            cells,
            radius_m: 0.8,
            seed,
        }
    }

    fn area(mesh: &PropMesh) -> f64 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let p = |i: u32| mesh.positions[usize::try_from(i).expect("index")];
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                let u = [
                    f64::from(b[0] - a[0]),
                    f64::from(b[1] - a[1]),
                    f64::from(b[2] - a[2]),
                ];
                let v = [
                    f64::from(c[0] - a[0]),
                    f64::from(c[1] - a[1]),
                    f64::from(c[2] - a[2]),
                ];
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt()
            })
            .sum()
    }

    fn unit(rng: &mut Rng) -> f32 {
        f32::from(u16::try_from(rng.next_range(0, 1001)).expect("small")) / 1000.0
    }

    /// Reference cells: every neighbor clips in nearest-first order, with no
    /// early exit and no hoisted tables.
    fn full_clip_cells(mesh: &PropMesh, sites: &[[f32; 3]]) -> Vec<PropMesh> {
        let mut scratch = ClipScratch::default();
        let mut cells = Vec::new();
        for (i, site) in sites.iter().enumerate() {
            let mut order: Vec<usize> = (0..sites.len()).filter(|j| *j != i).collect();
            order.sort_by(|a, b| {
                dist2(site, &sites[*a])
                    .total_cmp(&dist2(site, &sites[*b]))
                    .then(a.cmp(b))
            });
            let mut cell = mesh.clone();
            for j in order {
                let Some(plane) = plane_between(site, &sites[j]) else {
                    if j < i {
                        cell.indices.clear();
                        break;
                    }
                    continue;
                };
                clip_back_reused(&mut cell, &plane, &mut scratch);
                if cell.indices.is_empty() {
                    break;
                }
            }
            if !cell.indices.is_empty() {
                cells.push(cell);
            }
        }
        cells
    }

    /// The wall stretched into a 48 m strip: far neighbors are out of reach
    /// of a small cell, so the early exit fires.
    fn strip(offset_x: f32) -> PropMesh {
        let mut mesh = wall();
        for p in &mut mesh.positions {
            p[0] = p[0] * 12.0 + offset_x;
        }
        mesh
    }

    #[test]
    fn early_exit_keeps_every_cell_byte_for_byte() {
        // Origin-near, long, and far-from-origin shells (the last one has a
        // coarse `f32` grid, which the relative slack must cover).
        for mesh in [wall(), strip(0.0), strip(6000.0)] {
            let (min, max) = shell_box(&mesh).expect("valid shell");
            for (cells, seed) in [(8u32, 1u64), (24, 7), (64, 3), (64, 11)] {
                for frac in [0.1f32, 0.5, 0.9] {
                    let hit = [
                        min[0] + (max[0] - min[0]) * frac,
                        min[1] + (max[1] - min[1]) * frac,
                        min[2] + (max[2] - min[2]) * frac,
                    ];
                    let spec = spec(cells, seed);
                    let sites = scatter_sites(&min, &max, hit, &spec, 0);
                    let reference = full_clip_cells(&mesh, &sites);
                    let (got, _) = clip_cells(&mesh, &sites, &hit).expect("fractures");
                    let meshes: Vec<&PropMesh> = got.chunks.iter().map(|c| &c.mesh).collect();
                    assert_eq!(
                        meshes,
                        reference.iter().collect::<Vec<_>>(),
                        "cells {cells} seed {seed} frac {frac}"
                    );
                }
            }
        }
    }

    /// One fracture request on the fixture wall: hit, cells, seed.
    type Job = ([f32; 3], u32, u64);

    const JOBS: [Job; 5] = [
        ([0.3, 1.4, 0.2], 24, 7),
        ([0.5, 1.5, 0.2], 32, 1),
        ([-1.2, 2.0, 0.2], 16, 3),
        ([0.0, 1.5, 0.0], 64, 11),
        ([1.5, 0.5, -0.1], 8, 9),
    ];

    /// What a job observably produced: the chunk bytes and the call's counts.
    fn observe(job: &Job) -> (Vec<u8>, ClipCounts) {
        let (fracture, counts) =
            fracture_counted(&wall(), job.0, &spec(job.1, job.2)).expect("fractures");
        (fracture.bytes(), counts)
    }

    #[test]
    fn fracture_counts_are_stable_and_belong_to_the_call() {
        let first: Vec<_> = JOBS.iter().map(observe).collect();
        let second: Vec<_> = JOBS.iter().map(observe).collect();
        assert_eq!(first, second, "bytes and counts equal twice");
        for (job, (bytes, counts)) in JOBS.iter().zip(&first) {
            let plain = fracture_at(&wall(), job.0, &spec(job.1, job.2)).expect("fractures");
            assert_eq!(&plain.bytes(), bytes, "counting changes no byte");
            let cells = usize::try_from(job.1).expect("small");
            assert!(counts.clips > 0, "{job:?} clipped something");
            assert!(
                counts.clips + counts.skipped <= cells * (cells - 1),
                "{job:?}: every cell has at most cells - 1 neighbors"
            );
        }
        assert!(
            first.iter().any(|(_, counts)| counts.skipped > 0),
            "the early exit saves clips on a 64-cell wall"
        );
    }

    #[test]
    fn fracture_counts_match_serial_when_two_threads_fracture_at_once() {
        let serial: Vec<_> = JOBS.iter().map(observe).collect();
        let rounds = 6;
        let run_rounds = |reverse: bool| {
            let mut seen = Vec::new();
            for _ in 0..rounds {
                for k in 0..JOBS.len() {
                    let at = if reverse { JOBS.len() - 1 - k } else { k };
                    seen.push((at, observe(&JOBS[at])));
                }
            }
            seen
        };
        let (left, right) = std::thread::scope(|scope| {
            let a = scope.spawn(|| run_rounds(false));
            let b = scope.spawn(|| run_rounds(true));
            (
                a.join().expect("left thread"),
                b.join().expect("right thread"),
            )
        });
        assert_eq!(left.len(), rounds * JOBS.len());
        assert_eq!(right.len(), rounds * JOBS.len());
        for (at, got) in left.iter().chain(&right) {
            assert_eq!(got, &serial[*at], "job {at} differs from its serial run");
        }
    }

    #[test]
    fn same_hit_gives_identical_chunk_sets_twice() {
        let hit = [0.3, 1.4, 0.2];
        let a = fracture_at(&wall(), hit, &spec(24, 7)).expect("fractures");
        let b = fracture_at(&wall(), hit, &spec(24, 7)).expect("fractures");
        assert_eq!(a, b);
        assert_eq!(a.bytes(), b.bytes());
        assert_eq!(a.digest(), b.digest());
        assert!(a.chunks.len() >= 2);
        let other = fracture_at(&wall(), [-1.2, 2.0, 0.2], &spec(24, 7)).expect("fractures");
        assert_ne!(a.digest(), other.digest(), "another hit, another cut");
        let reseeded = fracture_at(&wall(), hit, &spec(24, 8)).expect("fractures");
        assert_ne!(a.digest(), reseeded.digest(), "another seed, another cut");
        println!(
            "fracture hit digest={:016x} chunks={}",
            a.digest(),
            a.chunks.len()
        );
    }

    #[test]
    fn the_chunk_cap_holds_and_bad_specs_refuse() {
        for cells in 2..=MAX_FRACTURE_CELLS {
            let f = fracture_at(&wall(), [0.0, 1.5, 0.2], &spec(cells, 3)).expect("fractures");
            assert!(
                f.chunks.len() <= usize::try_from(cells).expect("small"),
                "cells {cells} gave {}",
                f.chunks.len()
            );
            assert_eq!(f.chunks.len(), f.sites.len());
            assert!(usize::try_from(f.hit_chunk).expect("small") < f.chunks.len());
        }
        for bad in [0, 1, MAX_FRACTURE_CELLS + 1] {
            assert_eq!(
                fracture_at(&wall(), [0.0, 1.5, 0.0], &spec(bad, 1)).unwrap_err(),
                FractureError::BadCells
            );
        }
        for radius in [0.0, -1.0, f32::NAN, MAX_RADIUS_M + 1.0] {
            let s = FractureSpec {
                radius_m: radius,
                ..spec(8, 1)
            };
            assert_eq!(
                fracture_at(&wall(), [0.0, 1.5, 0.0], &s).unwrap_err(),
                FractureError::BadRadius
            );
        }
        for hit in [[f32::NAN, 1.0, 0.0], [9.0, 1.0, 0.0], [0.0, -1.0, 0.0]] {
            assert_eq!(
                fracture_at(&wall(), hit, &spec(8, 1)).unwrap_err(),
                FractureError::BadHit
            );
        }
        let empty = PropMesh {
            positions: Vec::new(),
            indices: Vec::new(),
        };
        assert_eq!(
            fracture_at(&empty, [0.0; 3], &spec(8, 1)).unwrap_err(),
            FractureError::Csg(CsgError::EmptyMesh)
        );
    }

    #[test]
    fn pieces_keep_the_whole_surface_and_cluster_at_the_hit() {
        let mesh = wall();
        let whole = area(&mesh);
        for seed in 0..10u64 {
            let f = fracture_at(&mesh, [0.5, 1.5, 0.2], &spec(32, seed)).expect("fractures");
            let sum: f64 = f.chunks.iter().map(|c| area(&c.mesh)).sum();
            let drift = (sum - whole).abs() / whole;
            assert!(drift < 1e-3, "seed {seed}: area drift {drift}");
        }
        // The cell holding the hit is smaller than the largest: fine near, coarse far.
        let f = fracture_at(&mesh, [0.5, 1.5, 0.2], &spec(32, 1)).expect("fractures");
        let areas: Vec<f64> = f.chunks.iter().map(|c| area(&c.mesh)).collect();
        let largest = areas.iter().copied().fold(0.0f64, f64::max);
        let near = areas[usize::try_from(f.hit_chunk).expect("small")];
        assert!(near < largest, "hit cell {near} vs largest {largest}");
    }

    #[test]
    fn break_events_follow_the_dx1_shape_and_replay() {
        let f = fracture_at(&wall(), [0.5, 1.5, 0.2], &spec(16, 2)).expect("fractures");
        let a = f.break_events(5, 2.0, BREAKING_THRESHOLD).expect("events");
        let b = f.break_events(5, 2.0, BREAKING_THRESHOLD).expect("events");
        assert_eq!(a, b);
        assert!(!a.is_empty());
        // Bond breaks come first, every event carries the hit tick.
        let first_freed = a
            .iter()
            .position(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .unwrap_or(a.len());
        assert!(a[..first_freed]
            .iter()
            .all(|e| matches!(e, BreakEvent::BondBroken { tick: 5, .. })));
        assert!(a[first_freed..]
            .iter()
            .all(|e| matches!(e, BreakEvent::ChunkFreed { tick: 5, .. })));
        // The struck chunk is freed: severing it leaves it no anchor path.
        assert!(a.contains(&BreakEvent::ChunkFreed {
            tick: 5,
            chunk: f.hit_chunk
        }));
    }

    #[test]
    fn cache_hit_cut_is_byte_equal_to_the_cold_cut() {
        let mesh = wall();
        let s = spec(24, 11);
        let cache = FractureCache::bake(&mesh, &s, 3).expect("bakes");
        assert_eq!(cache.len(), 27);
        assert_eq!(cache.grid(), 3);
        // Every cell center and a seeded set of hits agree with the cold path.
        let mut rng = Rng::new(99);
        let mut hits = Vec::new();
        for iz in 0..3 {
            for iy in 0..3 {
                for ix in 0..3 {
                    hits.push(cache.center_of(CacheKey { ix, iy, iz }));
                }
            }
        }
        for _ in 0..200 {
            hits.push([
                -2.0 + 4.0 * unit(&mut rng),
                3.0 * unit(&mut rng),
                -0.2 + 0.4 * unit(&mut rng),
            ]);
        }
        for hit in hits {
            let key = cache.key_of(hit).expect("inside");
            let cold = cache.cold(&mesh, key).expect("cold cut");
            let cached = cache.cut(hit).expect("cache hit");
            assert_eq!(cached.bytes(), cold.bytes(), "hit {hit:?}");
        }
    }

    #[test]
    fn cache_bake_bytes_repeat_and_bad_grids_refuse() {
        let mesh = wall();
        let s = spec(16, 4);
        let a = FractureCache::bake(&mesh, &s, 2).expect("bakes");
        let b = FractureCache::bake(&mesh, &s, 2).expect("bakes");
        assert_eq!(a.bytes(), b.bytes());
        assert_eq!(a.digest(), b.digest());
        let c = FractureCache::bake(&mesh, &spec(16, 5), 2).expect("bakes");
        assert_ne!(a.digest(), c.digest());
        for grid in [0, MAX_CACHE_GRID + 1] {
            assert_eq!(
                FractureCache::bake(&mesh, &s, grid).unwrap_err(),
                FractureError::BadGrid
            );
        }
        assert_eq!(a.cut([9.0, 0.0, 0.0]).unwrap_err(), FractureError::BadHit);
        println!("fracture cache grid=2 digest={:016x}", a.digest());
    }

    #[test]
    fn chunk_boxes_follow_the_chunks() {
        let f = fracture_at(&wall(), [0.0, 1.5, 0.2], &spec(8, 6)).expect("fractures");
        assert_eq!(chunk_boxes(&f).len(), f.chunks.len());
    }

    #[test]
    #[ignore = "wall-clock budget: run alone with --ignored on a quiet box"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn fracture_event_budget_one_millisecond() {
        let mesh = wall();
        // The arena admits 16 shards per event, so 16 cells is the budgeted
        // event; larger events are measured and printed, never asserted.
        let mut report = Vec::new();
        for cells in [16u32, 24, 32, 64] {
            let mut samples = Vec::new();
            for k in 0..200u32 {
                let hit = [
                    -1.5 + 3.0 * f32::from(u16::try_from(k % 10).expect("small")) / 10.0,
                    0.3 + 2.4 * f32::from(u16::try_from(k % 7).expect("small")) / 7.0,
                    0.2,
                ];
                let start = std::time::Instant::now();
                let f = fracture_at(&mesh, hit, &spec(cells, u64::from(k))).expect("fractures");
                samples.push(start.elapsed());
                assert!(f.chunks.len() <= usize::try_from(cells).expect("small"));
            }
            samples.sort_unstable();
            let (p50, p99) = (samples[100], samples[197]);
            println!("fracture event cells={cells} p50={p50:?} p99={p99:?}");
            report.push((cells, p99));
        }
        let (_, p99) = report[0];
        assert!(
            p99.as_micros() <= 1000,
            "16-cell fracture p99 {p99:?} over 1 ms"
        );
    }
}
