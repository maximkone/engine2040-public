//! wfc: wave function collapse tile bake — kinds plus an adjacency table.
//! Source: web: mxgmn/WaveFunctionCollapse simple tiled model + krychu/wfc,
//! read 2026-10-19, ideas only, zero bytes vendored.
//! Owner: engine-world, bake-time only, never the tick or the sim hash.
//! Units: cells are `u16` grid steps, `x` fastest then `z`; kinds are indices
//! `0..kinds`; direction 0 `+x`, 1 `-x`, 2 `+z`, 3 `-z`.
//! Determinism: the same `(width, height, adjacency, seed, constraints)` always
//! gives the same bytes; one collapse per step draws from `splitmix64(seed ^ step)`,
//! integers only, so the collapse order never depends on a float entropy, and
//! cells are scanned in grid order, never through a map.
//! Authored rules: [`Constraints`] pins a cell to a kind, forbids a kind in a
//! block and joins two cells with a path; [`bake_constrained`] satisfies every
//! rule or names the cell that cannot.
//! Errors: [`WfcError`] refuses a bad table, a bad or impossible rule and an
//! emptied wave; the solver backtracks within one seed up to [`MAX_BACKTRACKS`]
//! failed branches, then refuses with a typed error, never a partial grid (the
//! caller picks another seed).
//! Budget: `width * height <= MAX_WFC_SIDE^2` masks of one `u16`, one `Vec<u8>`
//! out, one `VecDeque` of cells; bake time, never the tick.
//! Consumer: the `world.bake`-style door row and the nav bake. An AI authors a
//! small table (docs/research/camera-mods-combat-2026-10-19/02-mods-and-pcg.md B).

use std::collections::VecDeque;
use std::fmt;

/// Content hash of a bake, the crate's one hash: [`engine_core::hash_bytes`],
/// exactly as [`world_hash`](crate::world_hash) tags terrain and ground bakes.
/// Re-exported rather than wrapped, so a WFC map and a terrain map share one
/// hash function and one `#[must_use]`, with no relay of its own.
pub use engine_core::hash_bytes as digest;

/// Bake format version of a WFC table or map row.
pub const WFC_VERSION: u8 = 1;
/// Largest accepted map side in cells. Above it a request is refused before
/// any allocation, so one call can never reserve gigabytes.
pub const MAX_WFC_SIDE: u16 = 128;
/// Largest accepted kind count: one bit per kind fits a `u16` mask.
pub const MAX_KINDS: usize = 16;
/// Direction count: 0 `+x`, 1 `-x`, 2 `+z`, 3 `-z`.
pub const DIRECTIONS: usize = 4;
/// Failed branches one constrained bake may try before it refuses.
///
/// The search is depth-first over grid order, so a cap makes the worst case
/// finite: no seed can hang a bake, and a refusal is typed, never a partial grid.
pub const MAX_BACKTRACKS: u32 = 512;

/// Tile kinds plus, for each kind and direction, the bitmask of the neighbour
/// kinds that may touch it.
///
/// Built through [`Adjacency::new`], so every stored table already passed
/// [`Adjacency::validate`]: no bit names a kind at or above `kinds`, and no
/// direction of any kind is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Adjacency {
    kinds: u8,
    masks: [[u16; DIRECTIONS]; MAX_KINDS],
}

impl Adjacency {
    /// Build a table from one mask row per kind, in kind order.
    ///
    /// Errors: [`WfcError::NoKinds`] for `kinds == 0`; [`WfcError::TooManyKinds`]
    /// above [`MAX_KINDS`]; [`WfcError::MaskOutOfRange`] for a bit naming a kind
    /// at or above `kinds`; [`WfcError::DeadEnd`] for a direction no neighbour
    /// can ever take. Rows past `kinds` are ignored, and a row the caller omits
    /// stays zero, which is the same dead end.
    pub fn new(kinds: u8, masks: &[[u16; DIRECTIONS]]) -> Result<Self, WfcError> {
        if kinds == 0 {
            return Err(WfcError::NoKinds);
        }
        if usize::from(kinds) > MAX_KINDS {
            return Err(WfcError::TooManyKinds {
                kinds: u16::from(kinds),
            });
        }
        let mut table = Self {
            kinds,
            masks: [[0; DIRECTIONS]; MAX_KINDS],
        };
        for (slot, row) in table.masks.iter_mut().zip(masks) {
            *slot = *row;
        }
        table.validate()?;
        Ok(table)
    }

    /// Kind count this table was built for.
    #[must_use]
    pub fn kinds(&self) -> u8 {
        self.kinds
    }

    /// Neighbour kinds kind `kind` may touch toward `dir`; 0 when `kind` or
    /// `dir` is out of range, so a caller probe never panics.
    #[must_use]
    pub fn allows(&self, kind: u8, dir: usize) -> u16 {
        self.masks
            .get(usize::from(kind))
            .and_then(|row| row.get(dir))
            .copied()
            .unwrap_or(0)
    }

    /// Re-check every stored row. Errors: [`WfcError::MaskOutOfRange`],
    /// [`WfcError::DeadEnd`].
    pub fn validate(&self) -> Result<(), WfcError> {
        let in_range = kind_bits(self.kinds);
        for (kind, row) in (0u8..self.kinds).zip(self.masks.iter()) {
            for (dir, &mask) in row.iter().enumerate() {
                if mask & !in_range != 0 {
                    return Err(WfcError::MaskOutOfRange { kind, dir });
                }
                if mask == 0 {
                    return Err(WfcError::DeadEnd { kind, dir });
                }
            }
        }
        Ok(())
    }
}

/// Why a WFC request or table was refused. Nothing is allocated on refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WfcError {
    /// A side exceeds [`MAX_WFC_SIDE`].
    TooLarge {
        /// The refused side in cells.
        side: u16,
    },
    /// More kinds than [`MAX_KINDS`].
    TooManyKinds {
        /// The refused kind count.
        kinds: u16,
    },
    /// A table needs at least one kind.
    NoKinds,
    /// An adjacency bit names a kind at or above `kinds`.
    MaskOutOfRange {
        /// Row the bit sits in.
        kind: u8,
        /// Direction the bit sits in.
        dir: usize,
    },
    /// A kind that can never have a neighbour toward this direction.
    DeadEnd {
        /// The stranded kind.
        kind: u8,
        /// The direction with no possible neighbour.
        dir: usize,
    },
    /// The wave collapsed to nothing at this cell; the caller picks another seed.
    Contradiction {
        /// Cell x in grid steps.
        x: u16,
        /// Cell z in grid steps.
        z: u16,
    },
    /// A serialized table or map names a format version this build cannot read.
    VersionUnknown {
        /// The refused version byte.
        version: u8,
    },
    /// A authored rule names a cell this map does not hold.
    CellOutOfBounds {
        /// Cell x in grid steps.
        x: u16,
        /// Cell z in grid steps.
        z: u16,
    },
    /// A rule names a kind at or above `adjacency.kinds()`.
    KindOutOfRange {
        /// Cell x the rule starts at.
        x: u16,
        /// Cell z the rule starts at.
        z: u16,
        /// The refused kind index.
        kind: TileId,
    },
    /// A forbidden block ends before it starts, so it holds no cell.
    RectInverted {
        /// Block first column.
        x: u16,
        /// Block first row.
        z: u16,
    },
    /// An `open` bit names a kind at or above `adjacency.kinds()`.
    OpenMaskOutOfRange {
        /// The refused kind index.
        kind: TileId,
    },
    /// A connect rule needs an `open` mask, and the mask is zero.
    OpenMaskEmpty,
    /// Two or more rules left this cell with no possible kind.
    UnsatisfiableCell {
        /// Cell x in grid steps.
        x: u16,
        /// Cell z in grid steps.
        z: u16,
    },
    /// The baked grid holds no `open` path between the two cells.
    ConnectUnreachable {
        /// Path start, grid steps.
        from: (u16, u16),
        /// Path end, grid steps.
        to: (u16, u16),
    },
    /// The search used every failed branch the cap allows at this cell.
    BacktrackCap {
        /// Cell the search was retrying.
        x: u16,
        /// Cell z the search was retrying.
        z: u16,
        /// The cap that was hit.
        cap: u32,
    },
}

impl fmt::Display for WfcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WfcError::TooLarge { side } => {
                write!(f, "wfc side {side} exceeds MAX_WFC_SIDE")
            }
            WfcError::TooManyKinds { kinds } => {
                write!(f, "wfc kind count {kinds} exceeds MAX_KINDS")
            }
            WfcError::NoKinds => write!(f, "wfc needs at least one kind"),
            WfcError::MaskOutOfRange { kind, dir } => {
                write!(
                    f,
                    "wfc mask for kind {kind} dir {dir} names a kind above kinds-1"
                )
            }
            WfcError::DeadEnd { kind, dir } => {
                write!(
                    f,
                    "wfc kind {kind} has no possible neighbour toward dir {dir}"
                )
            }
            WfcError::Contradiction { x, z } => {
                write!(f, "wfc wave emptied at cell ({x}, {z})")
            }
            WfcError::VersionUnknown { version } => {
                write!(f, "wfc version {version} unknown")
            }
            WfcError::CellOutOfBounds { x, z } => {
                write!(f, "wfc rule names cell ({x}, {z}) outside the map")
            }
            WfcError::KindOutOfRange { x, z, kind } => {
                write!(
                    f,
                    "wfc rule at cell ({x}, {z}) names kind {kind} above kinds-1"
                )
            }
            WfcError::RectInverted { x, z } => {
                write!(f, "wfc forbidden block at ({x}, {z}) ends before it starts")
            }
            WfcError::OpenMaskOutOfRange { kind } => {
                write!(f, "wfc open mask names kind {kind} above kinds-1")
            }
            WfcError::OpenMaskEmpty => {
                write!(f, "wfc connect rule needs a non-zero open mask")
            }
            WfcError::UnsatisfiableCell { x, z } => {
                write!(f, "wfc rules leave cell ({x}, {z}) with no kind")
            }
            WfcError::ConnectUnreachable { from, to } => {
                write!(
                    f,
                    "wfc baked no open path from ({}, {}) to ({}, {})",
                    from.0, from.1, to.0, to.1
                )
            }
            WfcError::BacktrackCap { x, z, cap } => {
                write!(
                    f,
                    "wfc used {cap} failed branches at cell ({x}, {z}) without a grid"
                )
            }
        }
    }
}

impl std::error::Error for WfcError {}

/// A tile kind index, `0..`[`Adjacency::kinds`]; the grid stores these bytes.
pub type TileId = u8;

/// Axis-aligned block of cells with inclusive corners, in grid steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// First column (west edge).
    pub x0: u16,
    /// First row (north edge).
    pub z0: u16,
    /// Last column, included.
    pub x1: u16,
    /// Last row, included.
    pub z1: u16,
}

impl Rect {
    /// A block from its two inclusive corners; [`Constraints::validate`] refuses
    /// a block that is inverted or reaches past the map.
    #[must_use]
    pub const fn new(x0: u16, z0: u16, x1: u16, z1: u16) -> Self {
        Self { x0, z0, x1, z1 }
    }

    /// A block of one cell.
    #[must_use]
    pub const fn cell(x: u16, z: u16) -> Self {
        Self::new(x, z, x, z)
    }

    /// `true` when `(x, z)` lies inside, corners included.
    #[must_use]
    pub const fn holds(&self, x: u16, z: u16) -> bool {
        x >= self.x0 && x <= self.x1 && z >= self.z0 && z <= self.z1
    }
}

/// Authored rules a constrained bake must satisfy, in grid steps.
///
/// Every rule is read in list order and applied as a mask on cells, so the same
/// lists always give the same masks. A rule that cannot hold is refused with the
/// cell it names ([`WfcError::UnsatisfiableCell`]); nothing is ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constraints {
    /// Cell `(x, z)` and the kind it must hold. Pins of one cell intersect, so
    /// two different pins on one cell are refused, not resolved.
    pub pinned: Vec<(u16, u16, TileId)>,
    /// A block that may never hold the kind.
    pub forbidden: Vec<(Rect, TileId)>,
    /// Two cells that must share a 4-way path of `open` kinds. The bake claims
    /// the L corridor (x first, then z) for every pair, so the path exists.
    pub connect: Vec<((u16, u16), (u16, u16))>,
    /// Kinds a `connect` path may step on; `0` refuses every connect rule.
    pub open: u16,
}

impl Constraints {
    /// Check every rule against the map size and the table before any allocation.
    /// Errors name the offending cell: [`WfcError::CellOutOfBounds`],
    /// [`WfcError::KindOutOfRange`], [`WfcError::RectInverted`],
    /// [`WfcError::OpenMaskOutOfRange`], [`WfcError::OpenMaskEmpty`].
    pub fn validate(&self, width: u16, height: u16, adjacency: &Adjacency) -> Result<(), WfcError> {
        let kinds = adjacency.kinds();
        for &(x, z, kind) in &self.pinned {
            cell_in_map(x, z, width, height)?;
            kind_in_range(kind, x, z, kinds)?;
        }
        for &(rect, kind) in &self.forbidden {
            if rect.x0 > rect.x1 || rect.z0 > rect.z1 {
                return Err(WfcError::RectInverted {
                    x: rect.x0,
                    z: rect.z0,
                });
            }
            cell_in_map(rect.x1, rect.z1, width, height)?;
            kind_in_range(kind, rect.x0, rect.z0, kinds)?;
        }
        for &(from, to) in &self.connect {
            cell_in_map(from.0, from.1, width, height)?;
            cell_in_map(to.0, to.1, width, height)?;
        }
        if self.connect.is_empty() {
            return Ok(());
        }
        self.validate_open(kinds)
    }

    /// Check the `open` mask alone: one bit per kind, at least one bit.
    fn validate_open(&self, kinds: u8) -> Result<(), WfcError> {
        if self.open == 0 {
            return Err(WfcError::OpenMaskEmpty);
        }
        let over = self.open & !kind_bits(kinds);
        if over == 0 {
            return Ok(());
        }
        let kind = u8::try_from(over.trailing_zeros()).unwrap_or(0);
        Err(WfcError::OpenMaskOutOfRange { kind })
    }
}

/// Refuse a cell outside the map; names the cell.
fn cell_in_map(x: u16, z: u16, width: u16, height: u16) -> Result<(), WfcError> {
    if x >= width || z >= height {
        return Err(WfcError::CellOutOfBounds { x, z });
    }
    Ok(())
}

/// Refuse a kind at or above `kinds`; names the cell the rule starts at.
fn kind_in_range(kind: TileId, x: u16, z: u16, kinds: u8) -> Result<(), WfcError> {
    if kind >= kinds {
        return Err(WfcError::KindOutOfRange { x, z, kind });
    }
    Ok(())
}

/// Bake one map: `width * height` cells of kind indices, row-major, `x` fastest.
///
/// The same as [`bake_constrained`] with [`Constraints::default`]: no authored
/// rule. Errors: [`WfcError::TooLarge`], [`WfcError::Contradiction`],
/// [`WfcError::BacktrackCap`].
pub fn bake(
    width: u16,
    height: u16,
    adjacency: &Adjacency,
    seed: u64,
) -> Result<Vec<u8>, WfcError> {
    bake_constrained(width, height, adjacency, seed, &Constraints::default())
}

/// Bake one map that satisfies every authored rule, or refuse with the reason.
///
/// Order: size, then rule validation, then masks, then arc consistency, then the
/// seeded search, then the connect check. Errors: [`WfcError::TooLarge`],
/// [`WfcError::CellOutOfBounds`], [`WfcError::KindOutOfRange`],
/// [`WfcError::RectInverted`], [`WfcError::OpenMaskOutOfRange`],
/// [`WfcError::OpenMaskEmpty`], [`WfcError::UnsatisfiableCell`],
/// [`WfcError::Contradiction`], [`WfcError::BacktrackCap`],
/// [`WfcError::ConnectUnreachable`].
///
/// Determinism: the same seed and rules give byte-identical grids. A refusal
/// leaves no partial grid behind.
pub fn bake_constrained(
    width: u16,
    height: u16,
    adjacency: &Adjacency,
    seed: u64,
    constraints: &Constraints,
) -> Result<Vec<u8>, WfcError> {
    if width > MAX_WFC_SIDE {
        return Err(WfcError::TooLarge { side: width });
    }
    if height > MAX_WFC_SIDE {
        return Err(WfcError::TooLarge { side: height });
    }
    constraints.validate(width, height, adjacency)?;
    let kinds = adjacency.kinds();
    let cells = usize::from(width) * usize::from(height);
    let mut masks = vec![kind_bits(kinds); cells];
    if apply_constraints(&mut masks, width, height, constraints)? {
        settle(&mut masks, adjacency, width, height)?;
    }
    let grid = search(&mut masks, adjacency, width, height, seed, MAX_BACKTRACKS)?;
    check_connect(&grid, width, height, constraints)?;
    Ok(grid)
}

/// Mask every rule onto the wave. `Ok(true)` when a mask shrank, `Ok(false)` for
/// an empty rule list. Errors: [`WfcError::UnsatisfiableCell`] at the first cell
/// left with no kind, in grid order.
fn apply_constraints(
    masks: &mut [u16],
    width: u16,
    height: u16,
    constraints: &Constraints,
) -> Result<bool, WfcError> {
    let mut changed = false;
    for &(x, z, kind) in &constraints.pinned {
        let cell = index(x, z, width);
        let pinned = masks[cell] & (1u16 << kind);
        changed |= pinned != masks[cell];
        masks[cell] = pinned;
    }
    for &(rect, kind) in &constraints.forbidden {
        for z in rect.z0..=rect.z1 {
            for x in rect.x0..=rect.x1 {
                let cell = index(x, z, width);
                let open = masks[cell] & !(1u16 << kind);
                changed |= open != masks[cell];
                masks[cell] = open;
            }
        }
    }
    for &(from, to) in &constraints.connect {
        for (x, z) in corridor(from, to) {
            let cell = index(x, z, width);
            let open = masks[cell] & constraints.open;
            changed |= open != masks[cell];
            masks[cell] = open;
        }
    }
    for z in 0..height {
        for x in 0..width {
            if masks[index(x, z, width)] == 0 {
                return Err(WfcError::UnsatisfiableCell { x, z });
            }
        }
    }
    Ok(changed)
}

/// Cells of the L corridor from `from` to `to`: every x first, then every z.
/// The last cell is `to`, so the corridor always joins the pair.
fn corridor(from: (u16, u16), to: (u16, u16)) -> Vec<(u16, u16)> {
    let (x1, z1) = to;
    let mut out = Vec::new();
    let mut x = from.0;
    while x != x1 {
        out.push((x, from.1));
        x = if x < x1 {
            x.saturating_add(1)
        } else {
            x.saturating_sub(1)
        };
    }
    let mut z = from.1;
    while z != z1 {
        out.push((x1, z));
        z = if z < z1 {
            z.saturating_add(1)
        } else {
            z.saturating_sub(1)
        };
    }
    out.push((x1, z1));
    out
}

/// Run arc consistency from every cell, so a mask restricted by a rule reaches
/// the whole map before the search starts.
fn settle(
    masks: &mut [u16],
    adjacency: &Adjacency,
    width: u16,
    height: u16,
) -> Result<(), WfcError> {
    let mut trail = Trail::default();
    for z in 0..height {
        for x in 0..width {
            propagate(masks, &mut trail, adjacency, width, height, (x, z))?;
        }
    }
    Ok(())
}

/// Byte grid of the wave; a settled cell holds exactly one kind.
fn settled(masks: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(masks.len());
    for &mask in masks {
        out.push(lowest_bit(mask));
    }
    out
}

/// One collapse decision still open to other kinds.
#[derive(Clone, Copy)]
struct Frame {
    /// Cell this decision sits on, row-major.
    cell: usize,
    /// Cell x in grid steps, kept for refusal messages.
    x: u16,
    /// Cell z in grid steps, kept for refusal messages.
    z: u16,
    /// Kinds not yet tried here, as a mask.
    candidates: u16,
    /// Seed draw that orders the kinds of this cell.
    want: u64,
    /// Trail length before this decision's own commit, so a retry undoes it.
    trail: usize,
}

impl Frame {
    /// Take the next kind to try: the `want % count_ones`-th of `candidates`,
    /// in bit order. `None` when every kind of this cell was tried.
    fn take_kind(&mut self, kinds: u8) -> Option<TileId> {
        let kind = nth_kind(self.candidates, self.want, kinds)?;
        self.candidates &= !(1u16 << kind);
        Some(kind)
    }
}

/// Cells changed since a checkpoint with their old mask, so a failed branch
/// restores the wave exactly. Never a map: the trail is an ordered `Vec`.
#[derive(Default)]
struct Trail {
    /// `(cell, old mask)`, oldest first.
    edits: Vec<(usize, u16)>,
}

impl Trail {
    /// Set one cell and remember its old mask.
    fn set(&mut self, masks: &mut [u16], cell: usize, mask: u16) {
        self.edits.push((cell, masks[cell]));
        masks[cell] = mask;
    }

    /// Length to remember before a branch and pass back to [`Trail::rollback`].
    fn mark(&self) -> usize {
        self.edits.len()
    }

    /// Undo every edit above `keep`, newest first.
    fn rollback(&mut self, masks: &mut [u16], keep: usize) {
        while self.edits.len() > keep {
            if let Some((cell, mask)) = self.edits.pop() {
                masks[cell] = mask;
            }
        }
    }
}

/// Seeded collapse with depth-first backtracking: collapse the fewest-kind cell,
/// propagate, and on a contradiction try the next kind of the nearest open
/// decision. At most `cap` failed branches; past it, a typed refusal.
///
/// Errors: [`WfcError::Contradiction`] when the root decision has no kind left,
/// [`WfcError::BacktrackCap`] when `cap` branches failed.
fn search(
    masks: &mut [u16],
    adjacency: &Adjacency,
    width: u16,
    height: u16,
    seed: u64,
    cap: u32,
) -> Result<Vec<u8>, WfcError> {
    let kinds = adjacency.kinds();
    let mut trail = Trail::default();
    let mut frames: Vec<Frame> = Vec::new();
    let mut step = 0u64;
    let mut failed = 0u32;
    let mut retry = false;
    loop {
        if !retry {
            let Some((x, z)) = next_cell(masks, width, height) else {
                break;
            };
            let cell = index(x, z, width);
            let want = engine_core::splitmix64(seed ^ step);
            step = step.wrapping_add(1);
            frames.push(Frame {
                cell,
                x,
                z,
                candidates: masks[cell],
                want,
                trail: trail.mark(),
            });
        }
        retry = false;
        let Some(frame) = frames.last_mut() else {
            break;
        };
        let Some(kind) = frame.take_kind(kinds) else {
            // Every kind failed here, so the decision before it was wrong.
            let Some(spent) = frames.pop() else {
                break;
            };
            let Some(parent) = frames.last() else {
                return Err(WfcError::Contradiction {
                    x: spent.x,
                    z: spent.z,
                });
            };
            let keep = parent.trail;
            trail.rollback(masks, keep);
            failed = failed.saturating_add(1);
            if failed > cap {
                return Err(WfcError::BacktrackCap {
                    x: spent.x,
                    z: spent.z,
                    cap,
                });
            }
            retry = true;
            continue;
        };
        let Some(frame) = frames.last() else {
            break;
        };
        let (cell, x, z, keep) = (frame.cell, frame.x, frame.z, frame.trail);
        trail.set(masks, cell, 1u16 << kind);
        if propagate(masks, &mut trail, adjacency, width, height, (x, z)).is_err() {
            trail.rollback(masks, keep);
            failed = failed.saturating_add(1);
            if failed > cap {
                return Err(WfcError::BacktrackCap { x, z, cap });
            }
            retry = true;
        }
    }
    Ok(settled(masks))
}

/// Every connect rule on the baked grid: both cells reachable over `open` kinds.
/// Errors: [`WfcError::ConnectUnreachable`] names the pair that lost its path.
fn check_connect(
    cells: &[u8],
    width: u16,
    height: u16,
    constraints: &Constraints,
) -> Result<(), WfcError> {
    for &(from, to) in &constraints.connect {
        if !path_holds(cells, width, height, constraints.open, from, to) {
            return Err(WfcError::ConnectUnreachable { from, to });
        }
    }
    Ok(())
}

/// Flood fill over passable kinds (a bitmask of kind indices): true when every
/// cell of a passable kind is reachable from the first passable cell.
///
/// False when `cells.len()` is not `width * height`. An empty passable set is
/// true: there is no passable cell left unreachable.
#[must_use]
pub fn is_connected(cells: &[u8], width: u16, height: u16, passable: u16) -> bool {
    if cells.len() != usize::from(width) * usize::from(height) {
        return false;
    }
    if passable == 0 {
        return true;
    }
    let (total, first) = passable_scan(cells, width, height, passable);
    match first {
        None => true,
        Some(start) => flood(cells, width, height, passable, start, None).0 == total,
    }
}

/// `true` when the grid holds a 4-way path of `passable` kinds from `from` to
/// `to`, both cells included. The connect rule's own check, never a rewrite.
#[must_use]
fn path_holds(
    cells: &[u8],
    width: u16,
    height: u16,
    passable: u16,
    from: (u16, u16),
    to: (u16, u16),
) -> bool {
    if cells.len() != usize::from(width) * usize::from(height) {
        return false;
    }
    if !in_set(passable, cells[index(from.0, from.1, width)]) {
        return false;
    }
    flood(cells, width, height, passable, from, Some(to)).1
}

/// Mask holding one bit per kind below `kinds` (kind 0 is bit 0).
#[must_use]
fn kind_bits(kinds: u8) -> u16 {
    if usize::from(kinds) >= MAX_KINDS {
        u16::MAX
    } else {
        (1u16 << kinds) - 1
    }
}

/// `true` when `kind` is a member of the set `set`; kinds at or above
/// [`MAX_KINDS`] are never members, so no shift overflows.
#[must_use]
fn in_set(set: u16, kind: u8) -> bool {
    set.checked_shr(u32::from(kind))
        .is_some_and(|shifted| shifted & 1 == 1)
}

/// Row-major cell index: `x` fastest, then `z`.
#[must_use]
fn index(x: u16, z: u16, width: u16) -> usize {
    usize::from(z) * usize::from(width) + usize::from(x)
}

/// Neighbour of `(x, z)` toward `dir`, or `None` at the map edge: borders are
/// open, so a border cell is never constrained from outside.
#[must_use]
fn step(x: u16, z: u16, dir: usize, width: u16, height: u16) -> Option<(u16, u16)> {
    match dir {
        0 => x.checked_add(1).filter(|nx| *nx < width).map(|nx| (nx, z)),
        1 => x.checked_sub(1).map(|nx| (nx, z)),
        2 => z.checked_add(1).filter(|nz| *nz < height).map(|nz| (x, nz)),
        3 => z.checked_sub(1).map(|nz| (x, nz)),
        _ => None,
    }
}

/// Fewest-allowed-kind cell that still holds more than one kind; lowest index
/// wins ties (integer, no float entropy). `None` when the wave is settled.
#[must_use]
fn next_cell(masks: &[u16], width: u16, height: u16) -> Option<(u16, u16)> {
    let mut best: Option<(u32, u16, u16)> = None;
    'scan: for z in 0..height {
        for x in 0..width {
            let count = masks[index(x, z, width)].count_ones();
            if count <= 1 {
                continue;
            }
            if best.is_none_or(|(fewest, _, _)| count < fewest) {
                best = Some((count, x, z));
            }
            if count == 2 {
                // Nothing beats two kinds, so the lowest index among them wins.
                break 'scan;
            }
        }
    }
    best.map(|(_, x, z)| (x, z))
}

/// The `want % popcount`-th allowed kind of `mask`; `None` when `mask` is empty.
#[must_use]
fn nth_kind(mask: u16, want: u64, kinds: u8) -> Option<u8> {
    let count = mask.count_ones();
    if count == 0 {
        return None;
    }
    let mut rank = want % u64::from(count);
    for kind in 0..kinds {
        if mask & (1u16 << kind) == 0 {
            continue;
        }
        if rank == 0 {
            return Some(kind);
        }
        rank -= 1;
    }
    None
}

/// Union of the kinds every kind still set in `mask` allows toward `dir`.
#[must_use]
fn allowed_toward(mask: u16, adjacency: &Adjacency, dir: usize) -> u16 {
    let mut allowed = 0u16;
    for (kind, row) in (0u8..adjacency.kinds()).zip(adjacency.masks.iter()) {
        if mask & (1u16 << kind) != 0 {
            allowed |= row.get(dir).copied().unwrap_or(0);
        }
    }
    allowed
}

/// Kind index of the lowest set bit of `mask`; `0` for an empty mask, which
/// [`bake`] never leaves behind: every emptied mask returns a contradiction.
#[must_use]
fn lowest_bit(mask: u16) -> u8 {
    u8::try_from(mask.trailing_zeros()).unwrap_or(0)
}

/// Shrink neighbour masks from `start` until the grid is arc-consistent, and
/// record every shrink on `trail` so a failed branch can undo it.
/// Errors: [`WfcError::Contradiction`] at the cell that lost every kind; the
/// caller rolls `trail` back to its own checkpoint.
fn propagate(
    masks: &mut [u16],
    trail: &mut Trail,
    adjacency: &Adjacency,
    width: u16,
    height: u16,
    start: (u16, u16),
) -> Result<(), WfcError> {
    let mut queue = VecDeque::from([start]);
    while let Some((x, z)) = queue.pop_front() {
        let mask = masks[index(x, z, width)];
        for dir in 0..DIRECTIONS {
            let Some((nx, nz)) = step(x, z, dir, width, height) else {
                continue;
            };
            let cell = index(nx, nz, width);
            let shrunk = masks[cell] & allowed_toward(mask, adjacency, dir);
            if shrunk == masks[cell] {
                continue;
            }
            if shrunk == 0 {
                return Err(WfcError::Contradiction { x: nx, z: nz });
            }
            trail.set(masks, cell, shrunk);
            queue.push_back((nx, nz));
        }
    }
    Ok(())
}

/// Passable-cell count and the first passable cell in row-major order.
#[must_use]
fn passable_scan(
    cells: &[u8],
    width: u16,
    height: u16,
    passable: u16,
) -> (usize, Option<(u16, u16)>) {
    let mut total = 0usize;
    let mut first = None;
    for z in 0..height {
        for x in 0..width {
            if in_set(passable, cells[index(x, z, width)]) {
                total += 1;
                if first.is_none() {
                    first = Some((x, z));
                }
            }
        }
    }
    (total, first)
}

/// Passable cells reachable from `start` by 4-way steps, `start` included, and
/// whether `stop` was among them.
/// Units: cell steps in, cell count out.
fn flood(
    cells: &[u8],
    width: u16,
    height: u16,
    passable: u16,
    start: (u16, u16),
    stop: Option<(u16, u16)>,
) -> (usize, bool) {
    let mut seen = vec![false; cells.len()];
    seen[index(start.0, start.1, width)] = true;
    let mut stack = vec![start];
    let mut reached = 0usize;
    let mut touches = false;
    while let Some((x, z)) = stack.pop() {
        reached += 1;
        touches |= stop == Some((x, z));
        for dir in 0..DIRECTIONS {
            let Some((nx, nz)) = step(x, z, dir, width, height) else {
                continue;
            };
            let cell = index(nx, nz, width);
            if seen[cell] || !in_set(passable, cells[cell]) {
                continue;
            }
            seen[cell] = true;
            stack.push((nx, nz));
        }
    }
    (reached, touches)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generous table: every kind tolerates every kind, itself included, so
    /// nothing ever propagates and each cell collapses on its own draw.
    fn generous() -> Adjacency {
        let rows = [[0b111u16; DIRECTIONS]; 3];
        Adjacency::new(3, &rows).expect("generous 3-kind table is valid")
    }

    /// Structured table: a 0 forces a 1 on both sides in x; a 1 forces nothing.
    /// Two 0s can never sit side by side, so no mask ever empties.
    fn structured() -> Adjacency {
        let rows = [[0b10, 0b10, 0b11, 0b11], [0b11, 0b11, 0b11, 0b11]];
        Adjacency::new(2, &rows).expect("structured table is valid")
    }

    /// Self-only table: a kind allows only itself everywhere, so the first
    /// collapse floods the whole map with that one kind.
    fn self_only() -> Adjacency {
        let rows = [[0b01, 0b01, 0b01, 0b01], [0b10, 0b10, 0b10, 0b10]];
        Adjacency::new(2, &rows).expect("self-only table is valid")
    }

    /// Tight table: a 2 needs a 1 above and below, while 0 and 1 are free. A
    /// greedy bake can still strand a 2 beside a settled 0, so it must backtrack.
    fn trap() -> Adjacency {
        let rows = [
            [0b111, 0b111, 0b111, 0b111],
            [0b111, 0b111, 0b111, 0b111],
            [0b010, 0b010, 0b111, 0b111],
        ];
        Adjacency::new(3, &rows).expect("trap table is valid")
    }

    /// The authored rules the seed sweep bakes: two pins, one block that may
    /// never hold a 2, and a path from (2, 8) to (13, 8) over kinds 0 and 1.
    fn authored() -> Constraints {
        Constraints {
            pinned: vec![(0, 0, 0), (15, 15, 1)],
            forbidden: vec![(Rect::new(4, 4, 7, 7), 2)],
            connect: vec![((2, 8), (13, 8))],
            open: 0b0011,
        }
    }

    /// Every adjacency of the baked grid agrees with the table.
    fn assert_neighbours_agree(cells: &[u8], width: u16, height: u16, adjacency: &Adjacency) {
        for z in 0..height {
            for x in 0..width {
                let kind = cells[index(x, z, width)];
                for dir in 0..DIRECTIONS {
                    let Some((nx, nz)) = step(x, z, dir, width, height) else {
                        continue;
                    };
                    let other = cells[index(nx, nz, width)];
                    assert_ne!(
                        adjacency.allows(kind, dir) & (1u16 << other),
                        0,
                        "cell ({x},{z}) kind {kind} refuses kind {other} toward dir {dir}"
                    );
                }
            }
        }
    }

    /// Every authored rule holds on the baked grid.
    fn assert_rules_hold(
        cells: &[u8],
        width: u16,
        height: u16,
        adjacency: &Adjacency,
        rules: &Constraints,
    ) {
        for &(x, z, kind) in &rules.pinned {
            assert_eq!(cells[index(x, z, width)], kind, "pin ({x}, {z}) holds");
        }
        for &(rect, kind) in &rules.forbidden {
            for z in rect.z0..=rect.z1 {
                for x in rect.x0..=rect.x1 {
                    assert_ne!(
                        cells[index(x, z, width)],
                        kind,
                        "block ({x}, {z}) stays clear of kind {kind}"
                    );
                }
            }
        }
        for &(from, to) in &rules.connect {
            assert!(
                path_holds(cells, width, height, rules.open, from, to),
                "path {from:?} to {to:?} joins over open kinds"
            );
        }
        assert_neighbours_agree(cells, width, height, adjacency);
    }

    #[test]
    fn bake_is_deterministic_and_seed_sensitive() {
        let adj = generous();
        let first = bake(16, 16, &adj, 7).expect("generous table bakes");
        let again = bake(16, 16, &adj, 7).expect("generous table bakes");
        assert_eq!(first.len(), 256);
        assert_eq!(first, again);
        assert_eq!(digest(&first), digest(&again));
        let other = bake(16, 16, &adj, 8).expect("generous table bakes");
        assert_ne!(first, other);
    }

    #[test]
    fn baked_neighbours_agree_with_the_table() {
        for adj in [generous(), structured()] {
            let cells = bake(16, 16, &adj, 11).expect("table bakes a full map");
            assert_eq!(cells.len(), 256);
            assert_neighbours_agree(&cells, 16, 16, &adj);
        }
    }

    #[test]
    fn impossible_table_dies_in_bake_not_in_new() {
        // Kind 0 demands kind 1 in +x and kind 0 in -x; kind 1 is the mirror.
        // Every direction lists exactly one kind, so `new` and `validate`
        // accept it: no direction is empty. The two rows still agree on no
        // adjacent pair at all, so the wave dies whichever kind collapses
        // first; the emptied cell is the collapsed one, (0, 0).
        let rows = [[0b10, 0b01, 0b11, 0b11], [0b01, 0b10, 0b11, 0b11]];
        let adj = Adjacency::new(2, &rows).expect("new accepts a locally live table");
        assert_eq!(
            bake(2, 1, &adj, 7),
            Err(WfcError::Contradiction { x: 0, z: 0 })
        );
        assert_eq!(
            bake(2, 1, &adj, 8),
            Err(WfcError::Contradiction { x: 0, z: 0 })
        );
    }

    #[test]
    fn new_refuses_bad_tables() {
        assert_eq!(Adjacency::new(0, &[]), Err(WfcError::NoKinds));
        let two = [[0b11u16; DIRECTIONS]; 2];
        assert_eq!(
            Adjacency::new(17, &two),
            Err(WfcError::TooManyKinds { kinds: 17 })
        );
        // A bit above kinds-1 names a kind that does not exist.
        let wide = [[0b100u16, 0b11, 0b11, 0b11], [0b11; DIRECTIONS]];
        assert_eq!(
            Adjacency::new(2, &wide),
            Err(WfcError::MaskOutOfRange { kind: 0, dir: 0 })
        );
        // A zero row can take no neighbour anywhere; an omitted row is the same.
        let dead = [[0u16; DIRECTIONS], [0b11; DIRECTIONS]];
        assert_eq!(
            Adjacency::new(2, &dead),
            Err(WfcError::DeadEnd { kind: 0, dir: 0 })
        );
        assert_eq!(
            Adjacency::new(2, &[[0b11u16; DIRECTIONS]]),
            Err(WfcError::DeadEnd { kind: 1, dir: 0 })
        );
        // The cap itself is accepted, and a probe out of range reads 0.
        let full = [[u16::MAX; DIRECTIONS]; MAX_KINDS];
        let adj = Adjacency::new(16, &full).expect("16 kinds are allowed");
        assert_eq!(adj.kinds(), 16);
        assert_eq!(adj.allows(0, 0), u16::MAX);
        assert_eq!(adj.allows(16, 0), 0);
        assert_eq!(adj.allows(0, DIRECTIONS), 0);
    }

    #[test]
    fn bake_refuses_a_side_above_the_cap() {
        let adj = generous();
        let over = MAX_WFC_SIDE + 1;
        assert_eq!(
            bake(over, 1, &adj, 7),
            Err(WfcError::TooLarge { side: over })
        );
        assert_eq!(
            bake(1, over, &adj, 7),
            Err(WfcError::TooLarge { side: over })
        );
        // The cap bakes; with a self-only table one collapse floods every cell.
        let side = usize::from(MAX_WFC_SIDE);
        let cells = bake(MAX_WFC_SIDE, MAX_WFC_SIDE, &self_only(), 7).expect("the cap bakes");
        assert_eq!(cells.len(), side * side);
        assert!(cells.iter().all(|&kind| kind == cells[0]));
        assert!(cells[0] < 2);
    }

    #[test]
    fn is_connected_finds_walls_and_open_ground() {
        let solid = [0u8; 64];
        assert!(is_connected(&solid, 8, 8, 0b1));
        // A full column of kind 1 splits the grid for a passable set of kind 0 only.
        let mut split = [0u8; 64];
        for z in 0..8u16 {
            split[index(2, z, 8)] = 1;
        }
        assert!(!is_connected(&split, 8, 8, 0b1));
        // Both kinds passable: the same bytes are connected again.
        assert!(is_connected(&split, 8, 8, 0b11));
        // No passable kind at all is vacuously connected; a short slice is refused.
        assert!(is_connected(&solid, 8, 8, 0));
        assert!(is_connected(&solid, 8, 8, 0b10));
        assert!(!is_connected(&solid, 8, 7, 0b1));
        // digest: equal slices hash equal, different bytes differ.
        assert_eq!(digest(&solid), digest(&[0u8; 64]));
        assert_ne!(digest(&solid), digest(&split));
    }

    #[test]
    fn constraints_are_checked_before_any_bake() {
        let adj = trap();
        let empty = Constraints::default();
        assert_eq!(empty.validate(16, 16, &adj), Ok(()));
        let check = |rules: &Constraints| rules.validate(16, 16, &adj);
        // A rule that names a cell outside the map names that cell.
        let outside = Constraints {
            pinned: vec![(16, 0, 0)],
            ..Constraints::default()
        };
        assert_eq!(
            check(&outside),
            Err(WfcError::CellOutOfBounds { x: 16, z: 0 })
        );
        // A kind above kinds-1 names the cell the rule starts at.
        let too_high = Constraints {
            pinned: vec![(3, 4, 3)],
            ..Constraints::default()
        };
        assert_eq!(
            check(&too_high),
            Err(WfcError::KindOutOfRange {
                x: 3,
                z: 4,
                kind: 3
            })
        );
        // A block that ends before it starts holds no cell, so it is refused.
        let inverted = Constraints {
            forbidden: vec![(Rect::new(5, 5, 4, 9), 0)],
            ..Constraints::default()
        };
        assert_eq!(check(&inverted), Err(WfcError::RectInverted { x: 5, z: 5 }));
        let past_edge = Constraints {
            forbidden: vec![(Rect::new(5, 5, 16, 9), 0)],
            ..Constraints::default()
        };
        assert_eq!(
            check(&past_edge),
            Err(WfcError::CellOutOfBounds { x: 16, z: 9 })
        );
        // Connect needs two cells in the map and a non-zero in-range open mask.
        let reach = Constraints {
            connect: vec![((0, 0), (16, 0))],
            open: 0b1,
            ..Constraints::default()
        };
        assert_eq!(
            check(&reach),
            Err(WfcError::CellOutOfBounds { x: 16, z: 0 })
        );
        let closed = Constraints {
            connect: vec![((0, 0), (1, 0))],
            open: 0,
            ..Constraints::default()
        };
        assert_eq!(check(&closed), Err(WfcError::OpenMaskEmpty));
        let wide = Constraints {
            connect: vec![((0, 0), (1, 0))],
            open: 0b1000,
            ..Constraints::default()
        };
        assert_eq!(check(&wide), Err(WfcError::OpenMaskOutOfRange { kind: 3 }));
        // The bake runs the same checks before it allocates a wave.
        assert_eq!(
            bake_constrained(16, 16, &adj, 7, &outside),
            Err(WfcError::CellOutOfBounds { x: 16, z: 0 })
        );
        let over = MAX_WFC_SIDE + 1;
        assert_eq!(
            bake_constrained(over, 1, &adj, 7, &outside),
            Err(WfcError::TooLarge { side: over })
        );
    }

    #[test]
    fn a_rule_conflict_names_the_cell() {
        let adj = trap();
        // Two pins of different kinds on one cell intersect to nothing.
        let twice = Constraints {
            pinned: vec![(2, 3, 0), (2, 3, 1)],
            ..Constraints::default()
        };
        assert_eq!(
            bake_constrained(16, 16, &adj, 7, &twice),
            Err(WfcError::UnsatisfiableCell { x: 2, z: 3 })
        );
        // A pin inside a block that forbids the same kind is the same conflict.
        let blocked = Constraints {
            pinned: vec![(2, 3, 0)],
            forbidden: vec![(Rect::cell(2, 3), 0)],
            ..Constraints::default()
        };
        assert_eq!(
            bake_constrained(16, 16, &adj, 7, &blocked),
            Err(WfcError::UnsatisfiableCell { x: 2, z: 3 })
        );
        // A pin on the claimed path that is not an open kind names the path cell.
        let off_path = Constraints {
            pinned: vec![(5, 8, 2)],
            connect: vec![((2, 8), (13, 8))],
            open: 0b0011,
            ..Constraints::default()
        };
        assert_eq!(
            bake_constrained(16, 16, &adj, 7, &off_path),
            Err(WfcError::UnsatisfiableCell { x: 5, z: 8 })
        );
    }

    #[test]
    fn a_pin_the_table_refuses_is_a_contradiction_at_the_neighbour() {
        // Self-only kinds never touch each other, so these two pins cannot meet.
        let adj = self_only();
        let rules = Constraints {
            pinned: vec![(0, 0, 0), (1, 0, 1)],
            ..Constraints::default()
        };
        assert_eq!(
            bake_constrained(2, 1, &adj, 7, &rules),
            Err(WfcError::Contradiction { x: 1, z: 0 })
        );
    }

    #[test]
    fn pins_hold_and_a_block_never_holds_its_kind() {
        let adj = generous();
        let rules = Constraints {
            pinned: vec![(0, 0, 2), (7, 7, 0)],
            forbidden: vec![(Rect::new(2, 2, 5, 5), 1)],
            ..Constraints::default()
        };
        let cells = bake_constrained(8, 8, &adj, 21, &rules).expect("rules fit a generous table");
        assert_eq!(cells[index(0, 0, 8)], 2);
        assert_eq!(cells[index(7, 7, 8)], 0);
        for z in 2..=5u16 {
            for x in 2..=5u16 {
                assert_ne!(cells[index(x, z, 8)], 1, "block ({x}, {z}) stays clear");
            }
        }
        assert_neighbours_agree(&cells, 8, 8, &adj);
    }

    #[test]
    fn twenty_seeds_solve_at_least_eighteen_within_the_backtrack_cap() {
        let adjacency = trap();
        let rules = authored();
        let (width, height) = (16u16, 16u16);
        let mut solved = 0u32;
        let mut refused = Vec::new();
        let mut times = Vec::with_capacity(20);
        for seed in 0..20u64 {
            let start = std::time::Instant::now();
            let grid = bake_constrained(width, height, &adjacency, seed, &rules);
            times.push(start.elapsed());
            match grid {
                Ok(cells) => {
                    assert_rules_hold(&cells, width, height, &adjacency, &rules);
                    let again = bake_constrained(width, height, &adjacency, seed, &rules)
                        .expect("a solved seed solves again");
                    assert_eq!(cells, again, "seed {seed} bakes identical bytes x2");
                    assert_eq!(digest(&cells), digest(&again));
                    solved = solved.saturating_add(1);
                }
                Err(err) => refused.push((seed, err)),
            }
        }
        times.sort_unstable();
        let p50 = times.get(times.len() / 2).copied().unwrap_or_default();
        println!(
            "wfc: solved {solved}/20 within cap {MAX_BACKTRACKS}, p50 bake {p50:?}, refused {refused:?}"
        );
        assert!(
            solved >= 18,
            "solved {solved}/20 within the backtrack cap, p50 {p50:?}, refused {refused:?}"
        );
    }

    #[test]
    fn a_seed_past_the_cap_is_a_typed_refusal_not_a_partial_grid() {
        let adjacency = trap();
        let rules = authored();
        let (width, height) = (16u16, 16u16);
        let cells = usize::from(width) * usize::from(height);
        // The same seed solves under the real cap.
        assert!(bake_constrained(width, height, &adjacency, 0, &rules).is_ok());
        let mut masks = vec![kind_bits(adjacency.kinds()); cells];
        assert!(apply_constraints(&mut masks, width, height, &rules).expect("rules fit the map"));
        settle(&mut masks, &adjacency, width, height).expect("rules leave a live wave");
        // Under a tight cap the same seed refuses, and names the cap it hit.
        assert!(
            matches!(
                search(&mut masks, &adjacency, width, height, 0, 8),
                Err(WfcError::BacktrackCap { cap: 8, .. })
            ),
            "a cap of 8 branches refuses with the cap named"
        );
        // A refused search leaves no grid; the real cap returns a full one.
        let grid = bake_constrained(width, height, &adjacency, 0, &rules).expect("the cap solves");
        assert_eq!(grid.len(), cells);
    }
}
