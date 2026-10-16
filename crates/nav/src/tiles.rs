//! tiles: versioned dynamic tiles plus flow fields over the grid.
//! Owner: nav crate; the arena breaks walls here and steers legions here.
//! Units: cells in, tile ids and versions out, ticks at 60 Hz.
//! Determinism: sorted break sets, ascending rebake order, integer BFS
//! with fixed neighbor order; no hash order, floats, or wall clock.
//! Errors: typed TileError; bad sets and bad wire refuse, never half-apply.
//! Budgets: steering is O(agents) table reads; a dirty tile rebakes under
//! 50 ms p95 and 500 agents step under 1 ms on the release gate.
//! Bake patterns reimplemented from the tile-cache school:
//! PORT(recastnavigation/recastnavigation@main:DetourTileCache)
//! PORT(janhohenheim/rerecast@main:crates) with EVIDENCE rows filed.

use std::collections::VecDeque;
use std::fmt;

use crate::cursor::{Cursor, Truncated};
use crate::flow::{bake_flow, follow};

/// Largest tile registry; bounds memory up front.
pub const MAX_TILES: usize = 65_536;
/// Largest break set; bounds one invalidation up front.
pub const MAX_BREAK_TILES: usize = 1024;
/// Largest grid cell count; denser grids refuse loudly.
pub const MAX_GRID_CELLS: usize = 1_048_576;
/// Largest cached flow-field count; older goals evict FIFO.
pub const MAX_FLOW_FIELDS: usize = 8;
/// Fixed tick rate a breach must open inside of (one second of ticks).
pub const TICKS_PER_SECOND: u32 = 60;
/// Largest queued obstacle request count; further requests refuse loudly.
pub const MAX_OBSTACLE_REQUESTS: usize = 64;
/// Largest tile set one box obstacle may touch; wider boxes refuse loudly.
pub const MAX_TOUCHED_TILES: usize = 8;
/// Wire magic (`E2TIL1`) at the head of every blob.
const WIRE_MAGIC: [u8; 6] = *b"E2TIL1";
/// Wire version; bump on any layout change, refuse anything else.
const WIRE_VERSION: u16 = 1;
/// Largest blob one decode accepts; larger blobs refuse loudly.
const MAX_WIRE_BYTES: usize = 16 << 20;
/// Wire magic (`E2PTH1`) at the head of every stamped-path blob.
const PATH_MAGIC: [u8; 6] = *b"E2PTH1";
/// Stamped-path wire version; bump on any layout change, refuse anything else.
const PATH_VERSION: u16 = 1;

/// Stable nav tile id; dense `0..tile_count` in a registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileId(pub u32);

/// One break batch naming the tiles it dirties, in any order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakSet {
    /// Match tick of the break at 60 Hz.
    pub tick: u32,
    /// Affected tile ids; sorted and deduped on build.
    pub tiles: Vec<TileId>,
}

impl BreakSet {
    /// Builds a break set from raw tile ids in any order.
    ///
    /// Sorts and dedups so shuffled reports invalidate identically.
    /// Errors: [`TileError::EmptySet`], [`TileError::TooManyTiles`],
    /// [`TileError::BadTile`].
    pub fn new(tick: u32, tiles: &[TileId], tile_count: usize) -> Result<Self, TileError> {
        if tiles.is_empty() {
            return Err(TileError::EmptySet);
        }
        if tiles.len() > MAX_BREAK_TILES {
            return Err(TileError::TooManyTiles {
                got: tiles.len(),
                max: MAX_BREAK_TILES,
            });
        }
        let mut sorted = tiles.to_vec();
        sorted.sort();
        for tile in &sorted {
            if tile.0 as usize >= tile_count {
                return Err(TileError::BadTile { tile: tile.0 });
            }
        }
        sorted.dedup();
        Ok(Self {
            tick,
            tiles: sorted,
        })
    }

    /// Maps freed chunk ids to tiles through a caller lane.
    ///
    /// The arena owns the chunk-to-tile map; nav only sorts the result.
    /// Chunks with no tile map to nothing instead of failing the batch.
    /// Errors: [`TileError::EmptySet`], [`TileError::TooManyTiles`],
    /// [`TileError::BadTile`].
    pub fn from_chunks(
        tick: u32,
        chunks: &[u32],
        tile_count: usize,
        mut tile_of: impl FnMut(u32) -> Option<TileId>,
    ) -> Result<Self, TileError> {
        if chunks.is_empty() {
            return Err(TileError::EmptySet);
        }
        if chunks.len() > MAX_BREAK_TILES {
            return Err(TileError::TooManyTiles {
                got: chunks.len(),
                max: MAX_BREAK_TILES,
            });
        }
        let mut tiles = Vec::new();
        for chunk in chunks {
            if let Some(tile) = tile_of(*chunk) {
                if tile.0 as usize >= tile_count {
                    return Err(TileError::BadTile { tile: tile.0 });
                }
                tiles.push(tile);
            }
        }
        if tiles.is_empty() {
            return Err(TileError::EmptySet);
        }
        tiles.sort();
        tiles.dedup();
        Ok(Self { tick, tiles })
    }
}

/// Salt-versioned obstacle handle; high 16 bits salt, low 16 slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObstacleRef(pub u32);
/// Bounded box-obstacle queue; add/remove enqueue, drain applies touched tiles only.
/// Caller: `TiledNav::update_budget`. PORT(recastnavigation/recastnavigation@9f4ce64:DetourTileCache).
#[derive(Clone, Debug, Default)]
pub struct ObstacleCache {
    slots: Vec<ObstacleSlot>,
    pending: Vec<(bool, usize)>,
}
/// One box slot: clamped bounds, reuse salt, live flag.
#[derive(Clone, Copy, Debug)]
struct ObstacleSlot([usize; 4], u16, bool);
impl ObstacleCache {
    /// Queues a rubble box over cells `b`; cells flip on drain.
    /// Errors: [`TileError::BadBounds`], [`TileError::TooManyTiles`], [`TileError::QueueFull`].
    pub fn add_box_obstacle(
        &mut self,
        nav: &TiledNav,
        b: [usize; 4],
    ) -> Result<ObstacleRef, TileError> {
        let bounds = clamp_box(nav, b)?;
        if self.pending.len() >= MAX_OBSTACLE_REQUESTS {
            return Err(TileError::QueueFull);
        }
        let mut slot = self.slots.len();
        if let Some(i) = self.slots.iter().position(|s| !s.2) {
            slot = i;
        } else {
            if slot >= 0xFFFF {
                return Err(TileError::QueueFull);
            }
            self.slots.push(ObstacleSlot(bounds, 1, false));
        }
        self.slots[slot].0 = bounds;
        self.slots[slot].2 = true;
        self.pending.push((true, slot));
        let salt = u32::from(self.slots[slot].1);
        let low = u32::try_from(slot).map_err(|_| TileError::QueueFull)?;
        Ok(ObstacleRef((salt << 16) | low))
    }
    /// Queues removal of a live box; cells clear on drain.
    /// Errors: [`TileError::UnknownObstacle`], [`TileError::QueueFull`].
    pub fn remove_obstacle(&mut self, obstacle: ObstacleRef) -> Result<(), TileError> {
        let slot = usize::try_from(obstacle.0 & 0xFFFF)
            .map_err(|_| TileError::UnknownObstacle(obstacle.0))?;
        let salt =
            u16::try_from(obstacle.0 >> 16).map_err(|_| TileError::UnknownObstacle(obstacle.0))?;
        let known = matches!(self.slots.get(slot), Some(s) if s.2 && s.1 == salt);
        if !known {
            return Err(TileError::UnknownObstacle(obstacle.0));
        }
        if self.pending.len() >= MAX_OBSTACLE_REQUESTS {
            return Err(TileError::QueueFull);
        }
        self.slots[slot].2 = false;
        self.slots[slot].1 = self.slots[slot].1.wrapping_add(1).max(1);
        self.pending.push((false, slot));
        Ok(())
    }
    /// Applies queued flips in order; rebake happens in `TiledNav::update_budget`.
    pub(crate) fn drain(&mut self, nav: &mut TiledNav) {
        for (add, slot) in std::mem::take(&mut self.pending) {
            let [x0, y0, x1, y1] = self.slots[slot].0;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let _ = nav.set_blocked(x, y, add);
                }
            }
        }
    }
}
/// Clamped box bounds; inversion, off-grid, or touching over 8 tiles refuses.
fn clamp_box(nav: &TiledNav, b: [usize; 4]) -> Result<[usize; 4], TileError> {
    if b[0] > b[2] || b[1] > b[3] || b[0] >= nav.width() || b[1] >= nav.height() {
        return Err(TileError::BadBounds);
    }
    let x1 = b[2].min(nav.width() - 1);
    let y1 = b[3].min(nav.height() - 1);
    let c = [b[0], b[1], x1, y1];
    let (_cols, _) = nav.tile_dims();
    let edge = nav.edge();
    let n = (c[2] / edge - c[0] / edge + 1) * (c[3] / edge - c[1] / edge + 1);
    if n > MAX_TOUCHED_TILES {
        return Err(TileError::TooManyTiles {
            got: n,
            max: MAX_TOUCHED_TILES,
        });
    }
    Ok(c)
}

/// Typed tile failures; bad sets refuse, never half-apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileError {
    /// No tiles were named.
    EmptySet,
    /// More tiles than one invalidation accepts.
    TooManyTiles {
        /// Tile count offered.
        got: usize,
        /// Invalidation bound refused past.
        max: usize,
    },
    /// Tile id past the registry.
    BadTile {
        /// Tile id offered.
        tile: u32,
    },
    /// Tile count is zero or over the registry bound.
    BadTileCount {
        /// Tile count offered.
        got: usize,
    },
    /// Version counter would overflow; the tile keeps its version.
    VersionOverflow {
        /// Tile id refused.
        tile: u32,
    },
    /// Grid dims are zero, over the cell bound, or overflow.
    BadDims {
        /// Grid width offered in cells.
        w: usize,
        /// Grid height offered in cells.
        h: usize,
    },
    /// Grid bytes do not match the dims.
    BadGridLen {
        /// Byte count offered.
        got: usize,
        /// Cell count wanted.
        want: usize,
    },
    /// Tile edge is zero or covers more tiles than the bound.
    BadEdge {
        /// Tile edge offered in cells.
        got: usize,
    },
    /// Goal is outside the grid or on a blocked cell.
    BadGoal {
        /// Goal x cell offered.
        x: usize,
        /// Goal y cell offered.
        y: usize,
    },
    /// Goal has no baked flow field yet.
    NoField,
    /// Cell is outside the grid.
    OutOfBounds,
    /// Agent and output lanes differ in length.
    MismatchedLanes {
        /// Agent count offered.
        agents: usize,
        /// Output slot count offered.
        slots: usize,
    },
    /// Blob magic is not `E2TIL1`.
    BadMagic,
    /// Blob version is not the wire version.
    VersionMismatch {
        /// Version found.
        got: u16,
    },
    /// Blob ends mid-record.
    Truncated,
    /// Blob has bytes past the last flow record.
    TrailingBytes {
        /// Leftover byte count.
        got: usize,
    },
    /// Blob breaks canonical order or bounds.
    NotCanonical(&'static str),
    /// Blob is larger than the decode bound.
    TooLarge {
        /// Byte count offered.
        got: usize,
    },
    /// Obstacle request queue is full.
    QueueFull,
    /// Stale or unknown obstacle handle value.
    UnknownObstacle(u32),
    /// Box bounds are inverted or off the grid.
    BadBounds,
}

impl fmt::Display for TileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySet => write!(f, "tiles: no tiles named"),
            Self::TooManyTiles { got, max } => {
                write!(f, "tiles: {got} tiles over bound {max}")
            }
            Self::BadTile { tile } => write!(f, "tiles: tile {tile} past the registry"),
            Self::BadTileCount { got } => write!(f, "tiles: tile count {got} refused"),
            Self::VersionOverflow { tile } => {
                write!(f, "tiles: tile {tile} version overflow")
            }
            Self::BadDims { w, h } => write!(f, "tiles: grid {w}x{h} refused"),
            Self::BadGridLen { got, want } => {
                write!(f, "tiles: grid bytes {got} want {want}")
            }
            Self::BadEdge { got } => write!(f, "tiles: tile edge {got} refused"),
            Self::BadGoal { x, y } => write!(f, "tiles: goal ({x}, {y}) refused"),
            Self::NoField => write!(f, "tiles: goal has no baked field"),
            Self::OutOfBounds => write!(f, "tiles: cell outside the grid"),
            Self::MismatchedLanes { agents, slots } => {
                write!(f, "tiles: {agents} agents with {slots} slots")
            }
            Self::BadMagic => write!(f, "tiles: bad magic"),
            Self::VersionMismatch { got } => write!(f, "tiles: version {got} refused"),
            Self::Truncated => write!(f, "tiles: blob truncated"),
            Self::TrailingBytes { got } => write!(f, "tiles: {got} trailing bytes"),
            Self::NotCanonical(why) => write!(f, "tiles: blob not canonical ({why})"),
            Self::TooLarge { got } => write!(f, "tiles: {got} bytes over decode bound"),
            Self::QueueFull => write!(f, "tiles: obstacle queue full"),
            Self::UnknownObstacle(o) => write!(f, "tiles: obstacle {o} unknown"),
            Self::BadBounds => write!(f, "tiles: box bounds refused"),
        }
    }
}

impl std::error::Error for TileError {}

/// Tile id for a registry index. A registry holds at most [`MAX_TILES`] tiles,
/// so the index always fits; a wider one saturates to `u32::MAX`, which no
/// registry accepts, so every checked accessor refuses it as `BadTile`.
fn tile_id_at(index: usize) -> TileId {
    TileId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// Per-tile versions plus dirty flags; walkers finish then repath.
///
/// Versions start at zero and bump once per rebaked invalidation.
/// `invalidate` only marks dirty; `rebake` bumps and clears, so a
/// walker reading its tile version repaths exactly when the version
/// it walks on goes stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileRegistry {
    versions: Vec<u32>,
    dirty: Vec<bool>,
}

impl TileRegistry {
    /// Empty registry over `tile_count` tiles (`1..=MAX_TILES`).
    /// Errors: [`TileError::BadTileCount`].
    pub fn new(tile_count: usize) -> Result<Self, TileError> {
        if tile_count == 0 || tile_count > MAX_TILES {
            return Err(TileError::BadTileCount { got: tile_count });
        }
        Ok(Self {
            versions: vec![0; tile_count],
            dirty: vec![false; tile_count],
        })
    }

    /// Tile count.
    pub fn tile_count(&self) -> usize {
        self.versions.len()
    }

    /// Version of one tile. Errors: [`TileError::BadTile`].
    pub fn version(&self, tile: TileId) -> Result<u32, TileError> {
        self.check(tile)?;
        Ok(self.versions[tile.0 as usize])
    }

    /// True when the tile waits for a rebake. Errors: [`TileError::BadTile`].
    pub fn is_dirty(&self, tile: TileId) -> Result<bool, TileError> {
        self.check(tile)?;
        Ok(self.dirty[tile.0 as usize])
    }

    /// Dirty tiles in ascending id order (allocates; tick code reads
    /// [`TileRegistry::is_dirty`] and [`TileRegistry::dirty_count`] instead).
    pub fn dirty_tiles(&self) -> Vec<TileId> {
        let mut out = Vec::new();
        for (index, dirty) in self.dirty.iter().enumerate() {
            if *dirty {
                out.push(tile_id_at(index));
            }
        }
        out
    }

    /// Dirty tile count without allocating.
    pub fn dirty_count(&self) -> usize {
        self.dirty.iter().filter(|dirty| **dirty).count()
    }

    /// Marks one tile dirty; true when newly dirtied.
    /// Errors: [`TileError::BadTile`].
    pub fn set_dirty(&mut self, tile: TileId) -> Result<bool, TileError> {
        self.check(tile)?;
        let slot = &mut self.dirty[tile.0 as usize];
        let fresh = !*slot;
        *slot = true;
        Ok(fresh)
    }

    /// Marks a break set dirty; versions wait for `rebake`.
    ///
    /// Returns the affected tiles in ascending order. Idempotent:
    /// invalidating the same set twice dirties once. Validates every
    /// tile first, so a bad set changes nothing.
    /// Errors: [`TileError::BadTile`].
    pub fn invalidate(&mut self, set: &BreakSet) -> Result<Vec<TileId>, TileError> {
        for tile in &set.tiles {
            self.check(*tile)?;
        }
        for tile in &set.tiles {
            self.dirty[tile.0 as usize] = true;
        }
        Ok(set.tiles.clone())
    }

    /// Rebakes one tile: bumps its version and clears the flag.
    ///
    /// Rebaking a clean tile still bumps (explicit recooks stay
    /// versioned). Errors: [`TileError::BadTile`],
    /// [`TileError::VersionOverflow`] (the flag stays set then).
    pub fn rebake(&mut self, tile: TileId) -> Result<u32, TileError> {
        self.check(tile)?;
        let slot = &mut self.versions[tile.0 as usize];
        *slot = slot
            .checked_add(1)
            .ok_or(TileError::VersionOverflow { tile: tile.0 })?;
        self.dirty[tile.0 as usize] = false;
        Ok(*slot)
    }

    /// True when the walking version went stale and must repath.
    /// Errors: [`TileError::BadTile`].
    pub fn needs_repath(&self, tile: TileId, walking_version: u32) -> Result<bool, TileError> {
        Ok(self.version(tile)? != walking_version)
    }

    /// Version table in tile order (wire order).
    fn versions(&self) -> &[u32] {
        &self.versions
    }

    /// Refuses tiles past the registry.
    fn check(&self, tile: TileId) -> Result<(), TileError> {
        if tile.0 as usize >= self.versions.len() {
            return Err(TileError::BadTile { tile: tile.0 });
        }
        Ok(())
    }
}

/// Version-stamped walk: a baked cell walk plus the tile-version
/// vector it was stamped against.
///
/// The tick dirties tiles on breaks ([`TiledNav::apply_breaks`]) and bumps
/// versions on rebake ([`TiledNav::rebake_budget`]); walkers hold a
/// `StampedPath` and repath exactly when [`StampedPath::is_stale`] flips,
/// so fighters never chase a dead polygon and fresh walkers never repath.
/// Stamps cover every tile the cells cross, sorted ascending, so identical
/// queries stamp identical bytes.
/// Budgets: stamping is one BFS at bake time (O(cells), allocates the walk
/// plus its stamps); [`StampedPath::is_stale`] is O(stamps) version reads
/// without allocating; [`TiledNav::stale_paths`] allocates the index lane
/// only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampedPath {
    w: usize,
    h: usize,
    edge: usize,
    start: (usize, usize),
    goal: (usize, usize),
    cells: Vec<(usize, usize)>,
    stamps: Vec<(TileId, u32)>,
}

impl StampedPath {
    /// Grid width in cells the walk was stamped against.
    pub fn width(&self) -> usize {
        self.w
    }

    /// Grid height in cells the walk was stamped against.
    pub fn height(&self) -> usize {
        self.h
    }

    /// Tile side in cells the walk was stamped against.
    pub fn edge(&self) -> usize {
        self.edge
    }

    /// Walk start cell.
    pub fn start(&self) -> (usize, usize) {
        self.start
    }

    /// Walk goal cell.
    pub fn goal(&self) -> (usize, usize) {
        self.goal
    }

    /// Walk cells from start to goal, one 4-dir step apart.
    pub fn cells(&self) -> &[(usize, usize)] {
        &self.cells
    }

    /// Tile-version vector in ascending tile order (wire order).
    pub fn stamps(&self) -> &[(TileId, u32)] {
        &self.stamps
    }

    /// Stamped version of one tile crossed by this walk, if crossed.
    /// Binary search over the sorted stamps; never allocates.
    pub fn stamp_of(&self, tile: TileId) -> Option<u32> {
        self.stamps
            .binary_search_by_key(&tile, |(tile, _)| *tile)
            .ok()
            .map(|index| self.stamps[index].1)
    }

    /// True when a rebake moved under the walk and it must repath.
    ///
    /// Compares every stamped version against the nav; dirty flags alone
    /// never flip this, only the rebake bump does. Never allocates.
    /// Errors: [`TileError::BadTile`], [`TileError::NotCanonical`]
    /// (the walk was stamped against a different grid shape).
    pub fn is_stale(&self, nav: &TiledNav) -> Result<bool, TileError> {
        self.check_shape(nav)?;
        for (tile, version) in &self.stamps {
            if nav.version(*tile)? != *version {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Exact stale subset: tiles the walk crosses whose version advanced,
    /// in ascending order. Bake-time shape; allocates the lane only.
    /// Errors: [`TileError::BadTile`], [`TileError::NotCanonical`]
    /// (the walk was stamped against a different grid shape).
    pub fn stale_tiles(&self, nav: &TiledNav) -> Result<Vec<TileId>, TileError> {
        self.check_shape(nav)?;
        let mut out = Vec::new();
        for (tile, version) in &self.stamps {
            if nav.version(*tile)? != *version {
                out.push(*tile);
            }
        }
        Ok(out)
    }

    /// Canonical stamped bytes; identical queries stamp identical bytes.
    /// Allocates the blob.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&PATH_MAGIC);
        out.extend_from_slice(&PATH_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.w as u32).to_le_bytes());
        out.extend_from_slice(&(self.h as u32).to_le_bytes());
        out.extend_from_slice(&(self.edge as u32).to_le_bytes());
        out.extend_from_slice(&(self.start.0 as u32).to_le_bytes());
        out.extend_from_slice(&(self.start.1 as u32).to_le_bytes());
        out.extend_from_slice(&(self.goal.0 as u32).to_le_bytes());
        out.extend_from_slice(&(self.goal.1 as u32).to_le_bytes());
        out.extend_from_slice(&(self.cells.len() as u32).to_le_bytes());
        for cell in &self.cells {
            out.extend_from_slice(&(cell.0 as u32).to_le_bytes());
            out.extend_from_slice(&(cell.1 as u32).to_le_bytes());
        }
        out.extend_from_slice(&(self.stamps.len() as u32).to_le_bytes());
        for (tile, version) in &self.stamps {
            out.extend_from_slice(&tile.0.to_le_bytes());
            out.extend_from_slice(&version.to_le_bytes());
        }
        out
    }

    /// Decodes canonical bytes; any non-stamp blob refuses with its reason.
    /// The stamp set must equal the tile coverage of the cells, so a walk
    /// can neither hide a crossed tile nor claim one it never enters.
    /// Errors: [`TileError::TooLarge`], [`TileError::BadMagic`],
    /// [`TileError::VersionMismatch`], [`TileError::Truncated`],
    /// [`TileError::TrailingBytes`], [`TileError::NotCanonical`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TileError> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(TileError::TooLarge { got: bytes.len() });
        }
        let mut read = Cursor::new(bytes);
        let mut magic = [0u8; 6];
        for slot in magic.iter_mut() {
            *slot = read.byte()?;
        }
        if magic != PATH_MAGIC {
            return Err(TileError::BadMagic);
        }
        let version = read.u16()?;
        if version != PATH_VERSION {
            return Err(TileError::VersionMismatch { got: version });
        }
        let (w, h) = (read.u32()? as usize, read.u32()? as usize);
        let edge = read.u32()? as usize;
        let cells_bound = w
            .checked_mul(h)
            .ok_or(TileError::NotCanonical("path dims"))?;
        if cells_bound == 0 || cells_bound > MAX_GRID_CELLS {
            return Err(TileError::NotCanonical("path dims"));
        }
        if edge == 0 {
            return Err(TileError::NotCanonical("path edge"));
        }
        let cols = w.div_ceil(edge);
        let rows = h.div_ceil(edge);
        let tile_count = cols
            .checked_mul(rows)
            .ok_or(TileError::NotCanonical("path tiles"))?;
        if tile_count == 0 || tile_count > MAX_TILES {
            return Err(TileError::NotCanonical("path tiles"));
        }
        let start = (read.u32()? as usize, read.u32()? as usize);
        let goal = (read.u32()? as usize, read.u32()? as usize);
        if start.0 >= w || start.1 >= h || goal.0 >= w || goal.1 >= h {
            return Err(TileError::NotCanonical("path endpoints"));
        }
        let cells = read_walk(&mut read, (w, h), cells_bound, start, goal)?;
        let stamp_count = read.u32()? as usize;
        if stamp_count == 0 || stamp_count > tile_count {
            return Err(TileError::NotCanonical("path stamps"));
        }
        let mut stamps = Vec::with_capacity(stamp_count);
        for _ in 0..stamp_count {
            let tile = TileId(read.u32()?);
            let version = read.u32()?;
            if tile.0 as usize >= tile_count {
                return Err(TileError::NotCanonical("path stamps"));
            }
            if stamps.last().is_some_and(|(prev, _)| *prev >= tile) {
                return Err(TileError::NotCanonical("path stamps"));
            }
            stamps.push((tile, version));
        }
        // Cells are in-grid and tile_count <= MAX_TILES, so every index fits a
        // u32; a wider one means the blob cannot be canonical.
        let mut covered: Vec<TileId> = cells
            .iter()
            .map(|(x, y)| {
                u32::try_from(y / edge * cols + x / edge)
                    .map(TileId)
                    .map_err(|_| TileError::NotCanonical("path stamps"))
            })
            .collect::<Result<_, _>>()?;
        covered.sort();
        covered.dedup();
        if covered.len() != stamps.len()
            || covered
                .iter()
                .zip(stamps.iter())
                .any(|(tile, (stamped, _))| tile != stamped)
        {
            return Err(TileError::NotCanonical("path stamps"));
        }
        if read.remaining() != 0 {
            return Err(TileError::TrailingBytes {
                got: read.remaining(),
            });
        }
        Ok(Self {
            w,
            h,
            edge,
            start,
            goal,
            cells,
            stamps,
        })
    }

    /// Refuses walks stamped against a different grid shape.
    fn check_shape(&self, nav: &TiledNav) -> Result<(), TileError> {
        if self.w != nav.w || self.h != nav.h || self.edge != nav.edge {
            return Err(TileError::NotCanonical("path shape"));
        }
        Ok(())
    }
}

/// Decode the cell walk of a stamped path: at most `cells_bound` in-grid
/// cells that begin at `start`, end at `goal` and advance one 4-neighbor step
/// at a time, so a blob cannot claim a walk the BFS bake would never emit.
fn read_walk(
    read: &mut Cursor<'_>,
    (w, h): (usize, usize),
    cells_bound: usize,
    start: (usize, usize),
    goal: (usize, usize),
) -> Result<Vec<(usize, usize)>, TileError> {
    let cell_count = read.u32()? as usize;
    if cell_count == 0 || cell_count > cells_bound {
        return Err(TileError::NotCanonical("path cells"));
    }
    let mut cells = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        let cell = (read.u32()? as usize, read.u32()? as usize);
        if cell.0 >= w || cell.1 >= h {
            return Err(TileError::NotCanonical("path cells"));
        }
        cells.push(cell);
    }
    if cells[0] != start || cells[cells.len() - 1] != goal {
        return Err(TileError::NotCanonical("path cells"));
    }
    for pair in cells.windows(2) {
        let step = pair[0].0.abs_diff(pair[1].0) + pair[0].1.abs_diff(pair[1].1);
        if step != 1 {
            return Err(TileError::NotCanonical("path cells"));
        }
    }
    Ok(cells)
}

/// Dynamic tiled nav: occupancy grid plus versioned tiles plus one flow
/// field per baked goal that steers whole legions.
///
/// The arena flips cells when walls break ([`TiledNav::set_blocked`] for
/// single breaches, [`TiledNav::apply_breaks`] for break batches), rebakes
/// dirty tiles on the tick ([`TiledNav::rebake_budget`]), then refreshes
/// goal fields ([`TiledNav::bake_goal`]). Runtime steering
/// ([`TiledNav::direction`], [`TiledNav::steer_legion`]) only reads baked
/// fields and never allocates; bake-time calls (`apply_breaks`,
/// `dirty_tiles`, `to_bytes`) may allocate and say so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TiledNav {
    w: usize,
    h: usize,
    edge: usize,
    cols: usize,
    rows: usize,
    blocked: Vec<bool>,
    registry: TileRegistry,
    goals: Vec<(usize, usize)>,
    fields: Vec<Vec<i32>>,
    stale: bool,
}

impl TiledNav {
    /// Bakes a fresh grid; every tile starts clean at version zero with no
    /// flow fields (bake goals after). `edge` is the tile side in cells.
    /// Errors: [`TileError::BadDims`], [`TileError::BadGridLen`],
    /// [`TileError::BadEdge`].
    pub fn bake(blocked: &[bool], w: usize, h: usize, edge: usize) -> Result<Self, TileError> {
        let cells = w.checked_mul(h).ok_or(TileError::BadDims { w, h })?;
        if cells == 0 || cells > MAX_GRID_CELLS {
            return Err(TileError::BadDims { w, h });
        }
        if blocked.len() != cells {
            return Err(TileError::BadGridLen {
                got: blocked.len(),
                want: cells,
            });
        }
        if edge == 0 {
            return Err(TileError::BadEdge { got: edge });
        }
        let cols = w.div_ceil(edge);
        let rows = h.div_ceil(edge);
        let tiles = cols
            .checked_mul(rows)
            .ok_or(TileError::BadEdge { got: edge })?;
        if tiles > MAX_TILES {
            return Err(TileError::BadEdge { got: edge });
        }
        Ok(Self {
            w,
            h,
            edge,
            cols,
            rows,
            blocked: blocked.to_vec(),
            registry: TileRegistry::new(tiles)?,
            goals: Vec::new(),
            fields: Vec::new(),
            stale: false,
        })
    }

    /// Bakes a deterministic arena grid from `seed` + size.
    ///
    /// Walls come from [`crate::flow::seeded_blocked`]; dims and edge
    /// validate exactly like [`TiledNav::bake`]. Same seed, size, density,
    /// and edge bake identical bytes (`to_bytes` x2); neighbor seeds diverge.
    /// Bake-time: allocates the grid plus the nav. Runtime steering stays on
    /// [`TiledNav::direction`] (one fighter brain) and
    /// [`TiledNav::steer_legion`] (whole FPS legions).
    /// Errors: [`TileError::BadDims`], [`TileError::BadGridLen`] (never: the
    /// seeded lane sizes exactly), [`TileError::BadEdge`].
    pub fn bake_seeded(
        seed: u64,
        w: usize,
        h: usize,
        edge: usize,
        density_permille: u32,
    ) -> Result<Self, TileError> {
        let blocked = crate::flow::seeded_blocked(seed, w, h, density_permille);
        Self::bake(&blocked, w, h, edge)
    }

    /// Grid width in cells.
    pub fn width(&self) -> usize {
        self.w
    }

    /// Grid height in cells.
    pub fn height(&self) -> usize {
        self.h
    }

    /// Tile side in cells.
    pub fn edge(&self) -> usize {
        self.edge
    }

    /// Tile grid dims `(columns, rows)`.
    pub fn tile_dims(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Tile count.
    pub fn tile_count(&self) -> usize {
        self.registry.tile_count()
    }

    /// True when any cached flow field is older than the grid.
    pub fn flows_stale(&self) -> bool {
        self.stale
    }

    /// Tile holding a cell. Errors: [`TileError::OutOfBounds`], and
    /// [`TileError::BadTile`] only if an index ever exceeds `u32` (a baked
    /// grid stays under [`MAX_TILES`], so that never happens).
    pub fn tile_of(&self, x: usize, y: usize) -> Result<TileId, TileError> {
        if x >= self.w || y >= self.h {
            return Err(TileError::OutOfBounds);
        }
        let index = y / self.edge * self.cols + x / self.edge;
        // A wider index refuses as a tile past the registry; it never wraps.
        let tile = u32::try_from(index).map_err(|_| TileError::BadTile { tile: u32::MAX })?;
        Ok(TileId(tile))
    }

    /// True when a cell is blocked. Errors: [`TileError::OutOfBounds`].
    pub fn is_blocked(&self, x: usize, y: usize) -> Result<bool, TileError> {
        if x >= self.w || y >= self.h {
            return Err(TileError::OutOfBounds);
        }
        Ok(self.blocked[y * self.w + x])
    }

    /// Version of one tile. Errors: [`TileError::BadTile`].
    pub fn version(&self, tile: TileId) -> Result<u32, TileError> {
        self.registry.version(tile)
    }

    /// True when the tile waits for a rebake. Errors: [`TileError::BadTile`].
    pub fn is_dirty(&self, tile: TileId) -> Result<bool, TileError> {
        self.registry.is_dirty(tile)
    }

    /// Dirty tiles in ascending order (allocates; see [`TiledNav::dirty_count`]).
    pub fn dirty_tiles(&self) -> Vec<TileId> {
        self.registry.dirty_tiles()
    }

    /// Dirty tile count without allocating.
    pub fn dirty_count(&self) -> usize {
        self.registry.dirty_count()
    }

    /// True when the walking version went stale and must repath.
    /// Errors: [`TileError::BadTile`].
    pub fn needs_repath(&self, tile: TileId, walking_version: u32) -> Result<bool, TileError> {
        self.registry.needs_repath(tile, walking_version)
    }

    /// Bakes a version-stamped cell walk from `start` to `goal`.
    ///
    /// BFS shortest walk in fixed E,W,S,N order, stamped with the current
    /// version of every tile the walk crosses, so a later rebake marks
    /// exactly the walks it moved under. `Ok(None)` when sealed; stale
    /// copies repath through [`TiledNav::refresh_path`]. Bake-time:
    /// allocates the walk plus its stamps.
    /// Errors: [`TileError::OutOfBounds`] (an endpoint past the grid),
    /// [`TileError::BadGoal`] (an endpoint on a blocked cell, naming it).
    pub fn find_path(
        &self,
        start: (usize, usize),
        goal: (usize, usize),
    ) -> Result<Option<StampedPath>, TileError> {
        if start.0 >= self.w || start.1 >= self.h {
            return Err(TileError::OutOfBounds);
        }
        if goal.0 >= self.w || goal.1 >= self.h {
            return Err(TileError::OutOfBounds);
        }
        if self.blocked[goal.1 * self.w + goal.0] {
            return Err(TileError::BadGoal {
                x: goal.0,
                y: goal.1,
            });
        }
        if self.blocked[start.1 * self.w + start.0] {
            return Err(TileError::BadGoal {
                x: start.0,
                y: start.1,
            });
        }
        let s = start.1 * self.w + start.0;
        let g = goal.1 * self.w + goal.0;
        if s == g {
            return Ok(Some(self.stamp_walk(start, goal, vec![start])?));
        }
        let mut parent: Vec<Option<usize>> = vec![None; self.w * self.h];
        let mut queue = VecDeque::new();
        parent[s] = Some(s);
        queue.push_back(s);
        while let Some(cell) = queue.pop_front() {
            if cell == g {
                break;
            }
            let (cx, cy) = (cell % self.w, cell / self.w);
            for (dx, dy) in [(1i32, 0), (-1, 0), (0, 1), (0, -1)] {
                let nx = cx as i32 + dx;
                let ny = cy as i32 + dy;
                if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 {
                    continue;
                }
                let next = ny as usize * self.w + nx as usize;
                if parent[next].is_none() && !self.blocked[next] {
                    parent[next] = Some(cell);
                    queue.push_back(next);
                }
            }
        }
        if parent[g].is_none() {
            return Ok(None);
        }
        let mut rev = Vec::new();
        let mut cell = g;
        loop {
            rev.push((cell % self.w, cell / self.w));
            if cell == s {
                break;
            }
            cell = parent[cell].expect("BFS parents chain back to the start");
        }
        rev.reverse();
        Ok(Some(self.stamp_walk(start, goal, rev)?))
    }

    /// True when `path` was stamped before the latest rebake of a tile it
    /// crosses and must repath. Tick read path: O(stamps) version reads,
    /// never allocates.
    /// Errors: [`TileError::BadTile`], [`TileError::NotCanonical`]
    /// (the walk was stamped against a different grid shape).
    pub fn path_is_stale(&self, path: &StampedPath) -> Result<bool, TileError> {
        path.is_stale(self)
    }

    /// Exact stale set over a held walk list: indices that must repath, in
    /// input order, so one rebake repaths exactly its stale walks.
    /// Foreign-shaped or unreadable walks count stale, so their walkers
    /// requery instead of chasing dead polygons. Allocates the index lane
    /// only.
    pub fn stale_paths(&self, paths: &[StampedPath]) -> Vec<usize> {
        let mut out = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            if path.is_stale(self).unwrap_or(true) {
                out.push(index);
            }
        }
        out
    }

    /// Repaths exactly when stale: `Ok(None)` when the held walk is fresh,
    /// a fresh [`StampedPath`] (or `Ok(None)` when newly sealed) when a
    /// rebake moved under it. Bake-time: allocates only on repath.
    /// Errors: [`TileError::OutOfBounds`], [`TileError::BadGoal`] (the grid
    /// moved under the endpoints), [`TileError::NotCanonical`] (the walk was
    /// stamped against a different grid shape).
    pub fn refresh_path(&self, path: &StampedPath) -> Result<Option<StampedPath>, TileError> {
        if !path.is_stale(self)? {
            return Ok(None);
        }
        self.find_path(path.start, path.goal)
    }

    /// Flips one cell (wall built or breached) and dirties its tile.
    ///
    /// Idempotent: rewriting the same value keeps the tile clean and the
    /// flows fresh. Returns the holding tile.
    /// Errors: [`TileError::OutOfBounds`].
    pub fn set_blocked(&mut self, x: usize, y: usize, blocked: bool) -> Result<TileId, TileError> {
        let tile = self.tile_of(x, y)?;
        if self.blocked[y * self.w + x] != blocked {
            self.blocked[y * self.w + x] = blocked;
            self.registry.set_dirty(tile)?;
            self.stale = true;
        }
        Ok(tile)
    }

    /// Applies one break batch: invalidates exactly its tiles (ascending)
    /// and marks cached flows stale. Returns the affected tiles.
    /// Errors: [`TileError::BadTile`]; a bad set changes nothing.
    pub fn apply_breaks(&mut self, set: &BreakSet) -> Result<Vec<TileId>, TileError> {
        let affected = self.registry.invalidate(set)?;
        if !affected.is_empty() {
            self.stale = true;
        }
        Ok(affected)
    }

    /// Rebakes the lowest dirty tile, if any, without allocating.
    ///
    /// Overflowed tiles keep their dirty flag and are skipped, so one
    /// stuck counter never blocks the rest of the row.
    pub fn rebake_one(&mut self) -> Option<TileId> {
        for index in 0..self.registry.tile_count() {
            let tile = tile_id_at(index);
            if !matches!(self.registry.is_dirty(tile), Ok(true)) {
                continue;
            }
            if self.registry.rebake(tile).is_ok() {
                self.stale = true;
                return Some(tile);
            }
        }
        None
    }

    /// Rebakes up to `max_tiles` dirty tiles in ascending order without
    /// allocating; returns how many rebaked. The tick calls this with its
    /// per-tick tile budget, so a breach opens within its tick budget.
    pub fn rebake_budget(&mut self, max_tiles: usize) -> usize {
        let mut done = 0;
        while done < max_tiles {
            if self.rebake_one().is_none() {
                break;
            }
            done += 1;
        }
        done
    }

    /// Applies queued obstacle flips, then rebakes up to `max_tiles` dirty
    /// tiles in ascending order without allocating; returns how many
    /// rebaked. Units: tile counts. Determinism: flips apply in queue
    /// order, rebakes ascend, so the same queue plus budget gives the same
    /// versions. The tick calls this with its per-tick tile budget.
    /// PORT(recastnavigation/recastnavigation@9f4ce64:DetourTileCache `update()`).
    pub fn update_budget(&mut self, cache: &mut ObstacleCache, max_tiles: usize) -> usize {
        cache.drain(self);
        self.rebake_budget(max_tiles)
    }

    /// Bakes (or refreshes) the flow field for `goal`.
    ///
    /// New goals evict the oldest past [`MAX_FLOW_FIELDS`]; when the grid
    /// changed since the last bake every cached field rebuilds, so all
    /// readers steer on the same generation. Allocates at most one field
    /// per stale generation.
    /// Errors: [`TileError::BadGoal`].
    pub fn bake_goal(&mut self, goal: (usize, usize)) -> Result<(), TileError> {
        let (gx, gy) = goal;
        if gx >= self.w || gy >= self.h || self.blocked[gy * self.w + gx] {
            return Err(TileError::BadGoal { x: gx, y: gy });
        }
        let known = self.goals.contains(&goal);
        if !known {
            if self.goals.len() >= MAX_FLOW_FIELDS {
                self.goals.remove(0);
                self.fields.remove(0);
            }
            self.goals.push(goal);
            self.fields.push(Vec::new());
        }
        if self.stale || !known {
            self.refresh_flows();
        }
        Ok(())
    }

    /// One downhill step toward a baked goal; `None` at the goal or when
    /// trapped. Runtime only: never bakes, never allocates.
    /// Errors: [`TileError::NoField`], [`TileError::OutOfBounds`].
    pub fn direction(
        &self,
        goal: (usize, usize),
        pos: (usize, usize),
    ) -> Result<Option<(usize, usize)>, TileError> {
        let field = self.field_of(goal)?;
        if pos.0 >= self.w || pos.1 >= self.h {
            return Err(TileError::OutOfBounds);
        }
        Ok(follow(field, self.w, self.h, pos))
    }

    /// Steps a whole legion one cell each toward a baked goal.
    ///
    /// Writers go to the caller `out` lane (same length as `agents`);
    /// movers advance downhill, holders keep their cell. Returns how many
    /// moved. Input order decides output order, so the batch replays
    /// exactly. Never allocates.
    /// Errors: [`TileError::NoField`], [`TileError::OutOfBounds`],
    /// [`TileError::MismatchedLanes`]; a bad batch writes nothing.
    pub fn steer_legion(
        &self,
        goal: (usize, usize),
        agents: &[(usize, usize)],
        out: &mut [(usize, usize)],
    ) -> Result<usize, TileError> {
        if agents.len() != out.len() {
            return Err(TileError::MismatchedLanes {
                agents: agents.len(),
                slots: out.len(),
            });
        }
        let field = self.field_of(goal)?;
        for pos in agents.iter() {
            if pos.0 >= self.w || pos.1 >= self.h {
                return Err(TileError::OutOfBounds);
            }
        }
        let mut moved = 0usize;
        for (index, pos) in agents.iter().enumerate() {
            match follow(field, self.w, self.h, *pos) {
                Some(next) => {
                    out[index] = next;
                    moved += 1;
                }
                None => {
                    out[index] = *pos;
                }
            }
        }
        Ok(moved)
    }

    /// True when the baked field reaches `pos` from `goal` (distance set).
    /// Runtime only: never bakes, never allocates. Callers refresh with
    /// [`TiledNav::bake_goal`] after rebakes first.
    /// Errors: [`TileError::NoField`], [`TileError::OutOfBounds`].
    pub fn path_open(&self, goal: (usize, usize), pos: (usize, usize)) -> Result<bool, TileError> {
        let field = self.field_of(goal)?;
        if pos.0 >= self.w || pos.1 >= self.h {
            return Err(TileError::OutOfBounds);
        }
        Ok(field[pos.1 * self.w + pos.0] >= 0)
    }

    /// Canonical versioned bytes; identical grids bake identical bytes.
    /// Allocates the blob.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&WIRE_MAGIC);
        out.extend_from_slice(&WIRE_VERSION.to_le_bytes());
        out.extend_from_slice(&(self.w as u32).to_le_bytes());
        out.extend_from_slice(&(self.h as u32).to_le_bytes());
        out.extend_from_slice(&(self.edge as u32).to_le_bytes());
        out.extend_from_slice(&(self.registry.tile_count() as u32).to_le_bytes());
        out.extend_from_slice(&u32::from(self.stale).to_le_bytes());
        out.extend(self.blocked.iter().map(|blocked| u8::from(*blocked)));
        for version in self.registry.versions() {
            out.extend_from_slice(&version.to_le_bytes());
        }
        for dirty in self.registry.dirty.iter() {
            out.push(u8::from(*dirty));
        }
        out.extend_from_slice(&(self.goals.len() as u32).to_le_bytes());
        for (goal, field) in self.goals.iter().zip(self.fields.iter()) {
            out.extend_from_slice(&(goal.0 as u32).to_le_bytes());
            out.extend_from_slice(&(goal.1 as u32).to_le_bytes());
            for dist in field {
                out.extend_from_slice(&dist.to_le_bytes());
            }
        }
        out
    }

    /// Decodes canonical bytes; any non-bake blob refuses with its reason.
    /// Errors: [`TileError::TooLarge`], [`TileError::BadMagic`],
    /// [`TileError::VersionMismatch`], [`TileError::Truncated`],
    /// [`TileError::TrailingBytes`], [`TileError::NotCanonical`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TileError> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(TileError::TooLarge { got: bytes.len() });
        }
        let mut read = Cursor::new(bytes);
        let mut magic = [0u8; 6];
        for slot in magic.iter_mut() {
            *slot = read.byte()?;
        }
        if magic != WIRE_MAGIC {
            return Err(TileError::BadMagic);
        }
        let version = read.u16()?;
        if version != WIRE_VERSION {
            return Err(TileError::VersionMismatch { got: version });
        }
        let (w, h) = (read.u32()? as usize, read.u32()? as usize);
        let edge = read.u32()? as usize;
        let tile_count = read.u32()? as usize;
        let cells = w
            .checked_mul(h)
            .ok_or(TileError::NotCanonical("grid dims"))?;
        if cells == 0 || cells > MAX_GRID_CELLS {
            return Err(TileError::NotCanonical("grid dims"));
        }
        if edge == 0 {
            return Err(TileError::NotCanonical("tile edge"));
        }
        if tile_count == 0
            || tile_count > MAX_TILES
            || tile_count != w.div_ceil(edge) * h.div_ceil(edge)
        {
            return Err(TileError::NotCanonical("tile count"));
        }
        let stale = match read.u32()? {
            0 => false,
            1 => true,
            _ => return Err(TileError::NotCanonical("stale flag")),
        };
        let blocked_raw = read.take(cells)?;
        let mut blocked = Vec::with_capacity(cells);
        for byte in blocked_raw {
            match byte {
                0 => blocked.push(false),
                1 => blocked.push(true),
                _ => return Err(TileError::NotCanonical("blocked bits")),
            }
        }
        let mut versions = Vec::with_capacity(tile_count);
        for _ in 0..tile_count {
            versions.push(read.u32()?);
        }
        let dirty_raw = read.take(tile_count)?;
        let mut dirty = Vec::with_capacity(tile_count);
        for byte in dirty_raw {
            match byte {
                0 => dirty.push(false),
                1 => dirty.push(true),
                _ => return Err(TileError::NotCanonical("dirty bits")),
            }
        }
        let goal_count = read.u32()? as usize;
        if goal_count > MAX_FLOW_FIELDS {
            return Err(TileError::NotCanonical("flow count"));
        }
        let mut goals = Vec::with_capacity(goal_count);
        let mut fields = Vec::with_capacity(goal_count);
        for _ in 0..goal_count {
            let (gx, gy) = (read.u32()? as usize, read.u32()? as usize);
            if gx >= w || gy >= h || blocked[gy * w + gx] {
                return Err(TileError::NotCanonical("goal cell"));
            }
            let mut field = Vec::with_capacity(cells);
            for _ in 0..cells {
                field.push(read.i32()?);
            }
            if field[gy * w + gx] != 0 {
                return Err(TileError::NotCanonical("goal dist"));
            }
            goals.push((gx, gy));
            fields.push(field);
        }
        if read.remaining() != 0 {
            return Err(TileError::TrailingBytes {
                got: read.remaining(),
            });
        }
        Ok(Self {
            w,
            h,
            edge,
            cols: w.div_ceil(edge),
            rows: h.div_ceil(edge),
            blocked,
            registry: TileRegistry { versions, dirty },
            goals,
            fields,
            stale,
        })
    }

    /// Stable `u64` over the canonical bytes (bake hash gate).
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }

    /// Validator: dims plus tile coverage plus flow shapes.
    /// Empty vec is green (allocates the report only).
    pub fn validate(&self) -> Vec<String> {
        let mut errs = Vec::new();
        let cells = self.w.saturating_mul(self.h);
        if self.w == 0 || self.h == 0 || cells > MAX_GRID_CELLS {
            errs.push("tiles: grid dims out of bound".to_string());
        }
        if self.edge == 0 {
            errs.push("tiles: zero tile edge".to_string());
        }
        if self.blocked.len() != cells {
            errs.push("tiles: blocked len != w*h".to_string());
        }
        if self.edge != 0
            && self.registry.tile_count() != self.w.div_ceil(self.edge) * self.h.div_ceil(self.edge)
        {
            errs.push("tiles: registry != tile coverage".to_string());
        }
        if self.goals.len() != self.fields.len() || self.goals.len() > MAX_FLOW_FIELDS {
            errs.push("tiles: flow cache shape refused".to_string());
        }
        for (goal, field) in self.goals.iter().zip(self.fields.iter()) {
            if field.len() != cells {
                errs.push("tiles: flow len != w*h".to_string());
                break;
            }
            if goal.0 >= self.w || goal.1 >= self.h {
                errs.push("tiles: goal out of bounds".to_string());
                break;
            }
            if self.blocked.len() == cells {
                if self.blocked[goal.1 * self.w + goal.0] {
                    errs.push("tiles: goal on blocked cell".to_string());
                    break;
                }
                if field[goal.1 * self.w + goal.0] != 0 {
                    errs.push("tiles: goal dist != 0".to_string());
                    break;
                }
                let mut bad = false;
                for (index, blocked) in self.blocked.iter().enumerate() {
                    if *blocked && field[index] != -1 {
                        bad = true;
                        break;
                    }
                }
                if bad {
                    errs.push("tiles: blocked cell dist != -1".to_string());
                    break;
                }
            }
        }
        errs
    }

    /// Cached field for a baked goal, if held.
    fn field_of(&self, goal: (usize, usize)) -> Result<&[i32], TileError> {
        self.goals
            .iter()
            .position(|held| *held == goal)
            .map(|index| &*self.fields[index])
            .ok_or(TileError::NoField)
    }

    /// Stamps a baked walk with the current version of every tile it
    /// crosses, sorted ascending so identical queries stamp identical
    /// bytes. Bake-time: allocates the stamp lane only.
    fn stamp_walk(
        &self,
        start: (usize, usize),
        goal: (usize, usize),
        cells: Vec<(usize, usize)>,
    ) -> Result<StampedPath, TileError> {
        let mut tiles = Vec::new();
        for cell in &cells {
            tiles.push(self.tile_of(cell.0, cell.1)?);
        }
        tiles.sort();
        tiles.dedup();
        let mut stamps = Vec::with_capacity(tiles.len());
        for tile in tiles {
            stamps.push((tile, self.registry.version(tile)?));
        }
        Ok(StampedPath {
            w: self.w,
            h: self.h,
            edge: self.edge,
            start,
            goal,
            cells,
            stamps,
        })
    }

    /// Rebuilds every cached field from the current grid in goal order.
    /// BFS per field is deterministic, so insertion order never leaks
    /// into the bytes.
    fn refresh_flows(&mut self) {
        for index in 0..self.goals.len() {
            let goal = self.goals[index];
            self.fields[index] = bake_flow(&self.blocked, self.w, self.h, goal);
        }
        self.stale = false;
    }
}

impl From<Truncated> for TileError {
    fn from(_: Truncated) -> Self {
        Self::Truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(w: usize, h: usize, edge: usize) -> TiledNav {
        TiledNav::bake(&vec![false; w * h], w, h, edge).expect("open grid bakes")
    }

    /// Wall column at `x` with one gap cell left open at `gap_y`.
    fn walled(w: usize, h: usize, wall_x: usize, gap_y: Option<usize>) -> Vec<bool> {
        let mut blocked = vec![false; w * h];
        for y in 0..h {
            if Some(y) != gap_y {
                blocked[y * w + wall_x] = true;
            }
        }
        blocked
    }

    #[test]
    fn invalidate_marks_dirty_without_bumping() {
        let mut reg = TileRegistry::new(16).expect("registry builds");
        let set = BreakSet::new(9, &[TileId(3), TileId(1), TileId(3)], 16).expect("set builds");
        assert_eq!(set.tiles, vec![TileId(1), TileId(3)], "sorted and deduped");
        let affected = reg.invalidate(&set).expect("invalidates");
        assert_eq!(affected, vec![TileId(1), TileId(3)]);
        assert!(reg.is_dirty(TileId(1)).expect("reads"));
        assert!(reg.is_dirty(TileId(3)).expect("reads"));
        assert!(!reg.is_dirty(TileId(2)).expect("reads"));
        assert_eq!(reg.version(TileId(1)), Ok(0), "versions wait for rebake");
        assert_eq!(reg.invalidate(&set).expect("invalidates"), affected);
        assert_eq!(reg.dirty_tiles(), vec![TileId(1), TileId(3)]);
        assert_eq!(reg.dirty_count(), 2);
    }

    #[test]
    fn rebake_bumps_only_its_tile_and_clears() {
        let mut reg = TileRegistry::new(16).expect("registry builds");
        let set = BreakSet::new(9, &[TileId(1), TileId(3)], 16).expect("set builds");
        reg.invalidate(&set).expect("invalidates");
        assert_eq!(reg.rebake(TileId(1)), Ok(1));
        assert!(!reg.is_dirty(TileId(1)).expect("reads"));
        assert!(reg.is_dirty(TileId(3)).expect("reads"));
        assert_eq!(reg.version(TileId(1)), Ok(1));
        assert_eq!(
            reg.version(TileId(3)),
            Ok(0),
            "untouched tiles keep versions"
        );
        assert_eq!(reg.rebake(TileId(1)), Ok(2), "second rebake bumps again");
    }

    #[test]
    fn walkers_repath_only_when_their_tile_advances() {
        let mut reg = TileRegistry::new(16).expect("registry builds");
        let set = BreakSet::new(4, &[TileId(2)], 16).expect("set builds");
        reg.invalidate(&set).expect("invalidates");
        assert_eq!(
            reg.needs_repath(TileId(2), 0),
            Ok(false),
            "dirty alone never repaths"
        );
        reg.rebake(TileId(2)).expect("rebakes");
        assert_eq!(reg.needs_repath(TileId(2), 0), Ok(true), "bump repaths");
        assert_eq!(
            reg.needs_repath(TileId(2), 1),
            Ok(false),
            "fresh walkers hold"
        );
    }

    #[test]
    fn shuffled_break_sets_invalidate_identically() {
        let mut first = TileRegistry::new(16).expect("registry builds");
        let mut second = TileRegistry::new(16).expect("registry builds");
        let a = BreakSet::new(7, &[TileId(5), TileId(2), TileId(8)], 16).expect("set builds");
        let b = BreakSet::new(7, &[TileId(8), TileId(5), TileId(2)], 16).expect("set builds");
        assert_eq!(a, b);
        assert_eq!(
            first.invalidate(&a).expect("invalidates"),
            second.invalidate(&b).expect("invalidates")
        );
        assert_eq!(first.dirty_tiles(), second.dirty_tiles());
    }

    #[test]
    fn chunks_map_to_sorted_tiles() {
        let set = BreakSet::from_chunks(3, &[9, 4, 9, 2], 16, |chunk| Some(TileId(chunk % 8)))
            .expect("maps");
        assert_eq!(set.tiles, vec![TileId(1), TileId(2), TileId(4)]);
        assert_eq!(set.tick, 3);
        let sparse = BreakSet::from_chunks(3, &[1, 2], 16, |chunk| {
            if chunk == 1 {
                None
            } else {
                Some(TileId(6))
            }
        })
        .expect("maps");
        assert_eq!(sparse.tiles, vec![TileId(6)]);
    }

    #[test]
    fn bad_sets_refuse_without_mutation() {
        let mut reg = TileRegistry::new(16).expect("registry builds");
        assert_eq!(BreakSet::new(1, &[], 16), Err(TileError::EmptySet));
        assert_eq!(
            BreakSet::from_chunks(1, &[], 16, |c| Some(TileId(c))),
            Err(TileError::EmptySet)
        );
        assert_eq!(
            TileRegistry::new(0),
            Err(TileError::BadTileCount { got: 0 })
        );
        assert_eq!(
            TileRegistry::new(MAX_TILES + 1),
            Err(TileError::BadTileCount { got: MAX_TILES + 1 })
        );
        assert_eq!(
            BreakSet::new(1, &[TileId(99)], 16),
            Err(TileError::BadTile { tile: 99 })
        );
        assert_eq!(
            reg.version(TileId(99)),
            Err(TileError::BadTile { tile: 99 })
        );
        assert_eq!(reg.rebake(TileId(99)), Err(TileError::BadTile { tile: 99 }));
        let many = vec![TileId(1); MAX_BREAK_TILES + 1];
        assert_eq!(
            BreakSet::new(1, &many, 16),
            Err(TileError::TooManyTiles {
                got: MAX_BREAK_TILES + 1,
                max: MAX_BREAK_TILES
            })
        );
        assert!(reg.dirty_tiles().is_empty(), "refusals change nothing");
        assert_eq!(reg.version(TileId(1)), Ok(0));
    }

    #[test]
    fn tile_ids_stay_exact_at_the_registry_bound() {
        // 256 x 256 cells at edge 1 is exactly MAX_TILES tiles: the last id
        // still fits a u32 and every id stays inside the registry.
        let nav =
            TiledNav::bake(&vec![false; 256 * 256], 256, 256, 1).expect("bound-sized grid bakes");
        assert_eq!(nav.tile_count(), MAX_TILES);
        assert_eq!(nav.tile_of(0, 0), Ok(TileId(0)));
        assert_eq!(nav.tile_of(255, 255), Ok(TileId(65_535)));
        assert_eq!(nav.tile_of(256, 0), Err(TileError::OutOfBounds));
        // One tile past the registry bound refuses at bake time.
        assert_eq!(
            TiledNav::bake(&vec![false; 257 * 256], 257, 256, 1).map(|nav| nav.tile_count()),
            Err(TileError::BadEdge { got: 1 })
        );
    }

    #[test]
    fn dirty_tiles_keep_exact_ids_at_the_bound() {
        let mut reg = TileRegistry::new(MAX_TILES).expect("bound-sized registry");
        reg.set_dirty(TileId(65_535))
            .expect("last tile is in range");
        assert_eq!(reg.dirty_tiles(), vec![TileId(65_535)]);
        // A wider index saturates to an id no registry accepts.
        assert_eq!(tile_id_at(usize::MAX), TileId(u32::MAX));
        assert_eq!(
            reg.version(TileId(u32::MAX)),
            Err(TileError::BadTile { tile: u32::MAX })
        );
    }

    #[test]
    fn rebake_one_keeps_exact_ids_at_the_bound() {
        let mut nav =
            TiledNav::bake(&vec![false; 256 * 256], 256, 256, 1).expect("bound-sized grid bakes");
        nav.registry
            .set_dirty(TileId(65_535))
            .expect("last tile is in range");
        assert_eq!(nav.rebake_one(), Some(TileId(65_535)));
        assert_eq!(nav.version(TileId(65_535)), Ok(1));
        assert_eq!(nav.rebake_one(), None, "nothing else is dirty");
    }

    #[test]
    fn bake_refuses_bad_dims_edge_and_len() {
        assert_eq!(
            TiledNav::bake(&[], 0, 8, 4),
            Err(TileError::BadDims { w: 0, h: 8 })
        );
        assert_eq!(
            TiledNav::bake(&[false; 16], 4, 4, 0),
            Err(TileError::BadEdge { got: 0 })
        );
        assert_eq!(
            TiledNav::bake(&[false; 15], 4, 4, 2),
            Err(TileError::BadGridLen { got: 15, want: 16 })
        );
        assert_eq!(
            TiledNav::bake(&[false; 4], 2, 2, 1).map(|nav| nav.tile_count()),
            Ok(4)
        );
    }

    #[test]
    fn breach_opens_a_path_within_one_second_of_ticks() {
        let (w, h) = (20, 12);
        let start = (0usize, 6usize);
        let goal = (19usize, 6usize);
        let mut nav = TiledNav::bake(&walled(w, h, 10, None), w, h, 4).expect("walled grid bakes");
        nav.bake_goal(goal).expect("goal bakes");
        assert_eq!(nav.path_open(goal, start), Ok(false), "sealed wall holds");
        // The breach lands at tick zero; the tick rebakes dirty tiles.
        let gap_tile = nav.set_blocked(10, 6, false).expect("breach flips");
        assert!(nav.is_dirty(gap_tile).expect("reads"));
        let mut ticks = 0u32;
        loop {
            nav.rebake_budget(usize::MAX);
            nav.bake_goal(goal).expect("field refreshes");
            ticks += 1;
            if nav.path_open(goal, start).expect("fresh field answers") {
                break;
            }
            assert!(ticks < TICKS_PER_SECOND, "breach must open in one second");
        }
        assert!(ticks <= TICKS_PER_SECOND);
        assert_eq!(nav.dirty_count(), 0, "rebake drains the row");
        // The downhill walk crosses the wall column only at the breach.
        let mut pos = start;
        let mut visited = vec![pos];
        for _ in 0..w * h {
            if pos == goal {
                break;
            }
            pos = nav
                .direction(goal, pos)
                .expect("fresh field answers")
                .expect("downhill holds to the goal");
            visited.push(pos);
        }
        assert_eq!(pos, goal, "walkers reach the far side");
        assert!(
            visited.contains(&(10usize, 6usize)),
            "the walk crosses at the breach: {visited:?}"
        );
        assert_eq!(nav.version(gap_tile), Ok(1), "one rebake bumps once");
        assert!(
            nav.needs_repath(gap_tile, 0).expect("reads"),
            "stale walkers repath on the bump"
        );
        assert!(
            !nav.needs_repath(gap_tile, 1).expect("reads"),
            "fresh walkers hold"
        );
    }

    #[test]
    fn same_value_flip_keeps_flows_fresh() {
        let mut nav = open(8, 8, 4);
        let goal = (7usize, 7usize);
        nav.bake_goal(goal).expect("goal bakes");
        nav.set_blocked(1, 1, false).expect("same value flips");
        assert!(!nav.flows_stale(), "no change means no work");
        assert_eq!(nav.dirty_count(), 0);
    }

    #[test]
    fn apply_breaks_names_exact_dirty_tiles() {
        let (w, h) = (16, 16);
        let mut nav = open(w, h, 4);
        assert_eq!(nav.tile_dims(), (4, 4));
        // Chunks land on tiles 0, 5 and 10 through the caller map.
        let set = BreakSet::from_chunks(11, &[0, 5, 5, 10], 16, |chunk| Some(TileId(chunk)))
            .expect("maps");
        let affected = nav.apply_breaks(&set).expect("applies");
        assert_eq!(affected, vec![TileId(0), TileId(5), TileId(10)]);
        assert_eq!(nav.dirty_tiles(), affected, "exactly those tiles");
        assert!(nav.flows_stale(), "cached flows wait for refresh");
        assert_eq!(nav.rebake_budget(2), 2, "budget rebakes two");
        assert_eq!(nav.dirty_tiles(), vec![TileId(10)]);
        assert_eq!(nav.version(TileId(0)), Ok(1));
        assert_eq!(nav.version(TileId(10)), Ok(0), "unrebaked tile holds");
        assert_eq!(nav.rebake_budget(usize::MAX), 1, "drain finishes");
        assert_eq!(nav.dirty_count(), 0);
    }

    #[test]
    fn fresh_wall_blocks_a_baked_goal() {
        let (w, h) = (12, 8);
        let goal = (11usize, 4usize);
        let mut nav = open(w, h, 4);
        nav.bake_goal(goal).expect("goal bakes");
        assert_eq!(nav.path_open(goal, (0, 4)), Ok(true));
        for y in 0..h {
            nav.set_blocked(6, y, true).expect("wall rises");
        }
        // Runtime still reads the old field until the refresh lands.
        assert_eq!(nav.path_open(goal, (0, 4)), Ok(true));
        nav.rebake_budget(usize::MAX);
        nav.bake_goal(goal).expect("field refreshes");
        assert_eq!(
            nav.path_open(goal, (0, 4)),
            Ok(false),
            "rebuilt wall seals again"
        );
        assert!(nav.validate().is_empty());
    }

    #[test]
    fn goal_and_cell_failures_refuse() {
        let mut nav = open(8, 8, 4);
        let goal = (7usize, 7usize);
        assert_eq!(
            nav.direction(goal, (0, 0)),
            Err(TileError::NoField),
            "runtime never bakes"
        );
        assert_eq!(nav.path_open(goal, (0, 0)), Err(TileError::NoField));
        assert_eq!(
            nav.bake_goal((8, 8)),
            Err(TileError::BadGoal { x: 8, y: 8 })
        );
        nav.set_blocked(3, 3, true).expect("wall rises");
        assert_eq!(
            nav.bake_goal((3, 3)),
            Err(TileError::BadGoal { x: 3, y: 3 })
        );
        nav.bake_goal(goal).expect("goal bakes");
        assert_eq!(nav.direction(goal, (8, 8)), Err(TileError::OutOfBounds));
        assert_eq!(nav.path_open(goal, (8, 0)), Err(TileError::OutOfBounds));
        assert_eq!(nav.tile_of(8, 0), Err(TileError::OutOfBounds));
        assert_eq!(
            nav.version(TileId(99)),
            Err(TileError::BadTile { tile: 99 })
        );
        let agents = vec![(0usize, 0usize)];
        let mut short: Vec<(usize, usize)> = Vec::new();
        assert_eq!(
            nav.steer_legion(goal, &agents, &mut short),
            Err(TileError::MismatchedLanes {
                agents: 1,
                slots: 0
            })
        );
        let mut out = vec![(0usize, 0usize)];
        assert_eq!(
            nav.steer_legion(goal, &[(8, 8)], &mut out),
            Err(TileError::OutOfBounds),
            "a bad batch writes nothing"
        );
        assert_eq!(out, vec![(0usize, 0usize)], "no half-apply");
    }

    #[test]
    fn legion_batch_matches_single_steps() {
        let (w, h) = (32, 32);
        let mut nav = open(w, h, 8);
        let goal = (31usize, 31usize);
        nav.bake_goal(goal).expect("goal bakes");
        // Deterministic spread of 500 agents over the grid.
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut agents = Vec::with_capacity(500);
        while agents.len() < 500 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let x = (state >> 33) as usize % w;
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let y = (state >> 33) as usize % h;
            if (x, y) == goal {
                continue;
            }
            agents.push((x, y));
        }
        assert_eq!(agents.len(), 500, "spread fills the legion");
        let mut first = vec![(0usize, 0usize); agents.len()];
        let mut second = vec![(0usize, 0usize); agents.len()];
        let moved = nav
            .steer_legion(goal, &agents, &mut first)
            .expect("legion steps");
        let again = nav
            .steer_legion(goal, &agents, &mut second)
            .expect("legion steps");
        assert_eq!(moved, again);
        assert_eq!(first, second, "batches replay exactly");
        assert!(moved > 0, "the open field moves the legion");
        for (index, agent) in agents.iter().enumerate() {
            let single = nav.direction(goal, *agent).expect("field answers");
            assert_eq!(
                Some(first[index]),
                single.map_or(Some(*agent), Some),
                "batch matches single steps"
            );
        }
    }

    #[test]
    fn wire_round_trips_and_shuffled_bytes_match() {
        let (w, h) = (16, 12);
        let goal = (15usize, 11usize);
        let mut first = TiledNav::bake(&walled(w, h, 8, Some(5)), w, h, 4).expect("bakes");
        let mut second = TiledNav::bake(&walled(w, h, 8, Some(5)), w, h, 4).expect("bakes");
        first.bake_goal(goal).expect("goal bakes");
        second.bake_goal(goal).expect("goal bakes");
        // Same breaches in opposite order still bake identical bytes.
        first.set_blocked(8, 4, false).expect("breach flips");
        first.set_blocked(8, 6, false).expect("breach flips");
        second.set_blocked(8, 6, false).expect("breach flips");
        second.set_blocked(8, 4, false).expect("breach flips");
        first.rebake_budget(usize::MAX);
        second.rebake_budget(usize::MAX);
        first.bake_goal(goal).expect("refreshes");
        second.bake_goal(goal).expect("refreshes");
        assert_eq!(first.to_bytes(), second.to_bytes(), "bytes x2");
        assert_eq!(first.digest(), second.digest());
        let back = TiledNav::from_bytes(&first.to_bytes()).expect("canonical decodes");
        assert_eq!(back, first, "round trip holds versions");
        assert_eq!(back.to_bytes(), first.to_bytes());
        assert!(back.validate().is_empty());
    }

    #[test]
    fn wire_refusals_name_their_cause() {
        let mut nav = open(8, 8, 4);
        nav.bake_goal((7, 7)).expect("goal bakes");
        let bytes = nav.to_bytes();
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(TiledNav::from_bytes(&bad_magic), Err(TileError::BadMagic));
        let mut bad_version = bytes.clone();
        bad_version[6] = 0x7f;
        assert!(matches!(
            TiledNav::from_bytes(&bad_version),
            Err(TileError::VersionMismatch { .. })
        ));
        assert_eq!(
            TiledNav::from_bytes(&bytes[..bytes.len() / 2]),
            Err(TileError::Truncated)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            TiledNav::from_bytes(&trailing),
            Err(TileError::TrailingBytes { got: 1 })
        );
        let mut bent = bytes.clone();
        let last = bent.len() - 1;
        bent[last] ^= 0x01;
        assert!(TiledNav::from_bytes(&bent).is_err());
        assert_eq!(
            TiledNav::from_bytes(&vec![0u8; MAX_WIRE_BYTES + 1]),
            Err(TileError::TooLarge {
                got: MAX_WIRE_BYTES + 1
            })
        );
    }

    #[test]
    fn validator_catches_a_blocked_goal_field() {
        let mut nav = open(8, 8, 4);
        nav.bake_goal((7, 7)).expect("goal bakes");
        assert!(nav.validate().is_empty());
        // Corrupt the grid under a baked field: goal cell blocked.
        nav.blocked[7 * 8 + 7] = true;
        assert!(!nav.validate().is_empty());
    }

    #[test]
    fn stamped_path_carries_current_versions_and_round_trips() {
        let (w, h) = (12, 8);
        let nav = open(w, h, 4);
        let (start, goal) = ((0usize, 0usize), (11usize, 7usize));
        let path = nav
            .find_path(start, goal)
            .expect("query answers")
            .expect("open grid connects");
        assert_eq!(path.start(), start);
        assert_eq!(path.goal(), goal);
        let cells = path.cells();
        assert_eq!(cells.first(), Some(&start));
        assert_eq!(cells.last(), Some(&goal));
        for pair in cells.windows(2) {
            let step = pair[0].0.abs_diff(pair[1].0) + pair[0].1.abs_diff(pair[1].1);
            assert_eq!(step, 1, "walks step 4-dir");
        }
        // Stamps cover exactly the crossed tiles at current versions.
        let mut covered: Vec<TileId> = cells
            .iter()
            .map(|(x, y)| nav.tile_of(*x, *y).expect("cells in bounds"))
            .collect();
        covered.sort();
        covered.dedup();
        let stamped: Vec<TileId> = path.stamps().iter().map(|(tile, _)| *tile).collect();
        assert_eq!(stamped, covered);
        for (tile, version) in path.stamps() {
            assert_eq!(nav.version(*tile), Ok(*version));
            assert_eq!(path.stamp_of(*tile), Some(*version));
        }
        assert_eq!(nav.path_is_stale(&path), Ok(false));
        // Identical queries stamp identical bytes with a round trip.
        let twin = open(w, h, 4);
        let twin_path = twin
            .find_path(start, goal)
            .expect("query answers")
            .expect("open grid connects");
        assert_eq!(path.to_bytes(), twin_path.to_bytes(), "bytes x2");
        let back = StampedPath::from_bytes(&path.to_bytes()).expect("canonical decodes");
        assert_eq!(back, path);
        assert_eq!(back.to_bytes(), path.to_bytes());
    }

    #[test]
    fn dirty_bump_repaths_exactly_the_stale_set() {
        let (w, h) = (16, 16);
        let mut nav = open(w, h, 4);
        let north = nav
            .find_path((0, 0), (15, 0))
            .expect("query answers")
            .expect("row connects");
        let south = nav
            .find_path((0, 15), (15, 15))
            .expect("query answers")
            .expect("row connects");
        // Straight rows: shortest walks never leave their row.
        assert_eq!(north.cells().len(), 16);
        assert!(north.cells().iter().all(|(_, y)| *y == 0));
        assert!(south.cells().iter().all(|(_, y)| *y == 15));
        let tile = nav.tile_of(4, 0).expect("cell in bounds");
        assert!(north.stamp_of(tile).is_some(), "north crosses the tile");
        assert_eq!(south.stamp_of(tile), None, "south never enters it");
        let set = BreakSet::new(2, &[tile], 16).expect("set builds");
        let affected = nav.apply_breaks(&set).expect("applies");
        assert_eq!(affected, vec![tile], "exactly the dirty tile");
        assert_eq!(nav.dirty_tiles(), vec![tile]);
        assert_eq!(
            nav.path_is_stale(&north),
            Ok(false),
            "dirty alone never repaths"
        );
        assert_eq!(nav.rebake_budget(usize::MAX), 1);
        assert_eq!(nav.version(tile), Ok(1), "one rebake bumps once");
        assert_eq!(
            nav.path_is_stale(&north),
            Ok(true),
            "the crossed tile advanced"
        );
        assert_eq!(nav.path_is_stale(&south), Ok(false), "fresh walkers hold");
        let held = vec![north.clone(), south.clone()];
        assert_eq!(nav.stale_paths(&held), vec![0], "exactly the stale walk");
        assert_eq!(north.stale_tiles(&nav), Ok(vec![tile]));
        assert_eq!(south.stale_tiles(&nav), Ok(vec![]));
        // Repath refreshes the stamp; the untouched walk keeps its bytes.
        let fresh = nav
            .refresh_path(&north)
            .expect("repaths")
            .expect("still connects");
        assert_eq!(nav.path_is_stale(&fresh), Ok(false));
        assert_eq!(fresh.stamp_of(tile), Some(1));
        assert_eq!(
            nav.refresh_path(&south).expect("reads"),
            None,
            "fresh walkers hold"
        );
        // A twin through the same ops stamps identical bytes.
        let mut twin = open(w, h, 4);
        twin.apply_breaks(&set).expect("applies");
        twin.rebake_budget(usize::MAX);
        assert_eq!(nav.to_bytes(), twin.to_bytes(), "nav bytes x2");
        let twin_fresh = twin
            .refresh_path(&north)
            .expect("repaths")
            .expect("still connects");
        assert_eq!(fresh.to_bytes(), twin_fresh.to_bytes(), "walk bytes x2");
        let back = StampedPath::from_bytes(&fresh.to_bytes()).expect("canonical decodes");
        assert_eq!(back, fresh);
    }

    #[test]
    fn stamped_path_queries_and_wire_refuse() {
        let mut nav = open(8, 8, 4);
        assert_eq!(nav.find_path((8, 0), (0, 0)), Err(TileError::OutOfBounds));
        assert_eq!(nav.find_path((0, 0), (0, 8)), Err(TileError::OutOfBounds));
        nav.set_blocked(7, 7, true).expect("wall rises");
        assert_eq!(
            nav.find_path((0, 0), (7, 7)),
            Err(TileError::BadGoal { x: 7, y: 7 })
        );
        assert_eq!(
            nav.find_path((7, 7), (0, 0)),
            Err(TileError::BadGoal { x: 7, y: 7 })
        );
        // A sealed wall returns no walk and never guesses one.
        let sealed = TiledNav::bake(&walled(8, 8, 4, None), 8, 8, 4).expect("walled grid bakes");
        assert_eq!(
            sealed.find_path((0, 0), (7, 7)).expect("query answers"),
            None
        );
        // A degenerate walk stamps its single tile.
        let same = nav
            .find_path((1, 1), (1, 1))
            .expect("query answers")
            .expect("same cell walks");
        assert_eq!(same.cells(), &[(1usize, 1usize)]);
        assert_eq!(same.stamps().len(), 1);
        assert_eq!(nav.path_is_stale(&same), Ok(false));
        // A foreign shape refuses with its reason and counts stale.
        let big = open(16, 16, 4);
        assert!(matches!(
            same.is_stale(&big),
            Err(TileError::NotCanonical(_))
        ));
        assert_eq!(big.stale_paths(std::slice::from_ref(&same)), vec![0]);
        assert!(matches!(
            big.refresh_path(&same),
            Err(TileError::NotCanonical(_))
        ));
        // Wire refusals name their cause.
        let bytes = same.to_bytes();
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            StampedPath::from_bytes(&bad_magic),
            Err(TileError::BadMagic)
        );
        let mut bad_version = bytes.clone();
        bad_version[6] = 0x7f;
        assert!(matches!(
            StampedPath::from_bytes(&bad_version),
            Err(TileError::VersionMismatch { .. })
        ));
        assert_eq!(
            StampedPath::from_bytes(&bytes[..bytes.len() / 2]),
            Err(TileError::Truncated)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            StampedPath::from_bytes(&trailing),
            Err(TileError::TrailingBytes { got: 1 })
        );
        assert_eq!(
            StampedPath::from_bytes(&vec![0u8; MAX_WIRE_BYTES + 1]),
            Err(TileError::TooLarge {
                got: MAX_WIRE_BYTES + 1
            })
        );
        // A tile past the coverage refuses.
        let mut bent_tile = bytes.clone();
        let tile_at = bent_tile.len() - 8;
        bent_tile[tile_at] = 0xff;
        assert!(matches!(
            StampedPath::from_bytes(&bent_tile),
            Err(TileError::NotCanonical(_))
        ));
        // A stamp count past the coverage refuses before reading.
        let mut bent_count = bytes.clone();
        let count_at = bent_count.len() - 12;
        bent_count[count_at] = 0xff;
        assert!(matches!(
            StampedPath::from_bytes(&bent_count),
            Err(TileError::NotCanonical(_))
        ));
    }

    #[test]
    fn seeded_bake_is_bytes_deterministic_for_fighter_and_legion() {
        // Same seed + size bakes identical bytes x2; a neighbor seed diverges.
        let first = TiledNav::bake_seeded(2040, 16, 16, 4, 150).expect("seeded grid bakes");
        let second = TiledNav::bake_seeded(2040, 16, 16, 4, 150).expect("seeded grid replays");
        assert_eq!(first.to_bytes(), second.to_bytes(), "seed+size -> bytes x2");
        assert_eq!(first.digest(), second.digest());
        let other = TiledNav::bake_seeded(2041, 16, 16, 4, 150).expect("neighbor seed bakes");
        assert_ne!(first.to_bytes(), other.to_bytes(), "adjacent seeds diverge");
        let back = TiledNav::from_bytes(&first.to_bytes()).expect("canonical decodes");
        assert_eq!(back, first, "round trip holds versions");
        assert!(back.validate().is_empty());
        // Gradient follow serves one fighter brain and a whole FPS legion
        // batch from the same baked field.
        let mut fighter =
            TiledNav::bake_seeded(2040, 16, 16, 4, 0).expect("open seeded grid bakes");
        let goal = (15usize, 15usize);
        fighter.bake_goal(goal).expect("goal bakes");
        let mut pos = (0usize, 0usize);
        for _ in 0..64 {
            match fighter.direction(goal, pos).expect("field answers") {
                Some(next) => pos = next,
                None => break,
            }
        }
        assert_eq!(pos, goal, "fighter follow reaches the far corner");
        let agents = vec![(0usize, 0usize); 8];
        let mut first_out = vec![(0usize, 0usize); 8];
        let mut second_out = vec![(0usize, 0usize); 8];
        let moved = fighter
            .steer_legion(goal, &agents, &mut first_out)
            .expect("legion steps");
        let again = fighter
            .steer_legion(goal, &agents, &mut second_out)
            .expect("legion replays");
        assert_eq!((moved, &first_out), (again, &second_out));
        assert!(moved > 0, "the open field moves the legion");
        for (index, agent) in agents.iter().enumerate() {
            let single = fighter.direction(goal, *agent).expect("field answers");
            assert_eq!(
                Some(first_out[index]),
                single.map_or(Some(*agent), Some),
                "batch matches single steps"
            );
        }
    }

    #[test]
    fn seeded_bake_refuses_bad_dims_without_half_apply() {
        assert_eq!(
            TiledNav::bake_seeded(7, 0, 8, 4, 100),
            Err(TileError::BadDims { w: 0, h: 8 })
        );
        assert_eq!(
            TiledNav::bake_seeded(7, 4, 4, 0, 100),
            Err(TileError::BadEdge { got: 0 })
        );
    }
}
