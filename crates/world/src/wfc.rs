//! wfc: wave function collapse tile bake — kinds plus an adjacency table.
//! Source: web: mxgmn/WaveFunctionCollapse simple tiled model + krychu/wfc,
//! read 2026-10-19, ideas only, zero bytes vendored.
//! Owner: engine-world, bake-time only, never the tick or the sim hash.
//! Units: cells are `u16` grid steps, `x` fastest then `z`; kinds are indices
//! `0..kinds`; direction 0 `+x`, 1 `-x`, 2 `+z`, 3 `-z`.
//! Determinism: the same `(width, height, adjacency, seed)` always gives the
//! same bytes; one collapse per step draws from `splitmix64(seed ^ step)`,
//! integers only, so the collapse order never depends on a float entropy.
//! Errors: [`WfcError`] refuses a bad table and an emptied wave; a contradiction
//! is returned, never retried with another seed (the caller picks the seed).
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
        }
    }
}

impl std::error::Error for WfcError {}

/// Bake one map: `width * height` cells of kind indices, row-major, `x` fastest.
///
/// A contradiction is refused, never retried. Errors: [`WfcError::TooLarge`],
/// [`WfcError::Contradiction`].
pub fn bake(
    width: u16,
    height: u16,
    adjacency: &Adjacency,
    seed: u64,
) -> Result<Vec<u8>, WfcError> {
    if width > MAX_WFC_SIDE {
        return Err(WfcError::TooLarge { side: width });
    }
    if height > MAX_WFC_SIDE {
        return Err(WfcError::TooLarge { side: height });
    }
    let kinds = adjacency.kinds();
    let cells = usize::from(width) * usize::from(height);
    let mut masks = vec![kind_bits(kinds); cells];
    let mut step = 0u64;
    while let Some((x, z)) = next_cell(&masks, width, height) {
        let cell = index(x, z, width);
        let want = engine_core::splitmix64(seed ^ step);
        let kind = nth_kind(masks[cell], want, kinds).ok_or(WfcError::Contradiction { x, z })?;
        masks[cell] = 1u16 << kind;
        step = step.wrapping_add(1);
        propagate(&mut masks, adjacency, width, height, (x, z))?;
    }
    let mut out = Vec::with_capacity(cells);
    for &mask in &masks {
        out.push(lowest_bit(mask));
    }
    Ok(out)
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
        Some(start) => flood(cells, width, height, passable, start) == total,
    }
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

/// Shrink neighbour masks from `start` until the grid is arc-consistent.
/// Errors: [`WfcError::Contradiction`] at the cell that lost every kind.
fn propagate(
    masks: &mut [u16],
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
            masks[cell] = shrunk;
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

/// Passable cells reachable from `start` by 4-way steps, `start` included.
/// Units: cell steps in, cell count out.
fn flood(cells: &[u8], width: u16, height: u16, passable: u16, start: (u16, u16)) -> usize {
    let mut seen = vec![false; cells.len()];
    seen[index(start.0, start.1, width)] = true;
    let mut stack = vec![start];
    let mut reached = 0usize;
    while let Some((x, z)) = stack.pop() {
        reached += 1;
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
    reached
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
            for z in 0..16u16 {
                for x in 0..16u16 {
                    let kind = cells[index(x, z, 16)];
                    for dir in 0..DIRECTIONS {
                        let Some((nx, nz)) = step(x, z, dir, 16, 16) else {
                            continue;
                        };
                        let other = cells[index(nx, nz, 16)];
                        assert_ne!(
                            adj.allows(kind, dir) & (1u16 << other),
                            0,
                            "cell ({x},{z}) kind {kind} refuses kind {other} toward dir {dir}"
                        );
                    }
                }
            }
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
}
