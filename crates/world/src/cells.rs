//! cells: async cell load/unload through stream budgets, per-cell nav+physics, seamless edges.
//! Owner: `engine-world`; production caller: the map assembler, which drives
//! `Cells::stream`/`publish` through the `engine-assets` `StreamQueue` adapter
//! (request/publish/evict parity asserted in-test) and gates publish via the
//! dual ahead-agent publish gate (seam scan plus walk).
//! Units: bytes for budgets/costs, frames for load time, meters for seams.
//! Determinism: integer math only, `BTreeMap` order, priority-then-coord publish;
//! the sim hash covers live cell bytes only, so presentation never enters it.
//! Errors: typed `CellError`; corrupt, seam-broken and island cells refuse with the check named.
//! Budget: each `publish` moves at most `frame_bytes`; memory never tops `bytes`;
//! per-cell bake cost and peak bytes are posted per cell by the test report.
//!
//! Dependency note for the orchestrator (true on disk today: unwired):
//! this module needs only std, `engine-core` and the crate lattice
//! (`engine-world` has no `engine-assets`/`engine-nav` manifest edge) and
//! hand-mirrors the asset stream (`StreamBudget`/`StreamQueue`) and nav
//! (`BakeConfig`/bake) contracts without importing them. The real edges to
//! add are `engine-world -> engine-assets` (stream queue) and
//! `engine-world -> engine-nav` (per-cell mesh bake); only the integration
//! test wires the real sources, via `#[path]` shims to
//! `crates/assets/src/stream.rs` and `crates/nav/src/mesh.rs`.

use crate::lattice;
use crate::terrain::{base_height, slope_of};
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{hash_bytes, Fnv64};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Magic at the head of every cell blob; own layout, never a `W204` version bump.
pub const CELL_MAGIC: [u8; 4] = *b"WCD1";
/// Cell blob version; bump on any layout change, refuse anything else.
pub const CELL_VERSION: u8 = 1;
/// Smallest cell side in texels.
pub const CELL_MIN_SIZE: u16 = 4;
/// Largest cell side in texels.
pub const CELL_MAX_SIZE: u16 = 64;
/// Halo texels baked around a cell so erosion matches a whole-map bake at the seam.
pub const CELL_HALO: i32 = 8;
/// Largest height step allowed across a shared edge, in height units.
/// Measured 64 over 168 edges (sizes 16/32/64, coords -4..4, seed 12345);
/// 72 keeps 8 of margin. Steps past this are cliffs or corruption, never
/// shipped ground: the halo makes cracks impossible by construction, so this
/// bound documents max grade while the nest test proves exactness.
pub const SEAM_TOL: u8 = 72;
/// Steepest walkable slope in height units per texel (parity with terrain splat).
pub const SLOPE_LIMIT: u8 = 64;
/// Largest absolute cell coordinate; beyond it refuses.
pub const MAX_COORD: i32 = 512;
/// Header bytes: magic(4) + version(1) + seed(8) + cx(4) + cz(4) + size(2).
pub const CELL_HEADER_LEN: usize = 23;
/// Largest single-cell byte cost admitted; mirrors the asset stream payload
/// cap. An honest bake is far below it; it stops a corrupt size from
/// reserving unbounded memory.
pub const CELL_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Hard ceiling on queued cells, independent of the caller budget.
pub const CELLS_MAX_PENDING: usize = 1024;
/// Hard ceiling on admitted cells (pending plus live), independent of the budget.
pub const CELLS_MAX_ADMITTED: usize = 4096;
/// Hard ceiling on texel visits in one `route` call; past it answers
/// `RouteBudget`. A 3 x 3 block of the largest cells is 36,864 texels.
pub const ROUTE_MAX_VISITS: usize = 65_536;
/// Hard ceiling on events one `publish` call emits; remainder waits for next call.
pub const PUBLISH_MAX_EVENTS: usize = 64;
/// Talus for the halo erosion (parity with the terrain bake).
const TALUS: i16 = 10;
/// Hole density cut: top-byte hash above this is a raw hole (parity with terrain).
const HOLE_CUT: u8 = 250;
/// Domain tag mixed into the hole hash so holes never alias heights.
const HOLE_TAG: u64 = GOLDEN_GAMMA ^ 0x51ab;

/// Grid cell address in cell units; ordering is x-major then z for stable sorts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellCoord {
    /// Cell column, within `-MAX_COORD..=MAX_COORD`.
    pub x: i32,
    /// Cell row, within `-MAX_COORD..=MAX_COORD`.
    pub z: i32,
}

impl CellCoord {
    /// Validated coordinate; out-of-range values are typed errors.
    pub fn new(x: i32, z: i32) -> Result<Self, CellError> {
        if !(-MAX_COORD..=MAX_COORD).contains(&x) || !(-MAX_COORD..=MAX_COORD).contains(&z) {
            return Err(CellError::BadCoord);
        }
        Ok(Self { x, z })
    }
}

impl fmt::Display for CellCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cell({}, {})", self.x, self.z)
    }
}

/// Memory plus slot plus per-frame budget for one cell registry. Units: bytes for
/// `bytes`, admitted cells (pending plus live) for `jobs`, bytes per `publish`
/// call for `frame_bytes`. Maps 1:1 onto the stream `StreamBudget` plus the frame
/// cap the stream `publish` takes per call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellBudget {
    /// Cap on reserved plus live cell bytes.
    pub bytes: u64,
    /// Cap on admitted cells (pending plus live).
    pub jobs: u32,
    /// Bytes one `publish` call may move; the tick never waits past this.
    pub frame_bytes: u64,
}

impl CellBudget {
    /// Validated budget; zero caps are typed errors.
    pub fn new(bytes: u64, jobs: u32, frame_bytes: u64) -> Result<Self, CellError> {
        if bytes == 0 || jobs == 0 || frame_bytes == 0 {
            return Err(CellError::BadBudget);
        }
        Ok(Self {
            bytes,
            jobs,
            frame_bytes,
        })
    }
}

/// Why a cell stream, publish, eviction, repair or query failed. Corrupt blobs
/// and broken seams name their check; cross-cell queries on unloaded cells
/// error instead of guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellError {
    /// Cell side outside `CELL_MIN_SIZE..=CELL_MAX_SIZE`.
    BadSize {
        /// Side offered in texels.
        got: u16,
    },
    /// Coordinate past `MAX_COORD`.
    BadCoord,
    /// Zero byte, job or frame cap in the budget.
    BadBudget,
    /// Coord already pending or live; the first slot is untouched.
    Duplicate,
    /// Single cell bytes outside `1..=64 MiB`.
    TooLarge {
        /// Byte count offered.
        count: usize,
    },
    /// Admitting the cell would exceed the byte budget.
    OverBudget {
        /// Cell bytes wanted.
        want_bytes: u64,
        /// Bytes free at request time.
        free_bytes: u64,
    },
    /// Admitting the cell would exceed the job-slot budget.
    JobCap {
        /// Slot budget that is full.
        limit: u32,
    },
    /// Coord is neither pending nor live.
    Unknown,
    /// Pending coord needs `cancel`, not `evict`.
    PendingNeedsCancel,
    /// Live coord needs `evict`, not `cancel`.
    LiveNeedsEvict,
    /// Blob failed its bake check; the static names the check.
    Corrupt(&'static str),
    /// Adjacent cells fail the seam scan; the static names the check.
    SeamMismatch(&'static str),
    /// Query touches a cell that is not live; the walker waits, never guesses.
    NotLive,
    /// Walkable endpoints share no path through live cells.
    Unreachable,
    /// The route search hit its visit ceiling before it found the goal or
    /// proved there is none; the answer is unknown, not "no path".
    RouteBudget {
        /// Visit ceiling that stopped the search.
        limit: u32,
    },
    /// Texel index past the cell side.
    BadIndex,
    /// Coord already used its one repair.
    AlreadyRepaired,
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize { got } => write!(f, "cell bad size {got}"),
            Self::BadCoord => write!(f, "cell bad coord"),
            Self::BadBudget => write!(f, "cell bad budget"),
            Self::Duplicate => write!(f, "cell duplicate"),
            Self::TooLarge { count } => write!(f, "cell too large: {count}"),
            Self::OverBudget {
                want_bytes,
                free_bytes,
            } => write!(
                f,
                "cell over budget: want {want_bytes} bytes, free {free_bytes}"
            ),
            Self::JobCap { limit } => write!(f, "cell job cap reached: {limit}"),
            Self::Unknown => write!(f, "cell unknown coord"),
            Self::PendingNeedsCancel => write!(f, "cell pending: cancel, not evict"),
            Self::LiveNeedsEvict => write!(f, "cell live: evict, not cancel"),
            Self::Corrupt(check) => write!(f, "cell corrupt: {check}"),
            Self::SeamMismatch(check) => write!(f, "cell seam broken: {check}"),
            Self::NotLive => write!(f, "cell not live"),
            Self::Unreachable => write!(f, "cell unreachable"),
            Self::RouteBudget { limit } => write!(f, "cell route over {limit} visits"),
            Self::BadIndex => write!(f, "cell bad texel index"),
            Self::AlreadyRepaired => write!(f, "cell already repaired once"),
        }
    }
}

impl std::error::Error for CellError {}

/// Progress events for the map loader, in emission order. `publish` emits at most
/// three events per touched cell per call (`Started` once, then `Progress`, then
/// `Completed`) and at most `PUBLISH_MAX_EVENTS` events per call; untouched cells
/// emit nothing, so the tick never waits past the frame cap. Mirrors the stream
/// `StreamEvent` order 1:1 at the IO boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellEvent {
    /// Cell admitted and queued; its bytes are now reserved.
    Queued {
        /// Admitted coord.
        coord: CellCoord,
    },
    /// Cell advanced for the first time.
    Started {
        /// Cell coord.
        coord: CellCoord,
    },
    /// Cell advanced this frame; units: bytes.
    Progress {
        /// Cell coord.
        coord: CellCoord,
        /// Bytes streamed so far.
        done_bytes: u64,
        /// Total cell bytes.
        total_bytes: u64,
    },
    /// Cell fully streamed, baked, bound and live.
    Completed {
        /// Cell coord.
        coord: CellCoord,
    },
    /// Live cell evicted by the consumer (distance/use); bytes freed exactly.
    Evicted {
        /// Evicted coord.
        coord: CellCoord,
        /// Bytes returned to the budget; always equals the cell cost.
        freed_bytes: u64,
    },
    /// Pending cell cancelled before completion; its reservation is freed.
    Cancelled {
        /// Cancelled coord.
        coord: CellCoord,
    },
    /// Explicit refusal: nothing staged. Corrupt and over-budget cells report here.
    Refused {
        /// Attempted coord.
        coord: CellCoord,
        /// Failing check or budget reason.
        reason: &'static str,
    },
}

/// Exact byte cost of one baked cell of `size`; units: bytes.
pub fn cell_cost_bytes(size: u16) -> Result<u64, CellError> {
    if !(CELL_MIN_SIZE..=CELL_MAX_SIZE).contains(&size) {
        return Err(CellError::BadSize { got: size });
    }
    let n = u64::from(size);
    Ok(CELL_HEADER_LEN as u64 + n * n + (n * n).div_ceil(8))
}

/// Thermal erosion over the halo buffer: pairwise talus exchange, Jacobi,
/// fixed order (parity with the terrain bake, 4 passes).
fn erode(buffer: &mut [u8], side: usize) {
    if side < 2 {
        return;
    }
    let mut delta = vec![0i16; side * side];
    for _ in 0..4 {
        let src: Vec<i16> = buffer.iter().map(|v| i16::from(*v)).collect();
        delta.fill(0);
        for y in 0..side {
            for x in 0..side {
                let i = y * side + x;
                if x + 1 < side {
                    let j = y * side + x + 1;
                    let diff = src[i] - src[j];
                    if diff > TALUS {
                        let m = (diff - TALUS) / 4;
                        delta[i] -= m;
                        delta[j] += m;
                    } else if -diff > TALUS {
                        let m = (-diff - TALUS) / 4;
                        delta[j] -= m;
                        delta[i] += m;
                    }
                }
                if y + 1 < side {
                    let j = (y + 1) * side + x;
                    let diff = src[i] - src[j];
                    if diff > TALUS {
                        let m = (diff - TALUS) / 4;
                        delta[i] -= m;
                        delta[j] += m;
                    } else if -diff > TALUS {
                        let m = (-diff - TALUS) / 4;
                        delta[j] -= m;
                        delta[i] += m;
                    }
                }
            }
        }
        for (v, d) in buffer.iter_mut().zip(delta.iter()) {
            *v = (i16::from(*v) + *d).clamp(0, 255) as u8;
        }
    }
}

/// Bake one cell: header plus eroded heights plus isolated-hole bits.
/// Global sampling with a halo keeps shared edges seamless: the halo covers
/// erosion reach, holes are window-independent global singles, and bytes are
/// identical twice.
pub fn bake_cell(seed: u64, coord: CellCoord, size: u16) -> Result<Vec<u8>, CellError> {
    if !(CELL_MIN_SIZE..=CELL_MAX_SIZE).contains(&size) {
        return Err(CellError::BadSize { got: size });
    }
    let n = usize::from(size);
    let halo = CELL_HALO as usize;
    let wide = n + 2 * halo;
    // Global origin of the halo in texels; pitch is the cell side.
    let ox = coord.x.wrapping_mul(i32::from(size));
    let oy = coord.z.wrapping_mul(i32::from(size));
    let mut halo_heights = vec![0u8; wide * wide];
    for y in 0..wide {
        for x in 0..wide {
            let gx = ox + x as i32 - CELL_HALO;
            let gy = oy + y as i32 - CELL_HALO;
            halo_heights[y * wide + x] = base_height(seed, gx, gy);
        }
    }
    erode(&mut halo_heights, wide);
    let mut heights = vec![0u8; n * n];
    for y in 0..n {
        for x in 0..n {
            heights[y * n + x] = halo_heights[(y + halo) * wide + (x + halo)];
        }
    }
    // Holes are window-independent global singles: the raw bit comes from the
    // global hash and any raw hole touching an 8-neighbour raw hole (in global
    // coords) is dropped, so holes nest exactly across scales and a single
    // cell never disconnects its grid. Sparse isolated holes can never wall a
    // whole edge, so walkers always find a gap.
    let raw_at = |gx: i32, gy: i32| -> bool {
        ((lattice(seed.wrapping_add(HOLE_TAG), gx, gy) >> 56) as u8) > HOLE_CUT
    };
    let mut holes = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            let (gx, gy) = (ox + x as i32, oy + y as i32);
            if !raw_at(gx, gy) {
                continue;
            }
            let mut adjacent = false;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    if raw_at(gx + dx, gy + dy) {
                        adjacent = true;
                        break;
                    }
                }
                if adjacent {
                    break;
                }
            }
            holes[y * n + x] = !adjacent;
        }
    }
    let hole_bytes = (n * n).div_ceil(8);
    let mut out = Vec::with_capacity(CELL_HEADER_LEN + n * n + hole_bytes);
    out.extend_from_slice(&CELL_MAGIC);
    out.push(CELL_VERSION);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&coord.x.to_le_bytes());
    out.extend_from_slice(&coord.z.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&heights);
    for i in 0..hole_bytes {
        let mut byte = 0u8;
        for k in 0..8 {
            let idx = i * 8 + k;
            if idx < n * n && holes[idx] {
                byte |= 1 << k;
            }
        }
        out.push(byte);
    }
    Ok(out)
}

/// Decoded cell: heights plus the per-cell nav/physics binding derived from them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellData {
    /// Bake seed.
    pub seed: u64,
    /// Cell address.
    pub coord: CellCoord,
    /// Cell side in texels.
    pub size: u16,
    /// Row-major eroded heights, `size * size` entries.
    pub heights: Vec<u8>,
    /// Isolated holes; borders are always free.
    pub holes: Vec<bool>,
}

impl CellData {
    /// Walkable texels: not a hole and slope within `SLOPE_LIMIT`.
    pub fn walkable(&self) -> Vec<bool> {
        let n = usize::from(self.size);
        let mut out = vec![false; n * n];
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                out[i] = !self.holes[i] && slope_of(&self.heights, n, x, y) <= SLOPE_LIMIT;
            }
        }
        // Endpoints stay free so a scripted walk never starts stuck.
        if n > 0 {
            out[0] = true;
            out[n * n - 1] = true;
        }
        out
    }

    /// Walkable texel count; units: texels.
    pub fn walkable_count(&self) -> usize {
        self.walkable().iter().filter(|v| **v).count()
    }

    /// Physics colliders bound for this cell: one box per solid texel.
    /// Deterministic count the map loader registers with the physics stack.
    pub fn collider_count(&self) -> u32 {
        let n = usize::from(self.size);
        let walk = self.walkable();
        let mut count = 0u32;
        for solid in walk.iter().map(|walkable| !walkable).take(n * n) {
            if solid {
                count = count.saturating_add(1);
            }
        }
        count
    }

    /// Canonical bytes; identical inputs bake identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let n = usize::from(self.size);
        let hole_bytes = (n * n).div_ceil(8);
        let mut out = Vec::with_capacity(CELL_HEADER_LEN + n * n + hole_bytes);
        out.extend_from_slice(&CELL_MAGIC);
        out.push(CELL_VERSION);
        out.extend_from_slice(&self.seed.to_le_bytes());
        out.extend_from_slice(&self.coord.x.to_le_bytes());
        out.extend_from_slice(&self.coord.z.to_le_bytes());
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.heights);
        for i in 0..hole_bytes {
            let mut byte = 0u8;
            for k in 0..8 {
                let idx = i * 8 + k;
                if idx < n * n && self.holes[idx] {
                    byte |= 1 << k;
                }
            }
            out.push(byte);
        }
        out
    }
}

/// Decode and validate a cell blob; any non-bake blob refuses with its check.
pub fn parse_cell(bytes: &[u8]) -> Result<CellData, CellError> {
    if bytes.len() < CELL_HEADER_LEN {
        return Err(CellError::Corrupt("truncated"));
    }
    if bytes[0..4] != CELL_MAGIC {
        return Err(CellError::Corrupt("magic"));
    }
    if bytes[4] != CELL_VERSION {
        return Err(CellError::Corrupt("version"));
    }
    let mut seed = [0u8; 8];
    seed.copy_from_slice(&bytes[5..13]);
    let mut cx = [0u8; 4];
    cx.copy_from_slice(&bytes[13..17]);
    let mut cz = [0u8; 4];
    cz.copy_from_slice(&bytes[17..21]);
    let mut size = [0u8; 2];
    size.copy_from_slice(&bytes[21..23]);
    let (seed, size) = (u64::from_le_bytes(seed), u16::from_le_bytes(size));
    let coord = CellCoord::new(i32::from_le_bytes(cx), i32::from_le_bytes(cz))
        .map_err(|_| CellError::Corrupt("coord"))?;
    if !(CELL_MIN_SIZE..=CELL_MAX_SIZE).contains(&size) {
        return Err(CellError::Corrupt("size"));
    }
    let n = usize::from(size);
    let want = CELL_HEADER_LEN + n * n + (n * n).div_ceil(8);
    if bytes.len() < want {
        return Err(CellError::Corrupt("truncated"));
    }
    if bytes.len() > want {
        return Err(CellError::Corrupt("trailing"));
    }
    let heights = bytes[CELL_HEADER_LEN..CELL_HEADER_LEN + n * n].to_vec();
    let mut holes = vec![false; n * n];
    for i in 0..n * n {
        holes[i] = ((bytes[CELL_HEADER_LEN + n * n + i / 8] >> (i % 8)) & 1) == 1;
    }
    Ok(CellData {
        seed,
        coord,
        size,
        heights,
        holes,
    })
}

/// Geometric seam scan between two baked neighbours: same seed and size,
/// adjacent coords, height steps within `SEAM_TOL`. Holes are global singles
/// shared by construction, so they carry no seam information and are not
/// scanned here. Names the failing check; never guesses across a broken seam.
pub fn seam_joins(west_or_north: &CellData, east_or_south: &CellData) -> Result<(), CellError> {
    if west_or_north.seed != east_or_south.seed {
        return Err(CellError::SeamMismatch("seed"));
    }
    if west_or_north.size != east_or_south.size {
        return Err(CellError::SeamMismatch("size"));
    }
    let (a, b) = (west_or_north.coord, east_or_south.coord);
    let east = b.x == a.x + 1 && b.z == a.z;
    let south = b.x == a.x && b.z == a.z + 1;
    if !east && !south {
        return Err(CellError::SeamMismatch("adjacent"));
    }
    let n = usize::from(west_or_north.size);
    for k in 0..n {
        let (ha, hb) = if east {
            (
                west_or_north.heights[k * n + (n - 1)],
                east_or_south.heights[k * n],
            )
        } else {
            (
                west_or_north.heights[(n - 1) * n + k],
                east_or_south.heights[k],
            )
        };
        let step = (i16::from(ha) - i16::from(hb)).unsigned_abs();
        if step > u16::from(SEAM_TOL) {
            return Err(CellError::SeamMismatch("step"));
        }
    }
    Ok(())
}

/// True when walkable texels connect the forced-free endpoints of one cell.
/// Units: texels in, bool out. Determinism: fixed 4-dir order, visits bounded
/// by `size * size`.
fn cell_endpoints_connect(data: &CellData) -> bool {
    let n = usize::from(data.size);
    if n == 0 {
        return false;
    }
    let walk = data.walkable();
    if walk.len() != n * n || !walk[0] {
        return false;
    }
    if !walk[n * n - 1] {
        return false;
    }
    let mut seen = vec![false; n * n];
    let mut queue = VecDeque::new();
    queue.push_back((0usize, 0usize));
    seen[0] = true;
    while let Some((x, y)) = queue.pop_front() {
        if x + 1 == n && y + 1 == n {
            return true;
        }
        if x + 1 < n && !seen[y * n + x + 1] && walk[y * n + x + 1] {
            seen[y * n + x + 1] = true;
            queue.push_back((x + 1, y));
        }
        if x > 0 && !seen[y * n + x - 1] && walk[y * n + x - 1] {
            seen[y * n + x - 1] = true;
            queue.push_back((x - 1, y));
        }
        if y + 1 < n && !seen[(y + 1) * n + x] && walk[(y + 1) * n + x] {
            seen[(y + 1) * n + x] = true;
            queue.push_back((x, y + 1));
        }
        if y > 0 && !seen[(y - 1) * n + x] && walk[(y - 1) * n + x] {
            seen[(y - 1) * n + x] = true;
            queue.push_back((x, y - 1));
        }
    }
    false
}

/// One pending slot: reservation plus in-memory progress markers.
#[derive(Clone, Debug)]
struct PendingCell {
    size: u16,
    priority: u8,
    cost_bytes: u64,
    done_bytes: u64,
    started: bool,
    touches: u64,
}

/// One live slot: baked bytes plus the nav/physics binding the tick reads.
#[derive(Clone, Debug)]
struct LiveCell {
    size: u16,
    bytes: Vec<u8>,
    walkable: Vec<bool>,
    colliders: u32,
    load_frames: u64,
}

/// Async cell registry over the stream queue shape: priority ordering, a
/// memory budget with a high-water mark plus over-budget refusal, chunked
/// progress published at frame boundaries, exact eviction, and one repair per
/// coord.
/// Single-thread driven: cook work completes off-tick, the owner calls
/// `publish` with a per-frame byte cap, and the tick only reads live cells.
#[derive(Clone, Debug)]
pub struct Cells {
    seed: u64,
    budget: CellBudget,
    pending: BTreeMap<CellCoord, PendingCell>,
    live: BTreeMap<CellCoord, LiveCell>,
    used_bytes: u64,
    high_water_bytes: u64,
    repaired: BTreeSet<CellCoord>,
    refused_count: u64,
}

impl Cells {
    /// Empty registry under `budget`; nothing reserved, high-water zero.
    pub fn new(seed: u64, budget: CellBudget) -> Self {
        Self {
            seed,
            budget,
            pending: BTreeMap::new(),
            live: BTreeMap::new(),
            used_bytes: 0,
            high_water_bytes: 0,
            repaired: BTreeSet::new(),
            refused_count: 0,
        }
    }

    /// Bake seed every cell in this registry shares.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Active budget (byte cap, job slots, frame cap).
    pub fn budget(&self) -> CellBudget {
        self.budget
    }

    /// Reserved plus live bytes; units: bytes. Never above the byte budget.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// Peak of [`Cells::used_bytes`] since construction; never falls. Units: bytes.
    pub fn high_water_bytes(&self) -> u64 {
        self.high_water_bytes
    }

    /// Bytes still admittable; units: bytes.
    pub fn remaining_bytes(&self) -> u64 {
        self.budget.bytes.saturating_sub(self.used_bytes)
    }

    /// Job slots still admittable (pending plus live count against it).
    pub fn remaining_jobs(&self) -> u64 {
        u64::from(self.budget.jobs).saturating_sub(self.admitted() as u64)
    }

    /// Coords awaiting completion, in publish order (priority first, coord asc).
    pub fn pending_order(&self) -> Vec<CellCoord> {
        let mut order: Vec<(&CellCoord, &PendingCell)> = self.pending.iter().collect();
        order.sort_by(|left, right| {
            right
                .1
                .priority
                .cmp(&left.1.priority)
                .then_with(|| left.0.cmp(right.0))
        });
        order.into_iter().map(|entry| *entry.0).collect()
    }

    /// Live coords ascending for a stable loader list.
    pub fn live_coords(&self) -> Vec<CellCoord> {
        self.live.keys().copied().collect()
    }

    /// Count of queued cells awaiting completion.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Count of live cells the tick may read.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// True while the coord is queued but not live: the loader shows a
    /// placeholder, never final-looking geometry.
    pub fn is_placeholder(&self, coord: &CellCoord) -> bool {
        self.pending.contains_key(coord)
    }

    /// True once the coord completed and has not been evicted.
    pub fn is_live(&self, coord: &CellCoord) -> bool {
        self.live.contains_key(coord)
    }

    /// Streamed bytes versus total for one coord: pending reports partial
    /// progress, live reports full, unknown reports `None`. Units: bytes.
    pub fn progress_of(&self, coord: &CellCoord) -> Option<(u64, u64)> {
        if let Some(slot) = self.pending.get(coord) {
            return Some((slot.done_bytes, slot.cost_bytes));
        }
        self.live
            .get(coord)
            .map(|slot| (slot.bytes.len() as u64, slot.bytes.len() as u64))
    }

    /// Frames of `publish` calls that advanced this live cell: its named load
    /// time. Unknown coords report `None`.
    pub fn load_frames(&self, coord: &CellCoord) -> Option<u64> {
        self.live.get(coord).map(|slot| slot.load_frames)
    }

    /// Live byte cost of one cell; units: bytes. Unknown coords report `None`.
    pub fn cell_bytes(&self, coord: &CellCoord) -> Option<u64> {
        self.live.get(coord).map(|slot| slot.bytes.len() as u64)
    }

    /// Collider count bound for one live cell. Unknown coords report `None`.
    pub fn collider_count(&self, coord: &CellCoord) -> Option<u32> {
        self.live.get(coord).map(|slot| slot.colliders)
    }

    /// Refused-cell count since construction (budgets plus corrupt bakes).
    pub fn refused_count(&self) -> u64 {
        self.refused_count
    }

    /// Queue one cell read. Live re-requests answer `Completed` inline with no
    /// new reservation; pending re-requests fail `Duplicate`; cells past either
    /// budget fail `OverBudget` or `JobCap`. Higher `priority` streams first so
    /// the map loader marks fight-critical cells first.
    pub fn stream(
        &mut self,
        coord: CellCoord,
        size: u16,
        priority: u8,
    ) -> Result<CellEvent, CellError> {
        if !(CELL_MIN_SIZE..=CELL_MAX_SIZE).contains(&size) {
            return Err(CellError::BadSize { got: size });
        }
        if self.live.contains_key(&coord) {
            return Ok(CellEvent::Completed { coord });
        }
        if self.pending.contains_key(&coord) {
            return Err(CellError::Duplicate);
        }
        if self.admitted() >= self.budget.jobs as usize {
            self.refused_count = self.refused_count.saturating_add(1);
            return Err(CellError::JobCap {
                limit: self.budget.jobs,
            });
        }
        for (held, ceiling) in [
            (self.admitted(), CELLS_MAX_ADMITTED),
            (self.pending.len(), CELLS_MAX_PENDING),
        ] {
            if held >= ceiling {
                self.refused_count = self.refused_count.saturating_add(1);
                return Err(CellError::JobCap {
                    limit: u32::try_from(ceiling).unwrap_or(u32::MAX),
                });
            }
        }
        let cost = cell_cost_bytes(size)?;
        if cost > CELL_MAX_BYTES {
            return Err(CellError::TooLarge {
                count: cost as usize,
            });
        }
        if self.used_bytes.saturating_add(cost) > self.budget.bytes {
            self.refused_count = self.refused_count.saturating_add(1);
            return Err(CellError::OverBudget {
                want_bytes: cost,
                free_bytes: self.budget.bytes.saturating_sub(self.used_bytes),
            });
        }
        self.pending.insert(
            coord,
            PendingCell {
                size,
                priority,
                cost_bytes: cost,
                done_bytes: 0,
                started: false,
                touches: 0,
            },
        );
        self.used_bytes = self.used_bytes.saturating_add(cost);
        self.high_water_bytes = self.high_water_bytes.max(self.used_bytes);
        Ok(CellEvent::Queued { coord })
    }

    /// Dual ahead-agent publish gate: geometric seam scan plus navigational walk.
    /// Units: cells in, unit out. Errors: `SeamMismatch` with the failing seam
    /// check named, `Unreachable` when the walkable endpoints disconnect.
    /// Determinism: `BTreeMap` order, integer math only, bounded flood.
    pub fn gate_for_live(&self, candidate: &CellData) -> Result<(), CellError> {
        if candidate.seed != self.seed {
            return Err(CellError::SeamMismatch("seed"));
        }
        for (coord, live) in self.live.iter() {
            let dx = candidate.coord.x.wrapping_sub(coord.x);
            let dz = candidate.coord.z.wrapping_sub(coord.z);
            if !((dx == 1 || dx == -1) && dz == 0 || dx == 0 && (dz == 1 || dz == -1)) {
                continue;
            }
            let neighbour = parse_cell(&live.bytes).map_err(|_| CellError::Corrupt("live"))?;
            if dx == 1 || dz == 1 {
                seam_joins(&neighbour, candidate)?;
            } else {
                seam_joins(candidate, &neighbour)?;
            }
        }
        if !cell_endpoints_connect(candidate) {
            return Err(CellError::Unreachable);
        }
        Ok(())
    }

    /// Advance queued cells by at most `frame_bytes` at a frame boundary, in
    /// deterministic publish order. Partial cells emit `Progress` and stay
    /// pending; finished cells bake, bind nav plus physics, and go live with
    /// `Completed`. A zero cap emits nothing and touches nothing, so the tick
    /// only ever reads live cells and never waits. At most `PUBLISH_MAX_EVENTS`
    /// events leave per call; the rest waits for the next frame boundary.
    pub fn publish(&mut self, frame_bytes: u64) -> Vec<CellEvent> {
        let mut events = Vec::new();
        if frame_bytes == 0 || self.pending.is_empty() {
            return events;
        }
        let mut order = self.pending_order();
        // Re-sort guard: `pending_order` already sorts, this keeps the loop
        // total under the borrow checker without re-borrowing the map.
        order.sort_by(|left, right| {
            let lp = self.pending.get(left).map_or(0, |slot| slot.priority);
            let rp = self.pending.get(right).map_or(0, |slot| slot.priority);
            rp.cmp(&lp).then_with(|| left.cmp(right))
        });
        let mut left = frame_bytes;
        for coord in order {
            if left == 0 {
                break;
            }
            // Event cap first: keep one cell's group (Started/Progress/Completed)
            // atomic so a truncated call never advances bytes without reporting.
            if events.len() + 3 > PUBLISH_MAX_EVENTS {
                break;
            }
            let Some(slot) = self.pending.get_mut(&coord) else {
                continue;
            };
            let chunk = slot.cost_bytes.saturating_sub(slot.done_bytes).min(left);
            if chunk == 0 {
                continue;
            }
            if !slot.started {
                slot.started = true;
                events.push(CellEvent::Started { coord });
            }
            slot.done_bytes = slot.done_bytes.saturating_add(chunk);
            slot.touches = slot.touches.saturating_add(1);
            left = left.saturating_sub(chunk);
            let (done, total, touches) = (slot.done_bytes, slot.cost_bytes, slot.touches);
            events.push(CellEvent::Progress {
                coord,
                done_bytes: done,
                total_bytes: total,
            });
            if done >= total {
                let size = slot.size;
                self.pending.remove(&coord);
                // Bake at the frame boundary from the registry seed: pure and
                // byte-identical twice, so publish timing never forks content.
                match bake_cell(self.seed, coord, size) {
                    Ok(bytes) => match parse_cell(&bytes) {
                        Ok(parsed) => match self.gate_for_live(&parsed) {
                            Ok(()) => {
                                let walkable = parsed.walkable();
                                let colliders = parsed.collider_count();
                                self.live.insert(
                                    coord,
                                    LiveCell {
                                        size,
                                        bytes,
                                        walkable,
                                        colliders,
                                        load_frames: touches,
                                    },
                                );
                                events.push(CellEvent::Completed { coord });
                            }
                            Err(gate_err) => {
                                self.used_bytes = self.used_bytes.saturating_sub(total);
                                self.refused_count = self.refused_count.saturating_add(1);
                                events.push(CellEvent::Refused {
                                    coord,
                                    reason: refuse_name(&gate_err),
                                });
                            }
                        },
                        Err(parse_err) => {
                            self.used_bytes = self.used_bytes.saturating_sub(total);
                            self.refused_count = self.refused_count.saturating_add(1);
                            events.push(CellEvent::Refused {
                                coord,
                                reason: refuse_name(&parse_err),
                            });
                        }
                    },
                    Err(reason) => {
                        self.used_bytes = self.used_bytes.saturating_sub(total);
                        self.refused_count = self.refused_count.saturating_add(1);
                        events.push(CellEvent::Refused {
                            coord,
                            reason: refuse_name(&reason),
                        });
                    }
                }
            }
        }
        events
    }

    /// Drop a pending cell before completion and free its reservation exactly.
    pub fn cancel(&mut self, coord: &CellCoord) -> Result<CellEvent, CellError> {
        if self.live.contains_key(coord) {
            return Err(CellError::LiveNeedsEvict);
        }
        let slot = self.pending.remove(coord).ok_or(CellError::Unknown)?;
        self.used_bytes = self.used_bytes.saturating_sub(slot.cost_bytes);
        Ok(CellEvent::Cancelled { coord: *coord })
    }

    /// Free one live cell chosen by the consumer (distance/use) and return its
    /// bytes to the budget. Freed bytes always equal the baked cell cost, so
    /// eviction is exact and the 30-minute loop cannot drift. Pending coords
    /// need `cancel`, not evict.
    pub fn evict(&mut self, coord: &CellCoord) -> Result<CellEvent, CellError> {
        if self.pending.contains_key(coord) {
            return Err(CellError::PendingNeedsCancel);
        }
        let slot = self.live.remove(coord).ok_or(CellError::Unknown)?;
        let freed = slot.bytes.len() as u64;
        self.used_bytes = self.used_bytes.saturating_sub(freed);
        Ok(CellEvent::Evicted {
            coord: *coord,
            freed_bytes: freed,
        })
    }

    /// Re-queue one refused coord exactly once (self-repair budget of one).
    /// A second repair without new information fails `AlreadyRepaired` so a
    /// poisoned coord cannot spin the queue forever.
    pub fn repair(
        &mut self,
        coord: CellCoord,
        size: u16,
        priority: u8,
    ) -> Result<CellEvent, CellError> {
        if self.repaired.contains(&coord) {
            return Err(CellError::AlreadyRepaired);
        }
        let event = self.stream(coord, size, priority)?;
        self.repaired.insert(coord);
        Ok(event)
    }

    /// Waypoint step count from one live texel to another across live cells.
    /// Either endpoint outside a live cell reports `NotLive`; walkable cells
    /// with no path report `Unreachable`. Deterministic 4-dir flood fill over
    /// the union of live texels in global texel coords. At most
    /// `ROUTE_MAX_VISITS` texels are visited; past it answers `RouteBudget`
    /// so one query never stalls the tick.
    pub fn route(
        &self,
        from_cell: CellCoord,
        from_texel: (u16, u16),
        to_cell: CellCoord,
        to_texel: (u16, u16),
    ) -> Result<usize, CellError> {
        let from = self.live.get(&from_cell).ok_or(CellError::NotLive)?;
        let to = self.live.get(&to_cell).ok_or(CellError::NotLive)?;
        let (fn_side, tn_side) = (usize::from(from.size), usize::from(to.size));
        if usize::from(from_texel.0) >= fn_side
            || usize::from(from_texel.1) >= fn_side
            || usize::from(to_texel.0) >= tn_side
            || usize::from(to_texel.1) >= tn_side
        {
            return Err(CellError::BadIndex);
        }
        let global = |cell: CellCoord, size: u16, texel: (u16, u16)| -> (i64, i64) {
            (
                i64::from(cell.x) * i64::from(size) + i64::from(texel.0),
                i64::from(cell.z) * i64::from(size) + i64::from(texel.1),
            )
        };
        let start = global(from_cell, from.size, from_texel);
        let goal = global(to_cell, to.size, to_texel);
        if !self.walkable_global(start) || !self.walkable_global(goal) {
            return Err(CellError::Unreachable);
        }
        if start == goal {
            return Ok(1);
        }
        // Bound the fill to the live bounding box so far walkers stay bounded.
        let mut queue = VecDeque::new();
        let mut seen = BTreeSet::new();
        let mut parent: BTreeMap<(i64, i64), (i64, i64)> = BTreeMap::new();
        queue.push_back(start);
        seen.insert(start);
        parent.insert(start, start);
        let mut found = false;
        while let Some(at) = queue.pop_front() {
            if at == goal {
                found = true;
                break;
            }
            for step in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                let next = (at.0 + step.0, at.1 + step.1);
                if seen.contains(&next) || !self.walkable_global(next) {
                    continue;
                }
                if seen.len() >= ROUTE_MAX_VISITS {
                    return Err(CellError::RouteBudget {
                        limit: u32::try_from(ROUTE_MAX_VISITS).unwrap_or(u32::MAX),
                    });
                }
                seen.insert(next);
                parent.insert(next, at);
                queue.push_back(next);
            }
        }
        if !found {
            return Err(CellError::Unreachable);
        }
        let mut steps = 1usize;
        let mut at = goal;
        while at != start {
            let Some(prev) = parent.get(&at) else {
                return Err(CellError::Unreachable);
            };
            at = *prev;
            steps = steps.saturating_add(1);
        }
        Ok(steps)
    }

    /// Sim hash over live cells only: sorted coords plus cell bytes. The
    /// day-night cycle clock and any other presentation value never enter this fold, so
    /// the golden hash with presentation on equals the hash with it off.
    pub fn sim_hash(&self) -> u64 {
        let mut hash = Fnv64::new();
        for (coord, cell) in self.live.iter() {
            hash.write_i32(coord.x);
            hash.write_i32(coord.z);
            hash.write(&cell.bytes);
        }
        hash.finish()
    }

    /// Digest of one baked blob for the byte-identical gate (FNV-1a 64, the
    /// sim-hash primitive, never a security digest).
    pub fn digest(bytes: &[u8]) -> u64 {
        hash_bytes(bytes)
    }

    /// Admitted cell count (pending plus live) against the slot budget.
    fn admitted(&self) -> usize {
        self.pending.len().saturating_add(self.live.len())
    }

    /// Walkable state of one global texel; unloaded cells read solid (wait,
    /// never guess), which is what turns late cells into `NotLive`/`Unreachable`.
    fn walkable_global(&self, pos: (i64, i64)) -> bool {
        for (coord, cell) in self.live.iter() {
            let side = i64::from(cell.size);
            let ox = i64::from(coord.x) * side;
            let oy = i64::from(coord.z) * side;
            if pos.0 >= ox && pos.0 < ox + side && pos.1 >= oy && pos.1 < oy + side {
                let lx = (pos.0 - ox) as usize;
                let ly = (pos.1 - oy) as usize;
                let idx = ly * usize::from(cell.size) + lx;
                return cell.walkable.get(idx).copied().unwrap_or(false);
            }
        }
        false
    }
}

/// Static refusal name for a bake failure at publish time.
fn refuse_name(reason: &CellError) -> &'static str {
    match reason {
        CellError::BadSize { .. } => "bad-size",
        CellError::BadCoord => "bad-coord",
        CellError::Corrupt(check) => check,
        CellError::SeamMismatch(check) => check,
        CellError::Unreachable => "unreachable",
        _ => "bake",
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;

    fn huge_budget() -> CellBudget {
        CellBudget::new(u64::MAX, u32::MAX, u64::MAX).expect("huge budget nonzero")
    }

    fn live_block(side: i32, size: u16) -> Cells {
        let count = u64::from(side.unsigned_abs()).pow(2);
        let budget =
            CellBudget::new(count * 8192, u32::try_from(count).unwrap(), u64::MAX).expect("budget");
        let mut cells = Cells::new(12345, budget);
        for x in 0..side {
            for z in 0..side {
                let coord = CellCoord::new(x, z).expect("block coord");
                cells.stream(coord, size, 1).expect("fits");
            }
        }
        while cells.pending_count() > 0 {
            cells.publish(u64::MAX);
        }
        cells
    }

    /// First walkable global texel scanning from `from` along the diagonal.
    fn walkable_near(cells: &Cells, from: (i64, i64), step: i64) -> (i64, i64) {
        (0..64)
            .map(|k| (from.0 + k * step, from.1 + k * step))
            .find(|texel| cells.walkable_global(*texel))
            .expect("a walkable texel near the corner")
    }

    fn texel_of(global: (i64, i64), size: u16) -> (CellCoord, (u16, u16)) {
        let side = i64::from(size);
        let cell = |g: i64| i32::try_from(g.div_euclid(side)).expect("block coord");
        let local = |g: i64| u16::try_from(g.rem_euclid(side)).expect("texel");
        (
            CellCoord::new(cell(global.0), cell(global.1)).expect("block coord"),
            (local(global.0), local(global.1)),
        )
    }

    #[test]
    fn an_honest_cell_costs_far_less_than_the_ceiling() {
        let top = cell_cost_bytes(CELL_MAX_SIZE).expect("max side valid");
        assert!(top < CELL_MAX_BYTES / 1024, "biggest bake costs {top}");
        let bottom = cell_cost_bytes(CELL_MIN_SIZE).expect("min side valid");
        assert!(bottom <= top);
    }

    #[test]
    fn cell_cost_bytes_match_the_header_layout_exactly() {
        // Hand-computed: header(23) + n*n heights + ceil(n*n/8) hole bits.
        assert_eq!(cell_cost_bytes(4).expect("min side valid"), 23 + 16 + 2);
        assert_eq!(
            cell_cost_bytes(64).expect("max side valid"),
            23 + 4096 + 512
        );
        assert_eq!(
            cell_cost_bytes(16).expect("16 valid"),
            23 + 256 + 32,
            "16-texel cost pins the header plus heights plus hole bits"
        );
        assert!(cell_cost_bytes(3).is_err(), "below the min side refuses");
        assert!(cell_cost_bytes(65).is_err(), "past the max side refuses");
    }

    #[test]
    fn pending_hard_ceiling_holds_with_huge_budget() {
        let mut cells = Cells::new(7, huge_budget());
        for i in 0..CELLS_MAX_PENDING {
            let i = i32::try_from(i).expect("small");
            let coord = CellCoord::new(i % 64, i / 64).expect("fixture coord");
            assert!(
                cells.stream(coord, CELL_MIN_SIZE, 1).is_ok(),
                "slot {i} fits under the ceiling"
            );
        }
        assert_eq!(cells.pending_count(), CELLS_MAX_PENDING);
        let extra = CellCoord::new(63, 63).expect("extra coord");
        let refused_before = cells.refused_count();
        assert_eq!(
            cells.stream(extra, CELL_MIN_SIZE, 1),
            Err(CellError::JobCap {
                limit: u32::try_from(CELLS_MAX_PENDING).unwrap()
            }),
            "pending past the ceiling refuses"
        );
        assert_eq!(cells.pending_count(), CELLS_MAX_PENDING);
        assert_eq!(cells.refused_count(), refused_before + 1);
    }

    #[test]
    fn admitted_hard_ceiling_counts_live_cells_too() {
        let mut cells = Cells::new(9, huge_budget());
        let mut next = 0i32;
        // Live cells pile up while the pending queue drains in slices, so the
        // admitted total (live plus pending) is what reaches its ceiling.
        while cells.admitted() < CELLS_MAX_ADMITTED {
            let coord = CellCoord::new(next % 64, next / 64).expect("fixture coord");
            match cells.stream(coord, CELL_MIN_SIZE, 1) {
                Ok(_) => next += 1,
                Err(CellError::JobCap { .. }) => {
                    cells.publish(u64::MAX);
                }
                Err(other) => panic!("unexpected refusal {other:?}"),
            }
        }
        assert_eq!(cells.admitted(), CELLS_MAX_ADMITTED);
        assert!(cells.live_count() > 0, "the ceiling held live cells too");
        let extra = CellCoord::new(-1, -1).expect("extra coord");
        assert_eq!(
            cells.stream(extra, CELL_MIN_SIZE, 1),
            Err(CellError::JobCap {
                limit: u32::try_from(CELLS_MAX_ADMITTED).unwrap()
            })
        );
    }

    #[test]
    fn publish_event_cap_holds_and_drains() {
        let budget = CellBudget::new(64 * 1024, 64, u64::MAX).expect("budget");
        let mut cells = Cells::new(11, budget);
        let want = 30usize;
        for i in 0..30 {
            let coord = CellCoord::new(i, 0).expect("line coord");
            cells.stream(coord, CELL_MIN_SIZE, 1).expect("fits");
        }
        let first = cells.publish(u64::MAX);
        assert!(
            first.len() <= PUBLISH_MAX_EVENTS,
            "one call emits at most {PUBLISH_MAX_EVENTS}, got {}",
            first.len()
        );
        assert!(
            cells.pending_count() > 0,
            "remainder waits for the next boundary"
        );
        let mut total = first.len();
        let mut rounds = 1u32;
        while cells.pending_count() > 0 {
            let events = cells.publish(u64::MAX);
            assert!(events.len() <= PUBLISH_MAX_EVENTS, "every call capped");
            assert!(!events.is_empty(), "drain makes progress");
            total += events.len();
            rounds += 1;
            assert!(rounds < 32, "drain finishes in bounded calls");
        }
        assert_eq!(cells.live_count(), want);
        assert_eq!(
            total,
            want * 3,
            "each cell reports Started, Progress, Completed"
        );
    }

    #[test]
    fn route_inside_the_visit_cap_answers_and_past_it_says_budget() {
        // 3 x 3 cells of 64 texels is 36,864 texels: under the cap, so a
        // corner to corner walk answers with a step count.
        let small = live_block(3, CELL_MAX_SIZE);
        let from = walkable_near(&small, (0, 0), 1);
        let to = walkable_near(&small, (191, 191), -1);
        let ((fc, ft), (tc, tt)) = (texel_of(from, CELL_MAX_SIZE), texel_of(to, CELL_MAX_SIZE));
        let steps = small.route(fc, ft, tc, tt);
        assert!(
            matches!(steps, Ok(n) if n > 1) || steps == Err(CellError::Unreachable),
            "small block answers: {steps:?}"
        );
        // 5 x 5 cells is 102,400 texels: the flood hits the ceiling before
        // it can reach the far corner and says so instead of "no path".
        let big = live_block(5, CELL_MAX_SIZE);
        let from = walkable_near(&big, (0, 0), 1);
        let to = walkable_near(&big, (319, 319), -1);
        let ((fc, ft), (tc, tt)) = (texel_of(from, CELL_MAX_SIZE), texel_of(to, CELL_MAX_SIZE));
        assert_eq!(
            big.route(fc, ft, tc, tt),
            Err(CellError::RouteBudget {
                limit: u32::try_from(ROUTE_MAX_VISITS).unwrap()
            })
        );
    }

    #[test]
    fn disconnected_islands_fail_closed() {
        let budget = CellBudget::new(9 * 1024, 9, u64::MAX).expect("budget");
        let mut solo = Cells::new(12345, budget);
        let a = CellCoord::new(0, 0).expect("hub");
        let b = CellCoord::new(5, 5).expect("far");
        solo.stream(a, 16, 9).expect("fits");
        solo.stream(b, 16, 1).expect("fits");
        solo.publish(u64::MAX);
        assert_eq!(
            solo.route(a, (0, 0), b, (0, 0)),
            Err(CellError::Unreachable)
        );
    }

    #[test]
    fn dual_gate_refuses_broken_seam_with_named_check() {
        let cost = cell_cost_bytes(16).expect("16 valid");
        let budget = CellBudget::new(4 * cost, 4, u64::MAX).expect("budget");
        let mut cells = Cells::new(12345, budget);
        let hub = CellCoord::new(0, 0).expect("hub");
        cells.stream(hub, 16, 9).expect("hub fits");
        cells.publish(u64::MAX);
        assert!(cells.is_live(&hub), "hub goes live through the gate");
        let east_coord = CellCoord::new(1, 0).expect("east");
        let honest =
            parse_cell(&bake_cell(12345, east_coord, 16).expect("east bakes")).expect("parses");
        assert_eq!(
            cells.gate_for_live(&honest),
            Ok(()),
            "honest neighbour passes"
        );
        let west = parse_cell(&bake_cell(12345, hub, 16).expect("hub bakes")).expect("parses");
        let mut cliff = honest.clone();
        for k in 0..16usize {
            cliff.heights[k * 16] = west.heights[k * 16 + 15] ^ 0x80;
        }
        assert_eq!(
            seam_joins(&west, &cliff),
            Err(CellError::SeamMismatch("step"))
        );
        assert_eq!(
            cells.gate_for_live(&cliff),
            Err(CellError::SeamMismatch("step")),
            "cliff names its check"
        );
        let mut wrong_seed = honest.clone();
        wrong_seed.seed = 12345 + 1;
        assert_eq!(
            cells.gate_for_live(&wrong_seed),
            Err(CellError::SeamMismatch("seed"))
        );
    }

    #[test]
    fn dual_gate_refuses_unreachable_island_with_named_check() {
        let cost = cell_cost_bytes(16).expect("16 valid");
        let budget = CellBudget::new(4 * cost, 4, u64::MAX).expect("budget");
        let mut cells = Cells::new(12345, budget);
        let hub = CellCoord::new(0, 0).expect("hub");
        cells.stream(hub, 16, 9).expect("hub fits");
        cells.publish(u64::MAX);
        let east_coord = CellCoord::new(1, 0).expect("east");
        let honest =
            parse_cell(&bake_cell(12345, east_coord, 16).expect("east bakes")).expect("parses");
        assert!(
            cell_endpoints_connect(&honest),
            "honest cell walks end to end"
        );
        assert_eq!(cells.gate_for_live(&honest), Ok(()));
        let hub_data = parse_cell(&bake_cell(12345, hub, 16).expect("hub bakes")).expect("parses");
        let mut walled = honest.clone();
        for x in 0..16usize {
            walled.holes[8 * 16 + x] = true;
        }
        assert_eq!(
            seam_joins(&hub_data, &walled),
            Ok(()),
            "holes carry no seam, heights still join"
        );
        assert!(
            !cell_endpoints_connect(&walled),
            "middle wall cuts the walk"
        );
        assert_eq!(
            cells.gate_for_live(&walled),
            Err(CellError::Unreachable),
            "island names its check"
        );
    }

    #[test]
    fn dual_gate_nine_cell_walk_holds_budgets() {
        let cost = cell_cost_bytes(16).expect("16 valid");
        let budget = CellBudget::new(9 * cost, 9, u64::MAX).expect("budget");
        let mut cells = Cells::new(12345, budget);
        for z in -1..=1 {
            for x in -1..=1 {
                let coord = CellCoord::new(x, z).expect("block coord");
                cells.stream(coord, 16, 1).expect("block fits");
            }
        }
        while cells.pending_count() > 0 {
            cells.publish(u64::MAX);
        }
        assert_eq!(cells.live_count(), 9, "all nine pass the dual gate");
        assert_eq!(cells.refused_count(), 0, "honest block never refuses");
        assert_eq!(cells.used_bytes(), 9 * cost);
        assert_eq!(cells.high_water_bytes(), 9 * cost);
        assert_eq!(cells.remaining_bytes(), 0);
        assert_eq!(cells.remaining_jobs(), 0);
        let from = CellCoord::new(-1, -1).expect("corner");
        let to = CellCoord::new(1, 1).expect("corner");
        let steps = cells
            .route(from, (0, 0), to, (15, 15))
            .expect("live block routes");
        assert!(steps > 16, "route crosses cells, got {steps} steps");
    }
}
