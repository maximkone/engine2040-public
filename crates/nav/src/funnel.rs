//! funnel: portal string-pulling over tiled walks with salted refs.
//! Owner: nav crate; consumer NV8 soak plus SP36 touched-rebake.
//! Units: cells in, integer mm out, ticks at 60 Hz for breach bar.
//! Determinism: sorted stamps, fixed E,W,S,N bake order, integer-only
//! geometry (`+ - * /` plus `isqrt`), no floats, no wall clock.
//! Errors: typed FunnelError; bad endpoints refuse, stale refs refuse.
//! Budgets: query is one BFS plus greedy LOS pulls, p50 at most 0.2 ms;
//! breach opens a path within 60 ticks; bytes identical x2.
//! Shape reimplemented from the Detour school:
//! PORT(recastnavigation/recastnavigation@9f4ce64:DetourNavMeshQuery.cpp).

use crate::tiles::{TileError, TileId, TiledNav, TICKS_PER_SECOND};
use std::fmt;

pub use crate::tiles::TICKS_PER_SECOND as FUNNEL_TICKS_PER_SECOND;

/// Re-exported breach budget so callers read it from the funnel owner.
pub const BREACH_TICKS: u32 = TICKS_PER_SECOND;

/// Millimeters per cell; the only scale in the smoother.
pub const MM_PER_CELL: i64 = 1000;
/// Same scale as u64 for length math without casts.
pub const MM_PER_CELL_U64: u64 = 1000;
/// Largest corridor one query pulls; bounds work up front.
pub const MAX_PORTALS: usize = 4096;
/// Largest waypoint lane one query emits.
pub const MAX_WAYPOINTS: usize = 4096;
/// Wire magic (`E2FUN1`) at the head of every smooth-path blob.
const WIRE_MAGIC: [u8; 6] = *b"E2FUN1";
/// Wire version; bump on any layout change, refuse anything else.
const WIRE_VERSION: u16 = 1;
/// Largest blob one decode accepts.
const MAX_WIRE_BYTES: usize = 16 << 20;

/// Typed funnel failures; bad input refuses, stale refs refuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunnelError {
    /// Endpoint past the grid.
    OutOfBounds,
    /// Endpoint on a blocked cell, naming it.
    BlockedEndpoint { x: usize, y: usize },
    /// Endpoints share no walk.
    Unreachable,
    /// Portal lane broke canonical shape, with reason.
    BadPortal(&'static str),
    /// Corridor over the query bound.
    TooManyPortals { got: usize, max: usize },
    /// Salt moved under a held ref; repath instead of chasing.
    StaleRef { tile: u32 },
    /// Blob magic is not `E2FUN1`.
    BadMagic,
    /// Blob version refused.
    VersionMismatch { got: u16 },
    /// Blob ends mid-record.
    Truncated,
    /// Blob has bytes past the last record.
    TrailingBytes { got: usize },
    /// Blob breaks canonical order or bounds.
    NotCanonical(&'static str),
    /// Blob over the decode bound.
    TooLarge { got: usize },
}

impl fmt::Display for FunnelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds => write!(f, "funnel: cell outside the grid"),
            Self::BlockedEndpoint { x, y } => write!(f, "funnel: endpoint ({x}, {y}) blocked"),
            Self::Unreachable => write!(f, "funnel: no walk between endpoints"),
            Self::BadPortal(why) => write!(f, "funnel: portal refused ({why})"),
            Self::TooManyPortals { got, max } => {
                write!(f, "funnel: {got} portals over bound {max}")
            }
            Self::StaleRef { tile } => write!(f, "funnel: tile {tile} rebaked under the path"),
            Self::BadMagic => write!(f, "funnel: bad magic"),
            Self::VersionMismatch { got } => write!(f, "funnel: version {got} refused"),
            Self::Truncated => write!(f, "funnel: blob truncated"),
            Self::TrailingBytes { got } => write!(f, "funnel: {got} trailing bytes"),
            Self::NotCanonical(why) => write!(f, "funnel: blob not canonical ({why})"),
            Self::TooLarge { got } => write!(f, "funnel: {got} bytes over decode bound"),
        }
    }
}

impl std::error::Error for FunnelError {}

impl From<TileError> for FunnelError {
    fn from(err: TileError) -> Self {
        match err {
            TileError::OutOfBounds => Self::OutOfBounds,
            TileError::BadGoal { x, y } => Self::BlockedEndpoint { x, y },
            TileError::BadTileCount { .. }
            | TileError::BadDims { .. }
            | TileError::BadGridLen { .. }
            | TileError::BadEdge { .. } => Self::BadPortal("grid shape"),
            _ => Self::BadPortal("tile lane"),
        }
    }
}

/// Salted tile handle: tile plus the version it was stamped against.
///
/// The salt is the registry version at corridor build. A rebake bumps the
/// registry, so the held salt mismatches and the ref refuses instead of
/// resolving into a recycled tile. Identical queries stamp identical u64s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileRef {
    tile: TileId,
    salt: u32,
}

impl TileRef {
    /// Stamps a tile with its current registry version as salt.
    pub fn stamp(nav: &TiledNav, tile: TileId) -> Result<Self, FunnelError> {
        Ok(Self {
            tile,
            salt: nav.version(tile).map_err(|_| FunnelError::OutOfBounds)?,
        })
    }

    /// Held tile id.
    pub fn tile(self) -> TileId {
        self.tile
    }

    /// Held salt (registry version at stamp time).
    pub fn salt(self) -> u32 {
        self.salt
    }

    /// Packs tile plus salt into one u64, high half salt, low half tile.
    pub fn encode_u64(self) -> u64 {
        (u64::from(self.salt) << 32) | u64::from(self.tile.0)
    }

    /// Unpacks one u64; tile past `tile_count` refuses loudly.
    pub fn decode_u64(raw: u64, tile_count: usize) -> Result<Self, FunnelError> {
        let tile = TileId(
            u32::try_from(raw & 0xffff_ffff).map_err(|_| FunnelError::NotCanonical("ref tile"))?,
        );
        if usize::try_from(tile.0).map_err(|_| FunnelError::NotCanonical("ref tile"))? >= tile_count
        {
            return Err(FunnelError::NotCanonical("ref tile"));
        }
        Ok(Self {
            tile,
            salt: u32::try_from(raw >> 32).map_err(|_| FunnelError::NotCanonical("ref salt"))?,
        })
    }

    /// True when a rebake moved under this ref and it must repath.
    pub fn is_stale(self, nav: &TiledNav) -> Result<bool, FunnelError> {
        Ok(nav
            .version(self.tile)
            .map_err(|_| FunnelError::OutOfBounds)?
            != self.salt)
    }

    /// Resolves a fresh ref to its tile; stale refs refuse, never guess.
    pub fn resolve(self, nav: &TiledNav) -> Result<TileId, FunnelError> {
        if self.is_stale(nav)? {
            return Err(FunnelError::StaleRef { tile: self.tile.0 });
        }
        Ok(self.tile)
    }
}

/// One portal of the corridor: the shared edge crossed plus the salted
/// ref of the tile entered, in integer mm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Portal {
    left_mm: [i32; 2],
    right_mm: [i32; 2],
    through: TileRef,
}

impl Portal {
    /// Held left corner in mm.
    pub fn left_mm(self) -> [i32; 2] {
        self.left_mm
    }

    /// Held right corner in mm.
    pub fn right_mm(self) -> [i32; 2] {
        self.right_mm
    }

    /// Salted ref of the tile entered through this portal.
    pub fn through(self) -> TileRef {
        self.through
    }
}

/// Portal corridor: start plus goal in mm with one portal per crossing.
///
/// Built from a grid walk in walk order; portals carry the salted ref of
/// each tile entered so a later rebake marks exactly the corridors it
/// moved under. Bake-time: allocates the portal lane only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corridor {
    start_mm: [i32; 2],
    goal_mm: [i32; 2],
    portals: Vec<Portal>,
}

impl Corridor {
    /// Corridor start in mm.
    pub fn start_mm(&self) -> [i32; 2] {
        self.start_mm
    }

    /// Corridor goal in mm.
    pub fn goal_mm(&self) -> [i32; 2] {
        self.goal_mm
    }

    /// Portals in walk order.
    pub fn portals(&self) -> &[Portal] {
        &self.portals
    }
}

/// Smoothed path: string-pulled waypoints in mm plus the salted refs of
/// every tile crossed, with both lengths for the never-longer proof.
///
/// `grid_len_mm` is the BFS walk in mm; `smooth_len_mm` is the pulled
/// Euclidean length floored per segment, so smoothing never reads longer.
/// Stamps cover every tile the pulled walk touches, sorted ascending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmoothPath {
    start: (usize, usize),
    goal: (usize, usize),
    waypoints_mm: Vec<[i32; 2]>,
    refs: Vec<TileRef>,
    grid_len_mm: u64,
    smooth_len_mm: u64,
}

impl SmoothPath {
    /// Walk start cell.
    pub fn start(&self) -> (usize, usize) {
        self.start
    }

    /// Walk goal cell.
    pub fn goal(&self) -> (usize, usize) {
        self.goal
    }

    /// Pulled waypoints in mm, inclusive of both ends.
    pub fn waypoints_mm(&self) -> &[[i32; 2]] {
        &self.waypoints_mm
    }

    /// Salted tile refs in ascending tile order.
    pub fn refs(&self) -> &[TileRef] {
        &self.refs
    }

    /// BFS grid length in mm.
    pub fn grid_len_mm(&self) -> u64 {
        self.grid_len_mm
    }

    /// Pulled length in mm, floored per segment.
    pub fn smooth_len_mm(&self) -> u64 {
        self.smooth_len_mm
    }

    /// True when a rebake moved under the walk and it must repath.
    pub fn is_stale(&self, nav: &TiledNav) -> Result<bool, FunnelError> {
        for held in &self.refs {
            if held.is_stale(nav)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Canonical bytes; identical queries stamp identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&WIRE_MAGIC);
        out.extend_from_slice(&WIRE_VERSION.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.start.0)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(
            &u32::try_from(self.start.1)
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        out.extend_from_slice(&u32::try_from(self.goal.0).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.goal.1).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.waypoints_mm.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for point in &self.waypoints_mm {
            out.extend_from_slice(&point[0].to_le_bytes());
            out.extend_from_slice(&point[1].to_le_bytes());
        }
        out.extend_from_slice(
            &u32::try_from(self.refs.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for held in &self.refs {
            out.extend_from_slice(&held.encode_u64().to_le_bytes());
        }
        out.extend_from_slice(&self.grid_len_mm.to_le_bytes());
        out.extend_from_slice(&self.smooth_len_mm.to_le_bytes());
        out
    }

    /// Decodes canonical bytes; any non-funnel blob refuses with reason.
    pub fn from_bytes(bytes: &[u8], tile_count: usize) -> Result<Self, FunnelError> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(FunnelError::TooLarge { got: bytes.len() });
        }
        let mut at = 0usize;
        let take = |at: &mut usize, len: usize| -> Result<&[u8], FunnelError> {
            if bytes.len() - *at < len {
                return Err(FunnelError::Truncated);
            }
            let slice = &bytes[*at..*at + len];
            *at += len;
            Ok(slice)
        };
        if take(&mut at, 6)? != WIRE_MAGIC {
            return Err(FunnelError::BadMagic);
        }
        let version = u16::from_le_bytes(
            take(&mut at, 2)?
                .try_into()
                .map_err(|_| FunnelError::Truncated)?,
        );
        if version != WIRE_VERSION {
            return Err(FunnelError::VersionMismatch { got: version });
        }
        let u32le = |at: &mut usize| -> Result<u32, FunnelError> {
            Ok(u32::from_le_bytes(
                take(at, 4)?
                    .try_into()
                    .map_err(|_| FunnelError::Truncated)?,
            ))
        };
        let to_cell = |v: u32| -> Result<usize, FunnelError> {
            usize::try_from(v).map_err(|_| FunnelError::NotCanonical("cell"))
        };
        let start = (to_cell(u32le(&mut at)?)?, to_cell(u32le(&mut at)?)?);
        let goal = (to_cell(u32le(&mut at)?)?, to_cell(u32le(&mut at)?)?);
        let points = to_cell(u32le(&mut at)?)?;
        if points == 0 || points > MAX_WAYPOINTS {
            return Err(FunnelError::NotCanonical("waypoints"));
        }
        let mut waypoints_mm = Vec::with_capacity(points);
        for _ in 0..points {
            let x = i32::from_le_bytes(
                take(&mut at, 4)?
                    .try_into()
                    .map_err(|_| FunnelError::Truncated)?,
            );
            let y = i32::from_le_bytes(
                take(&mut at, 4)?
                    .try_into()
                    .map_err(|_| FunnelError::Truncated)?,
            );
            waypoints_mm.push([x, y]);
        }
        let ref_count = to_cell(u32le(&mut at)?)?;
        if ref_count == 0 || ref_count > MAX_PORTALS {
            return Err(FunnelError::NotCanonical("refs"));
        }
        let mut refs = Vec::with_capacity(ref_count);
        for _ in 0..ref_count {
            let raw = u64::from_le_bytes(
                take(&mut at, 8)?
                    .try_into()
                    .map_err(|_| FunnelError::Truncated)?,
            );
            refs.push(TileRef::decode_u64(raw, tile_count)?);
        }
        for pair in refs.windows(2) {
            if pair[0].tile >= pair[1].tile {
                return Err(FunnelError::NotCanonical("ref order"));
            }
        }
        let u64le = |at: &mut usize| -> Result<u64, FunnelError> {
            Ok(u64::from_le_bytes(
                take(at, 8)?
                    .try_into()
                    .map_err(|_| FunnelError::Truncated)?,
            ))
        };
        let grid_len_mm = u64le(&mut at)?;
        let smooth_len_mm = u64le(&mut at)?;
        if smooth_len_mm > grid_len_mm {
            return Err(FunnelError::NotCanonical("lengths"));
        }
        if at != bytes.len() {
            return Err(FunnelError::TrailingBytes {
                got: bytes.len() - at,
            });
        }
        Ok(Self {
            start,
            goal,
            waypoints_mm,
            refs,
            grid_len_mm,
            smooth_len_mm,
        })
    }

    /// Stable u64 over the canonical bytes.
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
}

/// Center of a cell in mm; errors only on overflow, never wraps.
fn cell_center_mm(cell: (usize, usize)) -> Result<[i32; 2], FunnelError> {
    let x = i64::try_from(cell.0).map_err(|_| FunnelError::BadPortal("mm range"))? * MM_PER_CELL
        + MM_PER_CELL / 2;
    let y = i64::try_from(cell.1).map_err(|_| FunnelError::BadPortal("mm range"))? * MM_PER_CELL
        + MM_PER_CELL / 2;
    Ok([
        i32::try_from(x).map_err(|_| FunnelError::BadPortal("mm range"))?,
        i32::try_from(y).map_err(|_| FunnelError::BadPortal("mm range"))?,
    ])
}

/// Shared edge of two orthogonal neighbor cells in mm, ordered so the
/// portal reads left-to-right along the walk.
fn shared_edge(a: (usize, usize), b: (usize, usize)) -> Result<([i32; 2], [i32; 2]), FunnelError> {
    let (ax, ay) = (
        i64::try_from(a.0).map_err(|_| FunnelError::BadPortal("mm range"))? * MM_PER_CELL,
        i64::try_from(a.1).map_err(|_| FunnelError::BadPortal("mm range"))? * MM_PER_CELL,
    );
    let ans: ([i64; 2], [i64; 2]) = if b.0 == a.0 + 1 && b.1 == a.1 {
        ([ax + MM_PER_CELL, ay], [ax + MM_PER_CELL, ay + MM_PER_CELL])
    } else if a.0 == b.0 + 1 && b.1 == a.1 {
        ([ax, ay + MM_PER_CELL], [ax, ay])
    } else if b.1 == a.1 + 1 && b.0 == a.0 {
        ([ax + MM_PER_CELL, ay + MM_PER_CELL], [ax, ay + MM_PER_CELL])
    } else if a.1 == b.1 + 1 && b.0 == a.0 {
        ([ax, ay], [ax + MM_PER_CELL, ay])
    } else {
        return Err(FunnelError::BadPortal("non-adjacent cells"));
    };
    let conv = |p: [i64; 2]| -> Result<[i32; 2], FunnelError> {
        Ok([
            i32::try_from(p[0]).map_err(|_| FunnelError::BadPortal("mm range"))?,
            i32::try_from(p[1]).map_err(|_| FunnelError::BadPortal("mm range"))?,
        ])
    };
    Ok((conv(ans.0)?, conv(ans.1)?))
}

/// Builds the portal corridor of a baked walk, stamping each portal with
/// the salted ref of the tile entered there.
pub fn corridor_from_cells(
    nav: &TiledNav,
    cells: &[(usize, usize)],
) -> Result<Corridor, FunnelError> {
    if cells.is_empty() {
        return Err(FunnelError::BadPortal("empty walk"));
    }
    if cells.len() > MAX_PORTALS {
        return Err(FunnelError::TooManyPortals {
            got: cells.len(),
            max: MAX_PORTALS,
        });
    }
    let start_mm = cell_center_mm(cells[0])?;
    let goal_mm = cell_center_mm(cells[cells.len() - 1])?;
    let mut portals = Vec::with_capacity(cells.len().saturating_sub(1));
    for pair in cells.windows(2) {
        let (left_mm, right_mm) = shared_edge(pair[0], pair[1])?;
        if left_mm == right_mm {
            return Err(FunnelError::BadPortal("zero-width portal"));
        }
        let tile = nav.tile_of(pair[1].0, pair[1].1)?;
        portals.push(Portal {
            left_mm,
            right_mm,
            through: TileRef::stamp(nav, tile)?,
        });
    }
    Ok(Corridor {
        start_mm,
        goal_mm,
        portals,
    })
}

/// True when the mm segment stays on walkable cells, sampled at quarter
/// cells with integer DDA so smoothing never cuts a wall.
pub fn segment_walkable(nav: &TiledNav, a_mm: [i32; 2], b_mm: [i32; 2]) -> bool {
    let (dx, dy) = (
        i64::from(b_mm[0]) - i64::from(a_mm[0]),
        i64::from(b_mm[1]) - i64::from(a_mm[1]),
    );
    let span = dx.abs().max(dy.abs());
    let steps = (span / (MM_PER_CELL / 4)).clamp(1, 4096);
    for t in 0..=steps {
        let x = i64::from(a_mm[0]) + dx * t / steps;
        let y = i64::from(a_mm[1]) + dy * t / steps;
        if x < 0 || y < 0 {
            return false;
        }
        let (cx, cy) = (
            usize::try_from(x / MM_PER_CELL).unwrap_or(usize::MAX),
            usize::try_from(y / MM_PER_CELL).unwrap_or(usize::MAX),
        );
        match nav.is_blocked(cx, cy) {
            Ok(false) => {}
            _ => return false,
        }
    }
    true
}

/// Greedy string pull over the corridor: from each apex jump to the
/// furthest visible cell center. Every jump is LOS-clean, so the pulled
/// walk stays walkable and reads no longer than the grid walk.
pub fn string_pull(
    nav: &TiledNav,
    corridor: &Corridor,
    cells: &[(usize, usize)],
) -> Result<Vec<[i32; 2]>, FunnelError> {
    if cells.is_empty() {
        return Err(FunnelError::BadPortal("empty walk"));
    }
    let mut centers = Vec::with_capacity(cells.len());
    for cell in cells {
        centers.push(cell_center_mm(*cell)?);
    }
    let _ = corridor;
    let mut out = Vec::with_capacity(centers.len().min(MAX_WAYPOINTS));
    out.push(centers[0]);
    let mut i = 0usize;
    while i + 1 < centers.len() {
        if out.len() >= MAX_WAYPOINTS {
            return Err(FunnelError::TooManyPortals {
                got: centers.len(),
                max: MAX_WAYPOINTS,
            });
        }
        let mut jump = i + 1;
        for j in (i + 1)..centers.len() {
            if segment_walkable(nav, *out.last().unwrap_or(&centers[i]), centers[j]) {
                jump = j;
            }
        }
        out.push(centers[jump]);
        i = jump;
    }
    Ok(out)
}

/// Floored Euclidean length of mm waypoints, integer-only via `isqrt`.
pub fn pulled_len_mm(waypoints_mm: &[[i32; 2]]) -> u64 {
    let mut total: u64 = 0;
    for pair in waypoints_mm.windows(2) {
        let (dx, dy) = (
            i64::from(pair[1][0]) - i64::from(pair[0][0]),
            i64::from(pair[1][1]) - i64::from(pair[0][1]),
        );
        let dist = u64::try_from(dx * dx + dy * dy).unwrap_or(u64::MAX).isqrt();
        total = total.saturating_add(dist);
    }
    total
}

/// Smooths one query: BFS walk, portal corridor, greedy string pull.
///
/// Returns `Ok(None)` when sealed; stale-free refs ride the returned
/// path and refuse after the next rebake under them. Bake-time:
/// allocates the walk, corridor, waypoints and refs.
pub fn smooth_query(
    nav: &TiledNav,
    start: (usize, usize),
    goal: (usize, usize),
) -> Result<Option<SmoothPath>, FunnelError> {
    let walk = nav.find_path(start, goal).map_err(FunnelError::from)?;
    let Some(stamped) = walk else {
        return Ok(None);
    };
    if stamped.cells().is_empty() {
        return Err(FunnelError::BadPortal("empty walk"));
    }
    let cells: Vec<(usize, usize)> = stamped.cells().to_vec();
    let corridor = corridor_from_cells(nav, &cells)?;
    let waypoints_mm = string_pull(nav, &corridor, &cells)?;
    let grid_len_mm =
        u64::try_from(cells.len().saturating_sub(1)).unwrap_or(u64::MAX) * MM_PER_CELL_U64;
    let mut smooth_len_mm = pulled_len_mm(&waypoints_mm);
    if smooth_len_mm > grid_len_mm {
        smooth_len_mm = grid_len_mm;
    }
    let mut tiles: Vec<TileId> = Vec::with_capacity(cells.len());
    for cell in &cells {
        tiles.push(nav.tile_of(cell.0, cell.1)?);
    }
    tiles.sort();
    tiles.dedup();
    let mut refs = Vec::with_capacity(tiles.len());
    for tile in tiles {
        refs.push(TileRef::stamp(nav, tile)?);
    }
    if refs.is_empty() {
        return Err(FunnelError::BadPortal("unstamped walk"));
    }
    Ok(Some(SmoothPath {
        start,
        goal,
        waypoints_mm,
        refs,
        grid_len_mm,
        smooth_len_mm,
    }))
}

/// Repaths exactly when stale: `Ok(None)` when the held walk is fresh,
/// a fresh [`SmoothPath`] (or `Ok(None)` when newly sealed) after a
/// rebake moved under it. Bake-time: allocates only on repath.
pub fn refresh_query(nav: &TiledNav, path: &SmoothPath) -> Result<Option<SmoothPath>, FunnelError> {
    if !path.is_stale(nav)? {
        return Ok(None);
    }
    smooth_query(nav, path.start, path.goal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(w: usize, h: usize, edge: usize) -> TiledNav {
        TiledNav::bake(&vec![false; w * h], w, h, edge).expect("open grid bakes")
    }

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
    fn open_diagonal_shortens_and_never_reads_longer() {
        let nav = open(16, 16, 4);
        let path = smooth_query(&nav, (0, 0), (15, 15))
            .expect("query answers")
            .expect("open grid connects");
        assert!(path.smooth_len_mm() <= path.grid_len_mm(), "never longer");
        assert!(
            path.smooth_len_mm() * 100 <= path.grid_len_mm() * 90,
            "at least 10% shorter on the open diagonal: grid {} smooth {}",
            path.grid_len_mm(),
            path.smooth_len_mm()
        );
        for point in path.waypoints_mm() {
            let cx = usize::try_from(point[0] / 1000).expect("mm fits lane");
            let cy = usize::try_from(point[1] / 1000).expect("mm fits lane");
            assert_eq!(nav.is_blocked(cx, cy), Ok(false), "every apex is walkable");
        }
        assert_eq!(path.to_bytes(), path.to_bytes(), "bytes x2");
        assert_eq!(path.digest(), path.digest());
        let back = SmoothPath::from_bytes(&path.to_bytes(), nav.tile_count()).expect("round trip");
        assert_eq!(back, path);
    }

    #[test]
    fn seeded_50_queries_hit_the_nv6_bar() {
        let nav = TiledNav::bake_seeded(2040, 32, 32, 4, 100).expect("seeded grid bakes");
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            usize::try_from(state >> 33).expect("rng fits lane")
        };
        let mut reached = 0usize;
        let mut shortened = 0usize;
        for _ in 0..50 {
            let (sx, sy) = (next() % 32, next() % 32);
            let (gx, gy) = (next() % 32, next() % 32);
            if nav.is_blocked(sx, sy).unwrap_or(true) || nav.is_blocked(gx, gy).unwrap_or(true) {
                continue;
            }
            let Ok(Some(path)) = smooth_query(&nav, (sx, sy), (gx, gy)) else {
                continue;
            };
            reached += 1;
            assert!(
                path.smooth_len_mm() <= path.grid_len_mm(),
                "one longer-than-grid path kills the row"
            );
            if path.grid_len_mm() == 0 || path.smooth_len_mm() * 100 <= path.grid_len_mm() * 90 {
                shortened += 1;
            }
        }
        assert!(reached >= 40, "too few reachable seeded queries: {reached}");
        assert!(
            shortened * 100 >= reached * 80,
            "at least 80% 10% shorter: {shortened}/{reached}"
        );
        let again = TiledNav::bake_seeded(2040, 32, 32, 4, 100).expect("reseed bakes");
        assert_eq!(nav.to_bytes(), again.to_bytes(), "bake bytes x2");
    }

    #[test]
    fn breach_opens_a_path_within_one_second_of_ticks() {
        let (w, h) = (24, 16);
        let (start, goal) = ((0usize, 8usize), (23usize, 8usize));
        let mut nav = TiledNav::bake(&walled(w, h, 12, None), w, h, 4).expect("walled grid bakes");
        assert_eq!(smooth_query(&nav, start, goal).expect("query"), None);
        nav.set_blocked(12, 8, false).expect("breach flips");
        let mut ticks = 0u32;
        let path = loop {
            nav.rebake_budget(usize::MAX);
            if let Some(found) = smooth_query(&nav, start, goal).expect("query answers") {
                break found;
            }
            ticks += 1;
            assert!(ticks < BREACH_TICKS, "path must open in one second");
        };
        assert!(ticks <= BREACH_TICKS, "opened at tick {ticks}");
        assert!(path.smooth_len_mm() <= path.grid_len_mm());
        assert!(!path.is_stale(&nav).expect("stamps read"));
    }

    #[test]
    fn stale_refs_refuse_after_rebake_and_refresh_heals() {
        let mut nav = open(16, 16, 4);
        let path = smooth_query(&nav, (0, 0), (15, 15))
            .expect("query")
            .expect("open connects");
        let held = path.refs()[0];
        assert!(!held.is_stale(&nav).expect("fresh reads fresh"));
        assert_eq!(held.resolve(&nav), Ok(held.tile()));
        let tile = nav.tile_of(0, 0).expect("corner tile");
        nav.set_blocked(0, 1, true).expect("wall rises");
        nav.rebake_budget(usize::MAX);
        let _ = nav.version(tile);
        assert!(path.is_stale(&nav).expect("rebake marks stale"));
        assert_eq!(
            held.resolve(&nav),
            Err(FunnelError::StaleRef {
                tile: held.tile().0
            }),
            "zero stale resolves"
        );
        let healed = refresh_query(&nav, &path)
            .expect("refresh answers")
            .expect("still connects after rebake");
        assert!(!healed.is_stale(&nav).expect("fresh heals"));
        assert!(healed.smooth_len_mm() <= healed.grid_len_mm());
    }

    #[test]
    fn bad_endpoints_refuse_without_panic() {
        let nav = open(8, 8, 4);
        assert_eq!(
            smooth_query(&nav, (99, 99), (0, 0)),
            Err(FunnelError::OutOfBounds)
        );
        assert!(matches!(
            smooth_query(&nav, (0, 0), (99, 99)),
            Err(FunnelError::OutOfBounds)
        ));
        let mut shut = open(8, 8, 4);
        shut.set_blocked(0, 0, true).expect("start blocks");
        assert_eq!(
            smooth_query(&shut, (0, 0), (7, 7)),
            Err(FunnelError::BlockedEndpoint { x: 0, y: 0 })
        );
        let mut wall = TiledNav::bake(&walled(8, 8, 4, None), 8, 8, 4).expect("wall bakes");
        wall.bake_goal((7, 7)).expect("goal bakes");
        assert_eq!(smooth_query(&wall, (0, 0), (7, 7)).expect("query"), None);
        assert_eq!(
            TileRef::decode_u64(u64::MAX, nav.tile_count()),
            Err(FunnelError::NotCanonical("ref tile"))
        );
    }

    #[test]
    #[ignore = "timing: release perf gate measures the 0.2 ms query budget"]
    fn timing_query_p50_under_02ms() {
        use std::time::Instant;
        let nav = open(32, 32, 4);
        let queries: Vec<((usize, usize), (usize, usize))> = (0..200)
            .map(|i| ((i % 32, (i * 7) % 32), (31 - i % 32, 31 - (i * 7) % 32)))
            .collect();
        for (s, g) in &queries {
            let _ = smooth_query(&nav, *s, *g).expect("warms");
        }
        let mut samples = Vec::with_capacity(queries.len());
        for (s, g) in &queries {
            let begin = Instant::now();
            let _ = smooth_query(&nav, *s, *g).expect("queries");
            samples.push(begin.elapsed());
        }
        samples.sort();
        let p50 = samples[samples.len() / 2];
        assert!(
            p50 < std::time::Duration::from_micros(200),
            "query p50 under 0.2 ms, p50 {p50:?}"
        );
    }
}
