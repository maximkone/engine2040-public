//! index3: integer-cm 3D index over scene boxes.
//! Patterns (EB-6): forge packs/scene SpatialBroadphase sweep shape,
//! SceneSolverOps measure-fix-verify, SceneSpatialQa fixtures — reimplemented
//! std-only, nothing copied, no dependency.
//! Owner: spatial crate, consumers the placement solver (clearance) and the
//! relation builder (pair candidates). Units: meters in, integer centimeters
//! inside; the only float touch is the shared `to_cm` input rounding, never
//! the digest.
//! Determinism: id-sorted builds, id tie-breaks, BTree order, FNV-1a over
//! canonical integer bytes. Errors: typed `IndexError`, never a panic.
//! Budget: build O(n), box query O(log n + k), pair phase one sweep pass.
//! R* vs grid (release, same pool; timing tests print every run): pairs at
//! 5k crowded go to the shared sweep (1.2 ms vs legacy 5.6-9.2 ms, ~4.5x;
//! grid-cells 2.8-5 ms, tree-native 2-3.7 ms); uniform queries at 10k are a
//! wash (grid p95 ~20-38 us, tree ~36-44 us); the 1000-crate tower slice
//! goes to the tree (3 us vs 10-19 us); bulk build goes to the grid.
//! [`Index3`] is the [`Rstar`]: it wins the defining stacking queries
//! and needs no cell tuning, while [`Grid3`] stays as the measured
//! contender with full equality proofs.
//! Incremental updates (salsa red-green shape, std-only): each mutating edit
//! bumps `revision` and rebuilds exactly the dirty ids' subtrees inside
//! `apply`, so every query observes the verified revision; `last_dirty`
//! records the rebuilt ids. Single-edit rebuild p95 < 2 ms at 10k objects.

use super::index::{boxes_of, to_cm, xyz_hit, Box3, XYZ_CELL_CM};
use super::Actor;
use engine_core::Fnv64;
use std::collections::BTreeMap;
use std::fmt;

/// Wire magic at the head of every index blob (`E2IX30`).
const IX3_MAGIC: [u8; 6] = *b"E2IX30";
/// Blob layout version; bump on any change, refuse anything else.
const IX3_VERSION: u16 = 1;
/// Largest entry count one decode accepts; larger blobs refuse loudly.
const IX3_MAX_ENTRIES: u32 = 1_000_000;
/// Digest tag so index bytes never alias another domain hash.
const IX3_TAG: &[u8] = b"e2index3:1";

/// Typed failure for byte IO and bad query shapes. Invalid input returns
/// `Err`, never a guess and never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// First six bytes are not `E2IX30`.
    BadMagic,
    /// Version field is not 1.
    BadVersion(u16),
    /// Fewer bytes than the header and entry count promise.
    Truncated,
    /// Entry count exceeds the decode cap.
    TooMany(u32),
    /// Query box has `min > max` on some axis.
    InvertedBox,
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "index3: bad magic, want E2IX30"),
            Self::BadVersion(v) => write!(f, "index3: bad version {v}, want 1"),
            Self::Truncated => write!(f, "index3: truncated blob"),
            Self::TooMany(n) => write!(f, "index3: entry count {n} past cap"),
            Self::InvertedBox => write!(f, "index3: query min > max"),
        }
    }
}

impl std::error::Error for IndexError {}

/// Field-wise box equality. `Box3` lives in the legacy index module and
/// gains no derives from this task; compare rows explicitly instead.
fn same_box(a: &Box3, b: &Box3) -> bool {
    a.id == b.id
        && a.min_x == b.min_x
        && a.max_x == b.max_x
        && a.min_y == b.min_y
        && a.max_y == b.max_y
        && a.min_z == b.min_z
        && a.max_z == b.max_z
}

fn same_opt(a: Option<Box3>, b: Option<Box3>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => same_box(&x, &y),
        (None, None) => true,
        _ => false,
    }
}
/// Strict XZ overlap (the relation candidate shape: contact needs XZ first).
fn hit_xz(a: &Box3, b: &Box3) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_z < b.max_z && b.min_z < a.max_z
}

/// Rows that index anything: known position, known size, positive extents.
/// Matches the relation-validity rule, so NaN and flat actors stay isolated.
fn live_row(a: &Actor) -> bool {
    super::index::pos_known(a)
        && super::index::size_known(a)
        && a.sx > 0.0
        && a.sy > 0.0
        && a.sz > 0.0
}

/// Boxes for live rows only, id-sorted. Degenerate rows index nothing, and
/// since a zero-volume box never strictly hits, pair sets are unchanged.
pub(crate) fn live_boxes(actors: &[Actor]) -> Vec<Box3> {
    let live: Vec<Actor> = actors.iter().filter(|a| live_row(a)).cloned().collect();
    let mut rows = boxes_of(&live);
    rows.sort_by_key(|b| b.id);
    rows
}

/// Canonical entry bytes: id-sorted `(id, 6 x i32 LE)` rows. Both index
/// flavors serialize the same entry set, so their blobs are byte-identical
/// and the digest never sees a float or a pointer.
fn encode_entries(boxes: &BTreeMap<u32, Box3>) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + boxes.len() * 28);
    out.extend_from_slice(&IX3_MAGIC);
    out.extend_from_slice(&IX3_VERSION.to_le_bytes());
    out.extend_from_slice(&(boxes.len() as u32).to_le_bytes());
    for (id, b) in boxes {
        out.extend_from_slice(&id.to_le_bytes());
        for v in [b.min_x, b.max_x, b.min_y, b.max_y, b.min_z, b.max_z] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

fn decode_entries(bytes: &[u8]) -> Result<BTreeMap<u32, Box3>, IndexError> {
    if bytes.len() < 12 || bytes[..6] != IX3_MAGIC {
        return Err(IndexError::BadMagic);
    }
    let version = u16::from_le_bytes([bytes[6], bytes[7]]);
    if version != IX3_VERSION {
        return Err(IndexError::BadVersion(version));
    }
    let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if count > IX3_MAX_ENTRIES {
        return Err(IndexError::TooMany(count));
    }
    let want = 12 + count as usize * 28;
    if bytes.len() < want {
        return Err(IndexError::Truncated);
    }
    let mut boxes = BTreeMap::new();
    for i in 0..count as usize {
        let off = 12 + i * 28;
        let id = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let mut v = [0i32; 6];
        for (k, slot) in v.iter_mut().enumerate() {
            let o = off + 4 + k * 4;
            *slot = i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        }
        // Last row wins on duplicate ids; builders sort unique ids anyway.
        boxes.insert(
            id,
            Box3 {
                id,
                min_x: v[0],
                max_x: v[1],
                min_y: v[2],
                max_y: v[3],
                min_z: v[4],
                max_z: v[5],
            },
        );
    }
    Ok(boxes)
}

fn digest_entries(boxes: &BTreeMap<u32, Box3>) -> u64 {
    let mut h = Fnv64::new();
    h.write(IX3_TAG);
    h.write_u64(boxes.len() as u64);
    for (id, b) in boxes {
        h.write_u32(*id);
        for v in [b.min_x, b.max_x, b.min_y, b.max_y, b.min_z, b.max_z] {
            h.write_i32(v);
        }
    }
    h.finish()
}

/// XZ adjacency pairs `(lo, hi)` within `gap_cm` over an entry set, sorted.
/// Shared by both index flavors.
fn adjacent_of(boxes: &BTreeMap<u32, Box3>, gap_cm: i32) -> Vec<(u32, u32)> {
    let sorted: Vec<Box3> = boxes.values().copied().collect();
    sweep_adjacent(&sorted, gap_cm)
}

/// Levels above an XZ point in integer cm over an entry set: ids covering
/// the point, ordered by `(min_y, id)`. Shared by both index flavors.
fn column_of(boxes: &BTreeMap<u32, Box3>, x_cm: i32, z_cm: i32) -> Vec<u32> {
    let mut hits: Vec<(i32, u32)> = Vec::new();
    for (id, b) in boxes {
        if b.min_x <= x_cm && x_cm <= b.max_x && b.min_z <= z_cm && z_cm <= b.max_z {
            hits.push((b.min_y, *id));
        }
    }
    hits.sort_unstable();
    hits.into_iter().map(|(_, id)| id).collect()
}

/// Highest top above an XZ point in integer cm over an entry set: the
/// maximum `max_y` covering the point, or `None` when the column is empty.
/// Shared by both index flavors. Integer cm only, id-sorted scan.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "column-top ships before the solver rest hunk consumes it"
    )
)]
pub(crate) fn column_top_cm(boxes: &BTreeMap<u32, Box3>, x_cm: i32, z_cm: i32) -> Option<i32> {
    let mut top: Option<i32> = None;
    for b in boxes.values() {
        if b.min_x <= x_cm && x_cm <= b.max_x && b.min_z <= z_cm && z_cm <= b.max_z {
            top = Some(top.map_or(b.max_y, |cur| cur.max(b.max_y)));
        }
    }
    top
}

/// Ids whose Y range strictly overlaps `[y_lo, y_hi]` over an entry set,
/// sorted; an inverted band is empty. Shared by both index flavors.
fn band_of(boxes: &BTreeMap<u32, Box3>, y_lo: i32, y_hi: i32) -> Vec<u32> {
    if y_lo > y_hi {
        return Vec::new();
    }
    let mut out: Vec<u32> = boxes
        .iter()
        .filter(|(_, b)| b.min_y < y_hi && y_lo < b.max_y)
        .map(|(id, _)| *id)
        .collect();
    out.sort_unstable();
    out
}

/// 3D sweep-and-prune over an id-sorted box slice: `(lo, hi)` pairs with
/// strict volume overlap, sorted. One sort plus a linear active-window
/// pass: the measured pair engine (see `timing_pair_phase_5k`), shared by
/// both flavors and the `pairs3` entry. Trees and grids accelerate
/// *selective* queries; bulk enumeration belongs to the sweep.
fn sweep_pairs3(sorted: &[Box3]) -> Vec<(u32, u32)> {
    let mut order: Vec<usize> = (0..sorted.len()).collect();
    order.sort_by_key(|&i| {
        (
            sorted[i].min_x,
            sorted[i].min_y,
            sorted[i].min_z,
            sorted[i].id,
        )
    });
    sweep_pairs3_ordered(sorted, &order)
}

/// Sweep core over a caller-provided position order. The caller must sort
/// by `(min_x, min_y, min_z, id)`; `boxes_of` already emits that key, so
/// the one-shot entry skips a second sort.
fn sweep_pairs3_ordered(sorted: &[Box3], order: &[usize]) -> Vec<(u32, u32)> {
    let mut active: Vec<usize> = Vec::new();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &i in order {
        let r = &sorted[i];
        active.retain(|&j| sorted[j].max_x > r.min_x);
        for &j in &active {
            let o = &sorted[j];
            // X overlap holds by window order (o.min_x <= r.min_x < r.max_x
            // needs r.max_x > o.min_x, which follows from the sort); test
            // the exact 3D hit, which rechecks X cheaply and exactly.
            if xyz_hit(o, r) {
                let (lo, hi) = if o.id < r.id {
                    (o.id, r.id)
                } else {
                    (r.id, o.id)
                };
                out.push((lo, hi));
            }
        }
        active.push(i);
    }
    // Each unordered pair is found once (position order decides); sort only.
    out.sort_unstable();
    out
}

/// XZ sweep over an id-sorted box slice: `(lo, hi)` pairs with strict XZ
/// overlap, sorted. Same semantics as the legacy `overlap_pairs`: the relation
/// strict-contact candidate set (Supports/OnTopOf always sit in it).
pub(crate) fn sweep_xz(sorted: &[Box3]) -> Vec<(u32, u32)> {
    let mut order: Vec<usize> = (0..sorted.len()).collect();
    order.sort_by_key(|&i| (sorted[i].min_x, sorted[i].min_z, sorted[i].id));
    let mut active: Vec<usize> = Vec::new();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for &i in &order {
        let r = &sorted[i];
        active.retain(|&j| sorted[j].max_x > r.min_x);
        for &j in &active {
            let o = &sorted[j];
            if hit_xz(o, r) {
                let (lo, hi) = if o.id < r.id {
                    (o.id, r.id)
                } else {
                    (r.id, o.id)
                };
                pairs.push((lo, hi));
            }
        }
        active.push(i);
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// XZ adjacency sweep: pairs with disjoint XZ interiors whose squared XZ
/// edge gap fits in `gap_cm` and whose Y ranges strictly overlap, sorted.
/// `gap_cm = 0` catches face-touching boxes (the NextTo phase); larger
/// gaps mirror the relation side-by-side limit. Negative gaps index nothing.
fn sweep_adjacent(sorted: &[Box3], gap_cm: i32) -> Vec<(u32, u32)> {
    if gap_cm < 0 {
        return Vec::new();
    }
    let gap = i64::from(gap_cm);
    let limit2 = gap.saturating_mul(gap);
    let mut order: Vec<usize> = (0..sorted.len()).collect();
    order.sort_by_key(|&i| (sorted[i].min_x, sorted[i].min_z, sorted[i].id));
    let mut active: Vec<usize> = Vec::new();
    let mut pairs: Vec<(u32, u32)> = Vec::new();
    for &i in &order {
        let r = &sorted[i];
        active.retain(|&j| {
            i64::from(sorted[j].max_x).saturating_add(gap) > i64::from(r.min_x).saturating_sub(gap)
        });
        for &j in &active {
            let o = &sorted[j];
            let ix = (i64::from(r.max_x.min(o.max_x)) - i64::from(r.min_x.max(o.min_x))).max(0);
            let iz = (i64::from(r.max_z.min(o.max_z)) - i64::from(r.min_z.max(o.min_z))).max(0);
            if ix > 0 && iz > 0 {
                continue;
            }
            if !(r.min_y < o.max_y && o.min_y < r.max_y) {
                continue;
            }
            let gx = (i64::from(r.min_x.max(o.min_x)) - i64::from(r.max_x.min(o.max_x))).max(0);
            let gz = (i64::from(r.min_z.max(o.min_z)) - i64::from(r.max_z.min(o.max_z))).max(0);
            if gx.saturating_mul(gx).saturating_add(gz.saturating_mul(gz)) <= limit2 {
                let (lo, hi) = if o.id < r.id {
                    (o.id, r.id)
                } else {
                    (r.id, o.id)
                };
                pairs.push((lo, hi));
            }
        }
        active.push(i);
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// Shared incremental-update plan: sorted removals plus id-sorted fresh
/// rows for the changed ids. Both flavors consume the same plan, so
/// rebuild-vs-incremental agrees pair-for-pair, byte-for-byte and
/// free-tile for free-tile (the free-space field rides the same dirty ids).
fn update_plan(changed: &[Actor], removed: &[u32]) -> (Vec<u32>, BTreeMap<u32, Box3>) {
    let mut drops: Vec<u32> = removed.to_vec();
    drops.sort_unstable();
    drops.dedup();
    let live: Vec<Actor> = changed.iter().filter(|a| live_row(a)).cloned().collect();
    let mut fresh: BTreeMap<u32, Box3> = BTreeMap::new();
    for b in boxes_of(&live) {
        fresh.insert(b.id, b);
    }
    (drops, fresh)
}

/// Shared edit record: one salsa generation per mutating edit. `dirty`
/// holds the ids whose subtrees this edit rebuilt (sorted below); a no-op
/// edit clears the last-dirty list and leaves the revision alone, so cached
/// probes keyed on `(digest, revision)` stay valid until the revision moves.
/// The same revision covers the free-space field below. Revisions and
/// digests never reach the bytes: decode restarts at 0.
fn note_edit(
    revision: &mut u64,
    rebuilds: &mut u64,
    last_dirty: &mut Vec<u32>,
    mut dirty: Vec<u32>,
) {
    dirty.sort_unstable();
    dirty.dedup();
    if dirty.is_empty() {
        last_dirty.clear();
        return;
    }
    *revision = revision.wrapping_add(1);
    *rebuilds = rebuilds.wrapping_add(1);
    *last_dirty = dirty;
}

/// XZ tiles covered by one box at nav pitch, row-major `(z, x)` order.
/// Integer math only; live boxes cover at least one tile. The world cap
/// bounds giants: 200 m at 1 m pitch is 40k tiles, never a wrap.
fn free_tiles_of(b: &Box3) -> Vec<(i32, i32)> {
    let pitch = super::navcheck::TILE_CM.max(1);
    let x0 = b.min_x.div_euclid(pitch);
    let x1 = (b.max_x - 1).div_euclid(pitch);
    let z0 = b.min_z.div_euclid(pitch);
    let z1 = (b.max_z - 1).div_euclid(pitch);
    let mut out = Vec::new();
    let mut z = z0;
    while z <= z1 {
        let mut x = x0;
        while x <= x1 {
            out.push((x, z));
            if x == x1 || out.len() >= 40_000 {
                break;
            }
            x = x.saturating_add(1);
        }
        if z == z1 || out.len() >= 40_000 {
            break;
        }
        z = z.saturating_add(1);
    }
    out.sort_unstable_by_key(|&(x, z)| (z, x));
    out.dedup();
    out
}

/// Full free-space oracle over an entry set: tile to covering-box count.
/// Same tiles give same counts twice; empty input maps nothing.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "free oracle ships before the reachability bake consumes it"
    )
)]
fn rebuild_free(boxes: &BTreeMap<u32, Box3>) -> BTreeMap<(i32, i32), u32> {
    let mut map: BTreeMap<(i32, i32), u32> = BTreeMap::new();
    for b in boxes.values() {
        for t in free_tiles_of(b) {
            let count = map.get(&t).copied().unwrap_or(0);
            map.insert(t, count.saturating_add(1));
        }
    }
    map
}

/// Count one box footprint into the field.
fn free_add(map: &mut BTreeMap<(i32, i32), u32>, tiles: &[(i32, i32)]) {
    for t in tiles {
        let count = map.get(t).copied().unwrap_or(0);
        map.insert(*t, count.saturating_add(1));
    }
}

/// Uncount one box footprint; empty tiles leave the map.
fn free_remove(map: &mut BTreeMap<(i32, i32), u32>, tiles: &[(i32, i32)]) {
    for t in tiles {
        if let Some(count) = map.get(t).copied() {
            if count <= 1 {
                map.remove(t);
            } else {
                map.insert(*t, count - 1);
            }
        }
    }
}

/// Nav tile for one index box center in integer cm. The center recovers the
/// actor position, then the nav pitch maps it to a tile. Integer math only.
fn tile_of_box(b: &Box3) -> (i32, i32) {
    let cx = (i64::from(b.min_x) + i64::from(b.max_x)) / 2;
    let cz = (i64::from(b.min_z) + i64::from(b.max_z)) / 2;
    let pitch = i64::from(super::navcheck::TILE_CM);
    let tx = cx
        .div_euclid(pitch)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let tz = cz
        .div_euclid(pitch)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    // Clamped above, so the conversion holds; the fallback never fires.
    let tx = i32::try_from(tx).unwrap_or(0);
    let tz = i32::try_from(tz).unwrap_or(0);
    (tx, tz)
}

/// Dirty nav tiles for a box map plus dirty ids: center tile per live box,
/// row-major `(z, x)` order, deduped, empty when clean. Missing ids leave
/// no tile. Never reorders the index. Integer math only.
fn dirty_tiles_of(boxes: &BTreeMap<u32, Box3>, dirty: &[u32]) -> Vec<(i32, i32)> {
    let mut out = Vec::with_capacity(dirty.len());
    for id in dirty {
        if let Some(b) = boxes.get(id) {
            out.push(tile_of_box(b));
        }
    }
    out.sort_unstable_by_key(|&(x, z)| (z, x));
    out.dedup();
    out
}

/// Fixed row-major bytes for a dirty-tile set: `u32 LE` count plus
/// `(x LE, z LE)` rows in `(z, x)` order, deduped. Same tiles give same
/// bytes twice; empty input encodes a zero count. Integer math only.
pub fn dirty_tile_bytes(tiles: &[(i32, i32)]) -> Vec<u8> {
    let mut sorted: Vec<(i32, i32)> = tiles.to_vec();
    sorted.sort_unstable_by_key(|&(x, z)| (z, x));
    sorted.dedup();
    let count = u32::try_from(sorted.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(4 + sorted.len() * 8);
    out.extend_from_slice(&count.to_le_bytes());
    for (x, z) in sorted {
        out.extend_from_slice(&x.to_le_bytes());
        out.extend_from_slice(&z.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Grid3: persistent uniform-grid contender (cell = XYZ_CELL_CM, clamped span)
// ---------------------------------------------------------------------------

/// Persistent integer-cm uniform-grid 3D index. Build once from an actor
/// snapshot, then query and incrementally update without rebuilding.
/// Boxes wider than [`GRID_MAX_SPAN`] cells on any axis skip the cells and
/// live on the giants list, tested linearly: the grid never drops giant
/// pairs the way a clamped cell fan-out would.
#[derive(Debug, Clone)]
pub struct Grid3 {
    boxes: BTreeMap<u32, Box3>,
    cells: BTreeMap<(i32, i32, i32), Vec<u32>>,
    giants: Vec<u32>,
    cell: i32,
    revision: u64,
    rebuilds: u64,
    last_dirty: Vec<u32>,
}

/// Cell-span cap per axis before a box counts as a giant. Giants stay
/// exact via the overflow list instead of a lossy clamp.
const GRID_MAX_SPAN: i32 = 64;

impl Grid3 {
    /// Build over live actor rows; unknown or degenerate rows index nothing.
    pub fn build(actors: &[Actor]) -> Self {
        let boxes: BTreeMap<u32, Box3> =
            live_boxes(actors).into_iter().map(|b| (b.id, b)).collect();
        let mut grid = Self {
            boxes,
            cells: BTreeMap::new(),
            giants: Vec::new(),
            cell: XYZ_CELL_CM.max(1),
            revision: 0,
            rebuilds: 0,
            last_dirty: Vec::new(),
        };
        grid.reindex_all();
        grid
    }

    /// Monotonic edit revision: 0 at build/decode, +1 per mutating edit.
    /// `apply` rebuilds dirty cells synchronously, so every query observes
    /// the verified revision; editors can skip probes while this holds.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Count of mutating edits absorbed since build/decode.
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Sorted ids whose cells the last edit rebuilt; empty after a no-op.
    pub fn last_dirty(&self) -> Vec<u32> {
        self.last_dirty.clone()
    }

    /// Nav tiles touched by the last edit: center tile per rebuilt id,
    /// row-major `(z, x)` order, deduped, empty when clean. Integer math
    /// only; removals leave no tile. Never reorders the index.
    /// Determinism: same boxes give same tiles twice.
    pub fn dirty_tiles(&self) -> Vec<(i32, i32)> {
        dirty_tiles_of(&self.boxes, &self.last_dirty)
    }

    fn is_giant(&self, b: &Box3) -> bool {
        b.max_x.div_euclid(self.cell) - b.min_x.div_euclid(self.cell) > GRID_MAX_SPAN
            || b.max_y.div_euclid(self.cell) - b.min_y.div_euclid(self.cell) > GRID_MAX_SPAN
            || b.max_z.div_euclid(self.cell) - b.min_z.div_euclid(self.cell) > GRID_MAX_SPAN
    }

    fn cells_of(&self, b: &Box3) -> Vec<(i32, i32, i32)> {
        let x0 = b.min_x.div_euclid(self.cell);
        let x1 = (b.max_x - 1).div_euclid(self.cell);
        let y0 = b.min_y.div_euclid(self.cell);
        let y1 = (b.max_y - 1).div_euclid(self.cell);
        let z0 = b.min_z.div_euclid(self.cell);
        let z1 = (b.max_z - 1).div_euclid(self.cell);
        let mut out = Vec::new();
        // Spans are giant-capped by construction; the loops stay bounded.
        for cx in x0..=x1.min(x0 + GRID_MAX_SPAN) {
            for cy in y0..=y1.min(y0 + GRID_MAX_SPAN) {
                for cz in z0..=z1.min(z0 + GRID_MAX_SPAN) {
                    out.push((cx, cy, cz));
                }
            }
        }
        out
    }

    fn reindex_all(&mut self) {
        self.cells.clear();
        self.giants.clear();
        let ids: Vec<(u32, Box3)> = self.boxes.iter().map(|(id, b)| (*id, *b)).collect();
        for (id, b) in &ids {
            if self.is_giant(b) {
                self.giants.push(*id);
                continue;
            }
            for c in self.cells_of(b) {
                self.cells.entry(c).or_default().push(*id);
            }
        }
        self.giants.sort_unstable();
        for ids in self.cells.values_mut() {
            ids.sort_unstable();
        }
    }

    fn detach(&mut self, id: u32) {
        let Some(old) = self.boxes.get(&id).copied() else {
            return;
        };
        if self.giants.contains(&id) {
            self.giants.retain(|kept| *kept != id);
            return;
        }
        for c in self.cells_of(&old) {
            if let Some(ids) = self.cells.get_mut(&c) {
                ids.retain(|kept| *kept != id);
                if ids.is_empty() {
                    self.cells.remove(&c);
                }
            }
        }
    }

    fn attach(&mut self, b: Box3) {
        if self.is_giant(&b) {
            if self.giants.binary_search(&b.id).is_err() {
                self.giants.push(b.id);
                self.giants.sort_unstable();
            }
            self.boxes.insert(b.id, b);
            return;
        }
        for c in self.cells_of(&b) {
            let ids = self.cells.entry(c).or_default();
            if ids.binary_search(&b.id).is_err() {
                ids.push(b.id);
                ids.sort_unstable();
            }
        }
        self.boxes.insert(b.id, b);
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    /// True when no box is indexed.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// Strict 3D overlap pairs `(lo, hi)`, sorted. Uses the shared sweep
    /// enumerator over the entry set: matches the legacy `xyz_pairs` set
    /// exactly on the same input. See `pairs_cells` for the native grid
    /// enumeration kept for the contender comparison.
    pub fn pairs(&self) -> Vec<(u32, u32)> {
        let sorted: Vec<Box3> = self.boxes.values().copied().collect();
        sweep_pairs3(&sorted)
    }

    /// Native grid enumeration: per-cell candidate pairs plus giant
    /// cross-products, with exact hit tests, sorted and deduped. Same set
    /// as `pairs`; slower on crowded scenes (duplicate candidates plus map
    /// hops). Kept as the grid side of the R*-vs-grid numbers.
    pub fn pairs_cells(&self) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for ids in self.cells.values() {
            for (n, &i) in ids.iter().enumerate() {
                for &j in &ids[n + 1..] {
                    let (a, b) = (&self.boxes[&i], &self.boxes[&j]);
                    if xyz_hit(a, b) {
                        let (lo, hi) = if a.id < b.id {
                            (a.id, b.id)
                        } else {
                            (b.id, a.id)
                        };
                        out.push((lo, hi));
                    }
                }
            }
        }
        for &g in &self.giants {
            for (id, b) in &self.boxes {
                if *id == g {
                    continue;
                }
                if xyz_hit(&self.boxes[&g], b) {
                    let (lo, hi) = if g < *id { (g, *id) } else { (*id, g) };
                    out.push((lo, hi));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Strict XZ overlap pairs `(lo, hi)`, sorted. The relation strict-contact
    /// candidate set: every Supports/OnTopOf edge over live actors sits in
    /// it. Touching and near pairs ride [`adjacent_xz`](Self::adjacent_xz).
    pub fn pairs_xz(&self) -> Vec<(u32, u32)> {
        let sorted: Vec<Box3> = self.boxes.values().copied().collect();
        sweep_xz(&sorted)
    }

    /// XZ adjacency pairs `(lo, hi)` within `gap_cm`, sorted: disjoint XZ
    /// interiors, squared edge gap within range, Y ranges overlapping. The
    /// NextTo phase consumes this with the relation side-by-side limit.
    pub fn adjacent_xz(&self, gap_cm: i32) -> Vec<(u32, u32)> {
        adjacent_of(&self.boxes, gap_cm)
    }

    /// Ids of boxes strictly overlapping the query box, sorted. Inverted
    /// boxes return `Err`, never an empty pass that hides a caller bug.
    pub fn query_box(&self, min: [i32; 3], max: [i32; 3]) -> Result<Vec<u32>, IndexError> {
        if min[0] > max[0] || min[1] > max[1] || min[2] > max[2] {
            return Err(IndexError::InvertedBox);
        }
        let probe = Box3 {
            id: u32::MAX,
            min_x: min[0],
            max_x: max[0],
            min_y: min[1],
            max_y: max[1],
            min_z: min[2],
            max_z: max[2],
        };
        let mut seen: Vec<u32> = Vec::new();
        let x0 = min[0].div_euclid(self.cell);
        let x1 = (max[0] - 1).div_euclid(self.cell);
        let y0 = min[1].div_euclid(self.cell);
        let y1 = (max[1] - 1).div_euclid(self.cell);
        let z0 = min[2].div_euclid(self.cell);
        let z1 = (max[2] - 1).div_euclid(self.cell);
        // Probe spans clamp like entries; giants ride the overflow list.
        for cx in x0..=x1.min(x0 + GRID_MAX_SPAN) {
            for cy in y0..=y1.min(y0 + GRID_MAX_SPAN) {
                for cz in z0..=z1.min(z0 + GRID_MAX_SPAN) {
                    if let Some(ids) = self.cells.get(&(cx, cy, cz)) {
                        for &id in ids {
                            if xyz_hit(&self.boxes[&id], &probe) {
                                seen.push(id);
                            }
                        }
                    }
                }
            }
        }
        for &g in &self.giants {
            if xyz_hit(&self.boxes[&g], &probe) {
                seen.push(g);
            }
        }
        seen.sort_unstable();
        seen.dedup();
        Ok(seen)
    }

    /// Levels above an XZ point in integer cm: ids covering the point,
    /// ordered by `(min_y, id)`. Floors, stairs and overhangs read bottom-up.
    pub fn column(&self, x_cm: i32, z_cm: i32) -> Vec<u32> {
        column_of(&self.boxes, x_cm, z_cm)
    }

    /// Vertical band query: ids whose Y range strictly overlaps
    /// `[y_lo, y_hi]`, sorted. Siege-wall and single-floor isolation reads.
    pub fn band(&self, y_lo: i32, y_hi: i32) -> Vec<u32> {
        band_of(&self.boxes, y_lo, y_hi)
    }

    /// True when `probe` strictly overlaps any indexed box except
    /// `ignore_id`. The solver clearance hook: a solved placement must read
    /// `false` here against the scene-minus-object index.
    pub fn overlaps_any(&self, probe: &Box3, ignore_id: u32) -> bool {
        for (id, b) in &self.boxes {
            if *id == ignore_id {
                continue;
            }
            if xyz_hit(b, probe) {
                return true;
            }
        }
        false
    }

    /// Incremental update: `changed` rows upsert by id (unknown or
    /// degenerate rows delete), `removed` ids delete. Missing ids are a
    /// silent no-op. Only dirty cells detach and reattach, then the edit
    /// record stamps one revision for the whole batch. The result matches
    /// a full rebuild byte-for-byte.
    pub fn apply(&mut self, changed: &[Actor], removed: &[u32]) {
        let (drops, fresh) = update_plan(changed, removed);
        let mut changed_ids: Vec<u32> = changed.iter().map(|a| a.id).collect();
        changed_ids.sort_unstable();
        changed_ids.dedup();
        let mut dirty: Vec<u32> = Vec::new();
        for id in drops {
            self.detach(id);
            if self.boxes.remove(&id).is_some() {
                dirty.push(id);
            }
        }
        for id in changed_ids {
            match fresh.get(&id).copied() {
                Some(next) => {
                    if !same_opt(self.boxes.get(&id).copied(), Some(next)) {
                        self.detach(id);
                        self.attach(next);
                        dirty.push(id);
                    }
                }
                None => {
                    self.detach(id);
                    if self.boxes.remove(&id).is_some() {
                        dirty.push(id);
                    }
                }
            }
        }
        note_edit(
            &mut self.revision,
            &mut self.rebuilds,
            &mut self.last_dirty,
            dirty,
        );
    }

    /// Deterministic bytes: header plus id-sorted entry rows. Two builds
    /// over the same actors, and rebuild-vs-incremental, are byte-identical.
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_entries(&self.boxes)
    }

    /// Decode bytes from [`to_bytes`](Self::to_bytes); rebuilds the cells.
    /// Bad magic, version, truncation and past-cap counts refuse loudly.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IndexError> {
        let boxes = decode_entries(bytes)?;
        let mut grid = Self {
            boxes,
            cells: BTreeMap::new(),
            giants: Vec::new(),
            cell: XYZ_CELL_CM.max(1),
            revision: 0,
            rebuilds: 0,
            last_dirty: Vec::new(),
        };
        grid.reindex_all();
        Ok(grid)
    }

    /// Order-invariant FNV-1a digest over integer entry bytes. No float,
    /// no pointer, no build order reaches it.
    pub fn digest(&self) -> u64 {
        digest_entries(&self.boxes)
    }
}

// ---------------------------------------------------------------------------
// Rstar: integer-cm R*-tree contender (Guttman/Beckmann pattern, std-only)
// ---------------------------------------------------------------------------

/// Leaf capacity: shallow trees at 10k entries, cheap linear scans inside.
const RSTAR_MAX: usize = 8;
/// Minimum fill after a split (40% of capacity, Guttman floor).
const RSTAR_MIN: usize = 3;
/// Forced-reinsert count on first overflow per level (Beckmann ~30%).
const RSTAR_REINSERT: usize = 2;

/// Integer-cm minimum bounding rectangle. Volumes use saturating i64 so
/// degenerate giants refuse to wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mbr {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    min_z: i32,
    max_z: i32,
}

impl Mbr {
    fn of(b: &Box3) -> Self {
        Self {
            min_x: b.min_x,
            max_x: b.max_x,
            min_y: b.min_y,
            max_y: b.max_y,
            min_z: b.min_z,
            max_z: b.max_z,
        }
    }

    fn volume(&self) -> i64 {
        let dx = (i64::from(self.max_x) - i64::from(self.min_x)).max(0);
        let dy = (i64::from(self.max_y) - i64::from(self.min_y)).max(0);
        let dz = (i64::from(self.max_z) - i64::from(self.min_z)).max(0);
        dx.saturating_mul(dy).saturating_mul(dz)
    }

    /// Half surface area: the Beckmann margin criterion, integer-exact.
    fn margin(&self) -> i64 {
        let dx = (i64::from(self.max_x) - i64::from(self.min_x)).max(0);
        let dy = (i64::from(self.max_y) - i64::from(self.min_y)).max(0);
        let dz = (i64::from(self.max_z) - i64::from(self.min_z)).max(0);
        dx.saturating_mul(dy)
            .saturating_add(dy.saturating_mul(dz))
            .saturating_add(dz.saturating_mul(dx))
    }

    fn combine(&self, other: &Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            max_x: self.max_x.max(other.max_x),
            min_y: self.min_y.min(other.min_y),
            max_y: self.max_y.max(other.max_y),
            min_z: self.min_z.min(other.min_z),
            max_z: self.max_z.max(other.max_z),
        }
    }

    fn enlargement(&self, other: &Self) -> i64 {
        self.combine(other).volume().saturating_sub(self.volume())
    }

    fn overlap(&self, other: &Self) -> i64 {
        let dx = (i64::from(self.max_x.min(other.max_x)) - i64::from(self.min_x.max(other.min_x)))
            .max(0);
        let dy = (i64::from(self.max_y.min(other.max_y)) - i64::from(self.min_y.max(other.min_y)))
            .max(0);
        let dz = (i64::from(self.max_z.min(other.max_z)) - i64::from(self.min_z.max(other.min_z)))
            .max(0);
        dx.saturating_mul(dy).saturating_mul(dz)
    }

    fn hits(&self, other: &Self) -> bool {
        self.min_x < other.max_x
            && other.min_x < self.max_x
            && self.min_y < other.max_y
            && other.min_y < self.max_y
            && self.min_z < other.max_z
            && other.min_z < self.max_z
    }

    fn center2(&self) -> (i64, i64, i64) {
        (
            i64::from(self.min_x) + i64::from(self.max_x),
            i64::from(self.min_y) + i64::from(self.max_y),
            i64::from(self.min_z) + i64::from(self.max_z),
        )
    }
}

/// One tree entry: a box plus the child node index (`u32::MAX` on leaves).
/// Inner slots carry a copy of their child MBR as the box, kept in sync.
#[derive(Debug, Clone, Copy)]
struct Rentry {
    bx: Box3,
    child: u32,
}

#[derive(Debug, Clone)]
struct Rnode {
    mbr: Mbr,
    entries: Vec<Rentry>,
    leaf: bool,
    level: usize,
}

/// Exact union over leaf entry boxes.
fn leaf_mbr(entries: &[Rentry]) -> Option<Mbr> {
    let mut mbr: Option<Mbr> = None;
    for e in entries {
        let m = Mbr::of(&e.bx);
        mbr = Some(match mbr {
            Some(cur) => cur.combine(&m),
            None => m,
        });
    }
    mbr
}

/// Exact union over child MBR snapshots carried by inner slots.
fn inner_mbr(entries: &[Rentry]) -> Option<Mbr> {
    let mut mbr: Option<Mbr> = None;
    for e in entries {
        let m = Mbr {
            min_x: e.bx.min_x,
            max_x: e.bx.max_x,
            min_y: e.bx.min_y,
            max_y: e.bx.max_y,
            min_z: e.bx.min_z,
            max_z: e.bx.max_z,
        };
        mbr = Some(match mbr {
            Some(cur) => cur.combine(&m),
            None => m,
        });
    }
    mbr
}

fn slot_of(child: usize, m: &Mbr) -> Rentry {
    Rentry {
        bx: Box3 {
            id: u32::MAX,
            min_x: m.min_x,
            max_x: m.max_x,
            min_y: m.min_y,
            max_y: m.max_y,
            min_z: m.min_z,
            max_z: m.max_z,
        },
        child: child as u32,
    }
}

/// Insert outcome: `Done` carries the sibling index on split, `Retry`
/// carries reinsert requests for the top-level queue.
enum InsertOut {
    Done(Option<usize>),
    Retry(Vec<(Rentry, usize)>),
}

/// Integer-cm R\*-tree over scene boxes: Guttman disciplines with the
/// Beckmann split (margin-choice axis, overlap-choice position) and one
/// forced reinsert per level per insert. Reimplemented std-only from the
/// published pattern; nothing vendored. All tie-breaks fall back to entry
/// id, so the same id set builds the same tree on every target.
#[derive(Debug, Clone)]
pub struct Rstar {
    nodes: Vec<Rnode>,
    root: usize,
    boxes: BTreeMap<u32, Box3>,
    revision: u64,
    rebuilds: u64,
    last_dirty: Vec<u32>,
    /// Incremental free-space field: XZ tile to covering-box count at
    /// nav pitch. Placement broadphase and reachability read it; the
    /// full rebuild is the oracle.
    free: BTreeMap<(i32, i32), u32>,
}

impl Rstar {
    fn empty_root() -> (Vec<Rnode>, usize) {
        let seed = Mbr {
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
            min_z: 0,
            max_z: 0,
        };
        (
            vec![Rnode {
                mbr: seed,
                entries: Vec::new(),
                leaf: true,
                level: 0,
            }],
            0,
        )
    }

    /// Build over live actor rows, inserted id-sorted. The session record
    /// starts at revision 0; structural inserts during the build stamp none.
    pub fn build(actors: &[Actor]) -> Self {
        let (nodes, root) = Self::empty_root();
        let mut tree = Self {
            nodes,
            root,
            boxes: BTreeMap::new(),
            revision: 0,
            rebuilds: 0,
            last_dirty: Vec::new(),
            free: BTreeMap::new(),
        };
        for b in live_boxes(actors) {
            tree.insert_entry(b);
        }
        tree
    }

    /// Monotonic edit revision: 0 at build/decode, +1 per mutating edit.
    /// `apply` rebuilds dirty subtrees synchronously, so every query observes
    /// the verified revision; editors can skip probes while this holds.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Count of mutating edits absorbed since build/decode.
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }

    /// Sorted ids whose subtrees the last edit rebuilt; empty after a no-op.
    pub fn last_dirty(&self) -> Vec<u32> {
        self.last_dirty.clone()
    }

    /// Nav tiles touched by the last edit: center tile per rebuilt id,
    /// row-major `(z, x)` order, deduped, empty when clean. Integer math
    /// only; removals leave no tile. Never reorders the index.
    /// Determinism: same boxes give same tiles twice.
    pub fn dirty_tiles(&self) -> Vec<(i32, i32)> {
        dirty_tiles_of(&self.boxes, &self.last_dirty)
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    /// True when no box is indexed.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    /// Tree height in nodes (1 = leaf-only root). Bounded per id set.
    pub fn height(&self) -> usize {
        self.nodes[self.root].level + 1
    }

    /// R* subtree choice: leaf-bound children minimize overlap growth,
    /// volume growth, volume; higher levels minimize volume growth first.
    /// Final tie-break is slot order, which is insertion-deterministic.
    fn choose_child(&self, idx: usize, m: &Mbr) -> usize {
        let node = &self.nodes[idx];
        debug_assert!(!node.leaf);
        let leaf_level = node.level == 1;
        let mut best = 0usize;
        let mut best_key: Option<(i64, i64, i64)> = None;
        for (pos, e) in node.entries.iter().enumerate() {
            let child = &self.nodes[e.child as usize];
            let growth = child.mbr.enlargement(m);
            let key = if leaf_level {
                let combined = child.mbr.combine(m);
                let overlap_now: i64 = node
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(q, _)| *q != pos)
                    .map(|(_, o)| child.mbr.overlap(&self.nodes[o.child as usize].mbr))
                    .fold(0i64, |a, b| a.saturating_add(b));
                let overlap_next: i64 = node
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(q, _)| *q != pos)
                    .map(|(_, o)| combined.overlap(&self.nodes[o.child as usize].mbr))
                    .fold(0i64, |a, b| a.saturating_add(b));
                (
                    overlap_next.saturating_sub(overlap_now),
                    growth,
                    child.mbr.volume(),
                )
            } else {
                (0, growth, child.mbr.volume())
            };
            if best_key.is_none_or(|cur| key < cur) {
                best_key = Some(key);
                best = pos;
            }
        }
        best
    }

    /// R* split: the axis with the smallest summed margin wins, then the
    /// cut with the smallest overlap (ties: smallest total volume, then
    /// the first cut). Fixed iteration order keeps it deterministic.
    fn split(entries: Vec<Rentry>) -> (Vec<Rentry>, Vec<Rentry>) {
        let total = entries.len();
        debug_assert!(total > RSTAR_MIN);
        let lo = RSTAR_MIN;
        let hi = total - RSTAR_MIN;
        let mut ordered = entries;
        let mut best_axis = 0usize;
        let mut best_margin: Option<i64> = None;
        for axis in 0..3 {
            for by_max in [false, true] {
                sort_for_split(&mut ordered, axis, by_max);
                let mut margin_sum: i64 = 0;
                for k in lo..=hi {
                    let f = group_mbr(&ordered[..k]);
                    let s = group_mbr(&ordered[k..]);
                    margin_sum = margin_sum
                        .saturating_add(f.margin())
                        .saturating_add(s.margin());
                }
                if best_margin.is_none_or(|cur| margin_sum < cur) {
                    best_margin = Some(margin_sum);
                    best_axis = if by_max { axis + 3 } else { axis };
                }
            }
        }
        sort_for_split(&mut ordered, best_axis % 3, best_axis >= 3);
        let mut best_k = lo;
        let mut best_key: Option<(i64, i64)> = None;
        for k in lo..=hi {
            let f = group_mbr(&ordered[..k]);
            let s = group_mbr(&ordered[k..]);
            let key = (f.overlap(&s), f.volume().saturating_add(s.volume()));
            if best_key.is_none_or(|cur| key < cur) {
                best_key = Some(key);
                best_k = k;
            }
        }
        let second = ordered.split_off(best_k);
        (ordered, second)
    }

    fn refit(&mut self, idx: usize) {
        let node = &self.nodes[idx];
        let mbr = if node.leaf {
            leaf_mbr(&node.entries)
        } else {
            inner_mbr(&node.entries)
        };
        if let Some(m) = mbr {
            self.nodes[idx].mbr = m;
        }
    }

    /// Full insert path for one entry at `level`, growing the root on split.
    /// Reinsert requests ride an explicit queue instead of recursing, so a
    /// reinsert split can never invalidate a slot position held upstream.
    fn insert_level(&mut self, entry: Rentry, level: usize, reinserted: &mut [bool]) {
        let mut pending = vec![(entry, level)];
        while let Some((e, l)) = pending.pop() {
            match self.insert_rec(self.root, e, l, reinserted) {
                InsertOut::Done(sib) => {
                    if let Some(sib) = sib {
                        let left_mbr = self.nodes[self.root].mbr;
                        let right_mbr = self.nodes[sib].mbr;
                        let root_level = self.nodes[self.root].level + 1;
                        self.nodes.push(Rnode {
                            mbr: left_mbr.combine(&right_mbr),
                            entries: vec![slot_of(self.root, &left_mbr), slot_of(sib, &right_mbr)],
                            leaf: false,
                            level: root_level,
                        });
                        self.root = self.nodes.len() - 1;
                    }
                }
                InsertOut::Retry(mut more) => {
                    more.sort_by_key(|(e, _)| e.bx.id);
                    pending.extend(more.into_iter().rev());
                }
            }
        }
    }

    /// Insert with upsert semantics: a live id is replaced, keeping one row
    /// per id so incremental updates stay total. An identical reinsert is a
    /// quiet no-op (no revision); any structural change stamps one revision
    /// for exactly this id's subtrees.
    pub fn insert(&mut self, bx: Box3) {
        if same_opt(self.boxes.get(&bx.id).copied(), Some(bx)) {
            self.last_dirty.clear();
            return;
        }
        self.insert_entry(bx);
        note_edit(
            &mut self.revision,
            &mut self.rebuilds,
            &mut self.last_dirty,
            vec![bx.id],
        );
    }

    /// Structural upsert without the session record: build, decode and
    /// batch-apply paths stamp their own single revision per edit instead.
    fn insert_entry(&mut self, bx: Box3) {
        if self.boxes.contains_key(&bx.id) {
            self.remove_entry(bx.id);
        }
        self.boxes.insert(bx.id, bx);
        free_add(&mut self.free, &free_tiles_of(&bx));
        let mut reinserted = vec![false; 32];
        self.insert_level(
            Rentry {
                bx,
                child: u32::MAX,
            },
            0,
            &mut reinserted,
        );
    }

    /// Recursive insert. `Done` carries the sibling index on split; `Retry`
    /// carries reinsert requests that the top-level queue drains, so this
    /// function never re-enters the tree mid-path.
    fn insert_rec(
        &mut self,
        idx: usize,
        entry: Rentry,
        level: usize,
        reinserted: &mut [bool],
    ) -> InsertOut {
        if self.nodes[idx].leaf {
            debug_assert_eq!(self.nodes[idx].level, level);
            let m = Mbr::of(&entry.bx);
            self.nodes[idx].entries.push(entry);
            self.nodes[idx].mbr = self.nodes[idx].mbr.combine(&m);
            if self.nodes[idx].entries.len() > RSTAR_MAX {
                return self.handle_overflow(idx, level, reinserted);
            }
            return InsertOut::Done(None);
        }
        if self.nodes[idx].level == level {
            // Home level for a reinserted inner slot: rehang the subtree
            // here instead of descending past it to a leaf.
            debug_assert_ne!(entry.child, u32::MAX);
            self.nodes[idx].entries.push(entry);
            self.refit(idx);
            if self.nodes[idx].entries.len() > RSTAR_MAX {
                return self.handle_overflow(idx, level, reinserted);
            }
            return InsertOut::Done(None);
        }
        let m = Mbr::of(&entry.bx);
        let pos = self.choose_child(idx, &m);
        let child = self.nodes[idx].entries[pos].child as usize;
        match self.insert_rec(child, entry, level, reinserted) {
            InsertOut::Retry(more) => {
                // The child already refit itself; refresh and propagate.
                let child_mbr = self.nodes[child].mbr;
                self.nodes[idx].entries[pos] = slot_of(child, &child_mbr);
                self.refit(idx);
                return InsertOut::Retry(more);
            }
            InsertOut::Done(sib) => {
                if let Some(sib) = sib {
                    let sib_mbr = self.nodes[sib].mbr;
                    self.nodes[idx].entries.push(slot_of(sib, &sib_mbr));
                }
            }
        }
        let child_mbr = self.nodes[child].mbr;
        self.nodes[idx].entries[pos] = slot_of(child, &child_mbr);
        self.refit(idx);
        if self.nodes[idx].entries.len() > RSTAR_MAX {
            let node_level = self.nodes[idx].level;
            return self.handle_overflow(idx, node_level, reinserted);
        }
        InsertOut::Done(None)
    }

    fn handle_overflow(&mut self, idx: usize, level: usize, reinserted: &mut [bool]) -> InsertOut {
        let is_root = idx == self.root;
        if !is_root && level < reinserted.len() && !reinserted[level] {
            reinserted[level] = true;
            // Farthest-from-center first, id tie-break: Beckmann reinsert.
            let center = self.nodes[idx].mbr.center2();
            let mut order: Vec<(i64, u32, usize)> = self.nodes[idx]
                .entries
                .iter()
                .enumerate()
                .map(|(pos, e)| {
                    let c = Mbr::of(&e.bx).center2();
                    let dx = c.0 - center.0;
                    let dy = c.1 - center.1;
                    let dz = c.2 - center.2;
                    let d = dx
                        .saturating_mul(dx)
                        .saturating_add(dy.saturating_mul(dy))
                        .saturating_add(dz.saturating_mul(dz));
                    (d, e.bx.id, pos)
                })
                .collect();
            order.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            let keep = self.nodes[idx]
                .entries
                .len()
                .saturating_sub(RSTAR_REINSERT)
                .max(RSTAR_MIN);
            let drop_count = self.nodes[idx].entries.len().saturating_sub(keep);
            let mut positions: Vec<usize> =
                order.iter().take(drop_count).map(|(_, _, p)| *p).collect();
            positions.sort_unstable();
            let mut back: Vec<(Rentry, usize)> = Vec::new();
            for p in positions.into_iter().rev() {
                back.push((self.nodes[idx].entries.remove(p), level));
            }
            self.refit(idx);
            return InsertOut::Retry(back);
        }
        let leaf = self.nodes[idx].leaf;
        let entries = std::mem::take(&mut self.nodes[idx].entries);
        let (left, right) = Self::split(entries);
        self.nodes[idx].entries = left;
        self.refit(idx);
        let sib_mbr = self.nodes[idx].mbr;
        self.nodes.push(Rnode {
            mbr: sib_mbr,
            entries: right,
            leaf,
            level,
        });
        let sib_idx = self.nodes.len() - 1;
        self.refit(sib_idx);
        InsertOut::Done(Some(sib_idx))
    }

    /// Recursive removal. Returns `(found, orphans)`: orphan boxes come
    /// from unlinked underfull nodes and are reinserted id-sorted by the
    /// caller, keeping delete-plus-condense deterministic.
    fn remove_rec(&mut self, idx: usize, id: u32, m: &Mbr, orphans: &mut Vec<Box3>) -> bool {
        if self.nodes[idx].leaf {
            let before = self.nodes[idx].entries.len();
            self.nodes[idx].entries.retain(|e| e.bx.id != id);
            let found = self.nodes[idx].entries.len() != before;
            if found {
                if self.nodes[idx].entries.len() < RSTAR_MIN && idx != self.root {
                    let entries = std::mem::take(&mut self.nodes[idx].entries);
                    orphans.extend(entries.into_iter().map(|e| e.bx));
                    return true;
                }
                self.refit(idx);
            }
            return found;
        }
        // Slot order is insertion-deterministic; try every overlapping child.
        let slots: Vec<(u32, usize)> = self.nodes[idx]
            .entries
            .iter()
            .enumerate()
            .map(|(pos, e)| (e.child, pos))
            .collect();
        for (child, pos) in slots {
            let child_idx = child as usize;
            if !self.nodes[child_idx].mbr.hits(m) {
                continue;
            }
            if !self.remove_rec(child_idx, id, m, orphans) {
                continue;
            }
            if self.nodes[child_idx].entries.is_empty() && self.nodes[child_idx].leaf {
                // Emptied leaf (only possible at the root): keep the slot.
                self.refit(child_idx);
            }
            // Refresh or unlink the child slot.
            let unlink = !self.nodes[child_idx].leaf
                && self.nodes[child_idx].entries.len() < RSTAR_MIN
                && child_idx != self.root;
            if unlink {
                self.collect_boxes(child_idx, orphans);
                self.nodes[idx].entries.remove(pos);
            } else if self.nodes[child_idx].leaf && self.nodes[child_idx].entries.is_empty() {
                // Drained leaf collected above; drop the empty slot unless root.
                if child_idx != self.root {
                    self.collect_boxes(child_idx, orphans);
                    self.nodes[idx].entries.remove(pos);
                }
            } else {
                let child_mbr = self.nodes[child_idx].mbr;
                self.nodes[idx].entries[pos] = slot_of(child_idx, &child_mbr);
            }
            self.refit(idx);
            return true;
        }
        false
    }

    fn collect_boxes(&self, idx: usize, out: &mut Vec<Box3>) {
        let node = &self.nodes[idx];
        if node.leaf {
            out.extend(node.entries.iter().map(|e| e.bx));
        } else {
            for e in &node.entries {
                self.collect_boxes(e.child as usize, out);
            }
        }
    }

    /// Remove one id. Missing ids return `false` and change nothing (no
    /// revision); a real removal rebuilds exactly its subtrees and stamps
    /// one revision for this id.
    pub fn remove(&mut self, id: u32) -> bool {
        if !self.remove_entry(id) {
            return false;
        }
        note_edit(
            &mut self.revision,
            &mut self.rebuilds,
            &mut self.last_dirty,
            vec![id],
        );
        true
    }

    /// Structural removal without the session record; batch-apply stamps
    /// one revision for the whole batch instead.
    fn remove_entry(&mut self, id: u32) -> bool {
        let Some(bx) = self.boxes.get(&id).copied() else {
            return false;
        };
        let m = Mbr::of(&bx);
        let mut orphans: Vec<Box3> = Vec::new();
        if !self.remove_rec(self.root, id, &m, &mut orphans) {
            return false;
        }
        self.boxes.remove(&id);
        free_remove(&mut self.free, &free_tiles_of(&bx));
        // Collapse a single-child root to keep the height minimal.
        if !self.nodes[self.root].leaf && self.nodes[self.root].entries.len() == 1 {
            self.root = self.nodes[self.root].entries[0].child as usize;
        }
        if !self.nodes[self.root].leaf && self.nodes[self.root].entries.is_empty() {
            let (nodes, root) = Self::empty_root();
            self.nodes = nodes;
            self.root = root;
        }
        orphans.sort_by_key(|b| b.id);
        let mut reinserted = vec![false; 32];
        for b in orphans {
            if b.id == id {
                continue;
            }
            self.insert_level(
                Rentry {
                    bx: b,
                    child: u32::MAX,
                },
                0,
                &mut reinserted,
            );
        }
        true
    }

    fn query_rec(&self, idx: usize, q: &Mbr, out: &mut Vec<u32>) {
        let node = &self.nodes[idx];
        if !node.mbr.hits(q) {
            return;
        }
        if node.leaf {
            for e in &node.entries {
                if Mbr::of(&e.bx).hits(q) {
                    out.push(e.bx.id);
                }
            }
        } else {
            for e in &node.entries {
                self.query_rec(e.child as usize, q, out);
            }
        }
    }

    /// Strict 3D overlap pairs `(lo, hi)`, sorted. Uses the shared sweep
    /// enumerator over the entry set: matches the legacy `xyz_pairs` set
    /// exactly on the same input. See `pairs_tree` for the native tree
    /// enumeration kept for the contender comparison.
    pub fn pairs(&self) -> Vec<(u32, u32)> {
        let sorted: Vec<Box3> = self.boxes.values().copied().collect();
        sweep_pairs3(&sorted)
    }

    /// Native tree enumeration: one selective query per box, sorted. Same
    /// set as `pairs`; slower on crowded scenes (repeated root-to-leaf
    /// descents over wide MBR overlaps). Kept as the tree side of the
    /// R*-vs-grid numbers.
    pub fn pairs_tree(&self) -> Vec<(u32, u32)> {
        let mut ids: Vec<u32> = self.boxes.keys().copied().collect();
        ids.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::new();
        for id in ids {
            let m = Mbr::of(&self.boxes[&id]);
            let mut found: Vec<u32> = Vec::new();
            self.query_rec(self.root, &m, &mut found);
            found.sort_unstable();
            for peer in found {
                if peer > id && xyz_hit(&self.boxes[&id], &self.boxes[&peer]) {
                    out.push((id, peer));
                }
            }
        }
        out
    }

    /// Strict XZ overlap pairs `(lo, hi)`, sorted: the relation strict-contact
    /// candidate set. Touching and near pairs ride `adjacent_xz`.
    pub fn pairs_xz(&self) -> Vec<(u32, u32)> {
        let sorted: Vec<Box3> = self.boxes.values().copied().collect();
        sweep_xz(&sorted)
    }

    /// XZ adjacency pairs `(lo, hi)` within `gap_cm`, sorted: disjoint XZ
    /// interiors, squared edge gap within range, Y ranges overlapping. The
    /// NextTo phase consumes this with the relation side-by-side limit.
    pub fn adjacent_xz(&self, gap_cm: i32) -> Vec<(u32, u32)> {
        adjacent_of(&self.boxes, gap_cm)
    }

    /// Ids of boxes strictly overlapping the query box, sorted. Inverted
    /// boxes return `Err`, never an empty pass that hides a caller bug.
    pub fn query_box(&self, min: [i32; 3], max: [i32; 3]) -> Result<Vec<u32>, IndexError> {
        if min[0] > max[0] || min[1] > max[1] || min[2] > max[2] {
            return Err(IndexError::InvertedBox);
        }
        let q = Mbr {
            min_x: min[0],
            max_x: max[0],
            min_y: min[1],
            max_y: max[1],
            min_z: min[2],
            max_z: max[2],
        };
        let mut out: Vec<u32> = Vec::new();
        self.query_rec(self.root, &q, &mut out);
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// Levels above an XZ point in integer cm: ids covering the point,
    /// ordered by `(min_y, id)`. Floors, stairs and overhangs read bottom-up.
    pub fn column(&self, x_cm: i32, z_cm: i32) -> Vec<u32> {
        column_of(&self.boxes, x_cm, z_cm)
    }

    /// Vertical band query: ids whose Y range strictly overlaps
    /// `[y_lo, y_hi]`, sorted. Siege-wall and single-floor isolation reads.
    pub fn band(&self, y_lo: i32, y_hi: i32) -> Vec<u32> {
        band_of(&self.boxes, y_lo, y_hi)
    }

    /// True when `probe` strictly overlaps any indexed box except
    /// `ignore_id`. The solver clearance hook: a solved placement must read
    /// `false` here against the scene-minus-object index. The free-space
    /// field is the broadphase: all-free XZ tiles mean clear without the
    /// tree walk; occupied tiles fall through to the exact test.
    pub fn overlaps_any(&self, probe: &Box3, ignore_id: u32) -> bool {
        if probe.min_x < probe.max_x && probe.min_y < probe.max_y && probe.min_z < probe.max_z {
            let pitch = super::navcheck::TILE_CM.max(1);
            let x0 = probe.min_x.div_euclid(pitch);
            let x1 = (probe.max_x - 1).div_euclid(pitch);
            let z0 = probe.min_z.div_euclid(pitch);
            let z1 = (probe.max_z - 1).div_euclid(pitch);
            let nx = i64::from(x1) - i64::from(x0) + 1;
            let nz = i64::from(z1) - i64::from(z0) + 1;
            if nx > 0 && nz > 0 && nx.saturating_mul(nz) <= 1_024 {
                let mut any_hit = false;
                let mut z = z0;
                while z <= z1 && !any_hit {
                    let mut x = x0;
                    while x <= x1 {
                        if self.free.contains_key(&(x, z)) {
                            any_hit = true;
                            break;
                        }
                        if x == x1 {
                            break;
                        }
                        x = x.saturating_add(1);
                    }
                    if z == z1 {
                        break;
                    }
                    z = z.saturating_add(1);
                }
                if !any_hit {
                    return false;
                }
            }
        } else {
            return false;
        }
        let m = Mbr::of(probe);
        let mut found: Vec<u32> = Vec::new();
        self.query_rec(self.root, &m, &mut found);
        found
            .into_iter()
            .any(|id| id != ignore_id && xyz_hit(&self.boxes[&id], probe))
    }

    /// Incremental update: `changed` rows upsert by id (unknown or
    /// degenerate rows delete), `removed` ids delete. Missing ids are a
    /// silent no-op. Only dirty ids lose and regain their subtrees; one
    /// revision stamps the whole batch. The result matches a full rebuild
    /// byte-for-byte.
    pub fn apply(&mut self, changed: &[Actor], removed: &[u32]) {
        let (drops, fresh) = update_plan(changed, removed);
        let mut changed_ids: Vec<u32> = changed.iter().map(|a| a.id).collect();
        changed_ids.sort_unstable();
        changed_ids.dedup();
        let mut dirty: Vec<u32> = Vec::new();
        for id in drops {
            if self.remove_entry(id) {
                dirty.push(id);
            }
        }
        for id in changed_ids {
            match fresh.get(&id).copied() {
                Some(next) => {
                    if !same_opt(self.boxes.get(&id).copied(), Some(next)) {
                        self.insert_entry(next);
                        dirty.push(id);
                    }
                }
                None => {
                    if self.remove_entry(id) {
                        dirty.push(id);
                    }
                }
            }
        }
        note_edit(
            &mut self.revision,
            &mut self.rebuilds,
            &mut self.last_dirty,
            dirty,
        );
    }

    /// Deterministic bytes, same layout as [`Grid3`]: header plus id-sorted
    /// entry rows. Rebuild-vs-incremental and grid-vs-tree are byte-identical.
    pub fn to_bytes(&self) -> Vec<u8> {
        encode_entries(&self.boxes)
    }

    /// Decode bytes from [`to_bytes`](Self::to_bytes); rebuilds the tree
    /// id-sorted so the shape is deterministic per entry set.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, IndexError> {
        let boxes = decode_entries(bytes)?;
        let (nodes, root) = Self::empty_root();
        let mut tree = Self {
            nodes,
            root,
            boxes: BTreeMap::new(),
            revision: 0,
            rebuilds: 0,
            last_dirty: Vec::new(),
            free: BTreeMap::new(),
        };
        let mut rows: Vec<Box3> = boxes.values().copied().collect();
        rows.sort_by_key(|b| b.id);
        for b in rows {
            tree.insert_entry(b);
        }
        Ok(tree)
    }

    /// Order-invariant FNV-1a digest over integer entry bytes.
    pub fn digest(&self) -> u64 {
        digest_entries(&self.boxes)
    }
}

fn sort_for_split(entries: &mut [Rentry], axis: usize, by_max: bool) {
    entries.sort_by_key(|e| {
        let m = Mbr::of(&e.bx);
        let key = match (axis, by_max) {
            (0, false) => m.min_x,
            (0, true) => m.max_x,
            (1, false) => m.min_y,
            (1, true) => m.max_y,
            (2, false) => m.min_z,
            _ => m.max_z,
        };
        (key, e.bx.id)
    });
}

/// Exact union over a split group. Groups are non-empty by the split clamp.
fn group_mbr(group: &[Rentry]) -> Mbr {
    let mut mbr: Option<Mbr> = None;
    for e in group {
        let m = Mbr::of(&e.bx);
        mbr = Some(match mbr {
            Some(cur) => cur.combine(&m),
            None => m,
        });
    }
    // Callers only pass non-empty groups; fall back to the first entry.
    mbr.unwrap_or_else(|| Mbr::of(&group[0].bx))
}

// ---------------------------------------------------------------------------
// Index3: the measured winner behind one query entry
// ---------------------------------------------------------------------------

/// The shipped 3D index: [`Rstar`], the measured winner over [`Grid3`]
/// (pair phase at 5k objects, query at 10k; numbers in the timing tests).
/// Solver clearance and relation candidates both hang off this one entry point.
pub type Index3 = Rstar;

/// One-shot strict-3D pairs over an actor snapshot, sorted. Drop-in behind
/// the legacy `xyz_pairs` query entry: same set, one sweep pass, integer
/// cm. Deliberately builds no persistent structure: a one-shot query pays
/// only the box conversion plus the sweep (`boxes_of` already emits sweep
/// order, so no second sort).
pub fn pairs3(actors: &[Actor]) -> Vec<(u32, u32)> {
    let live: Vec<Actor> = actors.iter().filter(|a| live_row(a)).cloned().collect();
    let sorted = boxes_of(&live);
    let order: Vec<usize> = (0..sorted.len()).collect();
    sweep_pairs3_ordered(&sorted, &order)
}

/// Probe box for an actor center/half-extents in integer cm. Shared by the
/// solver clearance checks so callers never hand-roll the conversion.
pub fn probe_of(x_cm: i32, y_cm: i32, z_cm: i32, hx: i32, hy: i32, hz: i32) -> Box3 {
    Box3 {
        id: u32::MAX,
        min_x: x_cm - hx,
        max_x: x_cm + hx,
        min_y: y_cm - hy,
        max_y: y_cm + hy,
        min_z: z_cm - hz,
        max_z: z_cm + hz,
    }
}

/// Probe box for one actor's current transform, in integer cm.
pub fn probe_actor(a: &Actor) -> Option<Box3> {
    if !live_row(a) {
        return None;
    }
    Some(probe_of(
        to_cm(a.x),
        to_cm(a.y),
        to_cm(a.z),
        to_cm(a.sx).abs() / 2,
        to_cm(a.sy).abs() / 2,
        to_cm(a.sz).abs() / 2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{overlap_pairs, sweep_pairs, xyz_pairs};
    use crate::place::{Intent, ObjectRef, Target, Wall};
    use crate::relation::{digest as relation_digest, Relation};

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(id: u32, name: &str, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
        mk(id, name, x, 0.5, z, 1.0, 1.0, 1.0)
    }

    /// Overlap fixture: two touching pairs plus one far actor.
    fn overlap_scene() -> Vec<Actor> {
        vec![
            resting(0, "a", 0.0, 0.0),
            resting(1, "b", 0.5, 0.0),
            resting(2, "c", 5.0, 5.0),
            resting(3, "d", 5.4, 5.0),
            resting(4, "e", 40.0, 40.0),
        ]
    }

    /// Two-floor ludus: ground slab, first-floor slab, stairs of three
    /// steps, actors on each level, plus a balcony overhang. Resting pairs
    /// touch faces only; sunk volumes truly intersect.
    fn ludus() -> Vec<Actor> {
        vec![
            // Ground slab 10x0.2x10 at y=0.1 (top y=0.2).
            mk(0, "slab_ground", 0.0, 0.1, 0.0, 10.0, 0.2, 10.0),
            // First-floor slab 6x0.2x6 at y=3.1 (top y=3.2), north half.
            mk(1, "slab_first", 0.0, 3.1, -2.0, 6.0, 0.2, 6.0),
            // Stairs: three 1m steps rising south to north, 1m wide.
            mk(2, "step_0", 0.0, 0.5, 3.5, 1.0, 1.0, 1.0),
            mk(3, "step_1", 0.0, 1.5, 2.5, 1.0, 1.0, 1.0),
            mk(4, "step_2", 0.0, 2.5, 1.5, 1.0, 1.0, 1.0),
            // Balcony overhang: thin plate at y=2.6 over the south walkway.
            mk(5, "balcony", 3.0, 2.6, 3.0, 2.0, 0.2, 2.0),
            // Ground-floor actor on the slab (feet at 0.2 = slab top).
            mk(6, "walker", 3.0, 1.1, 3.0, 0.6, 1.8, 0.6),
            // First-floor actor standing on slab_first (feet at 3.2).
            mk(7, "guard", -1.0, 4.1, -2.0, 0.6, 1.8, 0.6),
            // Crate resting on the ground slab (feet at 0.2).
            mk(8, "crate", -3.0, 0.7, 3.0, 1.0, 1.0, 1.0),
        ]
    }

    /// Siege wall: four tall segments in a row plus a gate pier and a
    /// ladder actor mid-wall. Wall ids 10..14, props elsewhere.
    fn siege_wall() -> Vec<Actor> {
        vec![
            mk(10, "wall_a", -6.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(11, "wall_b", -2.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(12, "wall_c", 2.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(13, "wall_d", 6.0, 3.0, 0.0, 4.0, 6.0, 1.0),
            mk(14, "gate_pier", 0.0, 2.0, 0.0, 1.0, 4.0, 1.2),
            mk(15, "ladder", -2.0, 1.5, 1.2, 0.6, 3.0, 0.4),
            mk(16, "tent", -2.0, 1.0, 12.0, 3.0, 2.0, 3.0),
        ]
    }

    // -- overlap/sweep/xyz equivalence -------------------------------------

    #[test]
    fn index3_sp1_overlap_pairs_match_legacy() {
        let scene = overlap_scene();
        let rects = crate::index::build_rects(&scene);
        let mut legacy: Vec<(u32, u32)> = overlap_pairs(&rects)
            .into_iter()
            .map(|(a, b, _)| (a, b))
            .collect();
        legacy.sort_unstable();
        assert_eq!(Rstar::build(&scene).pairs_xz(), legacy);
        assert_eq!(Grid3::build(&scene).pairs_xz(), legacy);
        // 3D pairs agree with the legacy xyz path on flat scenes.
        assert_eq!(Rstar::build(&scene).pairs(), xyz_pairs(&scene));
        assert_eq!(Grid3::build(&scene).pairs(), xyz_pairs(&scene));
    }

    #[test]
    fn index3_sp1_stacked_volumes_match_xyz() {
        let low = resting(0, "low", 5.0, 5.0);
        let mut high = resting(1, "high", 5.0, 5.0);
        high.y = 1.6;
        assert!(pairs3(&[low.clone(), high.clone()]).is_empty());
        assert!(xyz_pairs(&[low.clone(), high.clone()]).is_empty());
        high.y = 1.4;
        assert_eq!(pairs3(&[low.clone(), high.clone()]), vec![(0, 1)]);
        assert_eq!(Grid3::build(&[low, high]).pairs(), vec![(0, 1)]);
    }

    #[test]
    fn index3_sp1_resting_contact_is_xz_only() {
        // Resting contact shares XZ but never strictly intersects in 3D:
        // the XZ sweep pairs it, the 3D index must not.
        let scene = ludus();
        let rstar = Rstar::build(&scene);
        let grid = Grid3::build(&scene);
        // Walker (id 6) under the balcony (id 5): XZ pair, no 3D hit.
        assert!(rstar.pairs_xz().contains(&(5, 6)));
        assert!(!rstar.pairs().contains(&(5, 6)));
        assert!(!grid.pairs().contains(&(5, 6)));
        // Guard on slab_first: XZ pair, no strict 3D hit (resting faces).
        assert!(rstar.pairs_xz().contains(&(1, 7)));
        assert!(!rstar.pairs().contains(&(1, 7)));
        // Crate on the ground slab: same story.
        assert!(rstar.pairs_xz().contains(&(0, 8)));
        assert!(!rstar.pairs().contains(&(0, 8)));
    }

    #[test]
    fn index3_sp1_report_unchanged_on_flat_scene() {
        // The full overlap report over a flat scene is identical with the index
        // present: same findings, same hash, both input orders.
        let scene = overlap_scene();
        let r1 = crate::run(&scene);
        let mut rev = scene.clone();
        rev.reverse();
        let r2 = crate::run(&rev);
        assert_eq!(r1.hash, r2.hash);
        assert_eq!(r1.count("overlap"), 2);
        assert_eq!(Rstar::build(&scene).digest(), Rstar::build(&rev).digest());
        assert_eq!(Grid3::build(&scene).digest(), Grid3::build(&rev).digest());
    }

    #[test]
    fn index3_sp1_sweep_scope_is_documented() {
        // Swept (velocity) pairs stay an XZ concern: the sweep check still
        // fires on movers, and the static 3D index agrees there is no
        // resting contact.
        let mover = Actor {
            vx: 2.0,
            ..resting(0, "mover", 0.0, 0.0)
        };
        let wall = resting(1, "wall", 1.5, 0.0);
        let scene = vec![mover, wall];
        assert_eq!(sweep_pairs(&scene, 60).len(), 1);
        assert!(pairs3(&scene).is_empty());
    }

    // -- staged gate plus brief inputs unchanged -------------------------------

    #[test]
    fn index3_sp2_staged_gate_stable_with_levels() {
        let before = ludus();
        let mut staged = before.clone();
        // Step the walker aside but keep it grounded on the slab and clear
        // of every volume: the XZ pair set is unchanged, so the gate is quiet.
        staged[6] = mk(6, "walker", 4.0, 1.1, 4.0, 0.6, 1.8, 0.6);
        let delta = crate::staged::check_staged(&before, &staged);
        assert!(delta.findings.is_empty());
        // Index agrees the moved walker touches nothing.
        let index = Rstar::build(&staged);
        let probe = probe_actor(&staged[6]).expect("valid probe");
        assert!(!index.overlaps_any(&probe, 6));
        // A stacked intruder is caught by both the gate and the index.
        let mut bad = staged.clone();
        bad.push(mk(9, "intruder", 4.0, 1.1, 4.0, 0.6, 1.8, 0.6));
        let delta2 = crate::staged::check_staged(&before, &bad);
        assert!(delta2.count("overlap") >= 1);
        let bad_index = Rstar::build(&bad);
        let bad_probe = probe_actor(&bad[9]).expect("valid probe");
        assert!(bad_index.overlaps_any(&bad_probe, 9));
    }

    // -- relation agreement ------------------------------------------------

    #[test]
    fn index3_sp3_relation_edges_sit_in_candidates() {
        // The relation consumer contract, per edge kind: resting contact needs
        // strict XZ interiors (pairs_xz); containment also tolerates flush
        // faces (adjacent gap 0); side-by-side pairs ride the relation
        // gap limit with vertical overlap (adjacent_xz).
        use crate::relation::NEXT_TO_CM;
        for scene in [ludus(), siege_wall(), overlap_scene()] {
            let graph = crate::relation::build(&scene);
            let tree = Rstar::build(&scene);
            let strict: std::collections::BTreeSet<(u32, u32)> =
                tree.pairs_xz().into_iter().collect();
            let flush: std::collections::BTreeSet<(u32, u32)> =
                tree.adjacent_xz(0).into_iter().collect();
            let near: std::collections::BTreeSet<(u32, u32)> =
                tree.adjacent_xz(NEXT_TO_CM).into_iter().collect();
            let key = |a: &u32, b: &u32| {
                if a < b {
                    (*a, *b)
                } else {
                    (*b, *a)
                }
            };
            for (a, rel, b) in &graph.edges {
                let k = key(a, b);
                match rel {
                    Relation::Supports | Relation::OnTopOf => {
                        assert!(
                            strict.contains(&k),
                            "edge ({a}, {rel:?}, {b}) missing from strict XZ candidates"
                        );
                    }
                    Relation::Inside => {
                        assert!(
                            strict.contains(&k) || flush.contains(&k),
                            "edge ({a}, {rel:?}, {b}) missing from containment candidates"
                        );
                    }
                    Relation::NextTo => {
                        assert!(
                            near.contains(&k),
                            "edge ({a}, {rel:?}, {b}) missing from adjacency candidates"
                        );
                    }
                    Relation::Facing => {}
                }
            }
            // Grid agrees with the tree candidate-for-candidate.
            assert_eq!(
                Grid3::build(&scene).pairs_xz(),
                Rstar::build(&scene).pairs_xz()
            );
            assert_eq!(
                Grid3::build(&scene).adjacent_xz(NEXT_TO_CM),
                Rstar::build(&scene).adjacent_xz(NEXT_TO_CM)
            );
        }
    }

    #[test]
    fn index3_sp3_digest_untouched() {
        // Relation digests are stable with the index alongside: same scene
        // twice, both orders, grid and tree digests match each other.
        let scene = ludus();
        let g1 = crate::relation::build(&scene);
        let mut rev = scene.clone();
        rev.reverse();
        let g2 = crate::relation::build(&rev);
        assert_eq!(relation_digest(&g1), relation_digest(&g2));
        assert_eq!(Rstar::build(&scene).digest(), Grid3::build(&scene).digest());
    }

    #[test]
    fn index3_sp3_incremental_matches_rebuild() {
        // Dirty-subtree idea (salsa red-green pattern): callers pass the
        // changed rows; untouched rows keep their index rows. Full rebuild
        // and incremental apply agree pair-for-pair and byte-for-byte.
        let old = ludus();
        let mut grids = Grid3::build(&old);
        let mut trees = Rstar::build(&old);
        let mut next = old.clone();
        next[6] = mk(6, "walker", -4.0, 1.1, 4.0, 0.6, 1.8, 0.6);
        next.push(mk(20, "new_crate", 4.0, 0.7, -4.0, 1.0, 1.0, 1.0));
        grids.apply(&[next[6].clone(), next[9].clone()], &[]);
        trees.apply(&[next[6].clone(), next[9].clone()], &[]);
        let full_grid = Grid3::build(&next);
        let full_tree = Rstar::build(&next);
        assert_eq!(grids.pairs(), full_grid.pairs());
        assert_eq!(trees.pairs(), full_tree.pairs());
        assert_eq!(grids.pairs_xz(), full_grid.pairs_xz());
        assert_eq!(trees.pairs_xz(), full_tree.pairs_xz());
        assert_eq!(grids.to_bytes(), full_grid.to_bytes());
        assert_eq!(trees.to_bytes(), full_tree.to_bytes());
        // Removal needs no listing beyond the id, like rebuild_cached.
        grids.apply(&[], &[20]);
        trees.apply(&[], &[20]);
        let smaller: Vec<Actor> = next.into_iter().filter(|a| a.id != 20).collect();
        assert_eq!(grids.pairs(), Grid3::build(&smaller).pairs());
        assert_eq!(trees.pairs(), Rstar::build(&smaller).pairs());
        assert_eq!(trees.to_bytes(), Rstar::build(&smaller).to_bytes());
        assert_eq!(grids.digest(), Grid3::build(&smaller).digest());
    }

    // -- solver clearance -------------------------------------------------------

    #[test]
    fn index3_sp5_solved_placements_read_clear() {
        // Every solved placement must read clear on the
        // scene-minus-object index, or the consumer wiring has drifted.
        let actors = vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 9.8, 2.0, 0.2),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 9.8, 2.0, 0.2),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(4, "gate", 3.0, 0.5, -4.5, 1.0, 1.0, 0.5),
            mk(5, "brazier", 0.0, 0.5, 2.0, 0.6, 1.0, 0.6),
        ];
        let intent = Intent {
            object: ObjectRef::Name("brazier".to_string()),
            wants: vec![
                crate::place::Want {
                    relation: Relation::NextTo,
                    target: Target::Wall(Wall::North),
                    distance_cm: Some((0, 60)),
                },
                crate::place::Want {
                    relation: Relation::NextTo,
                    target: Target::Object(4),
                    distance_cm: Some((150, 250)),
                },
            ],
        };
        let outs = crate::place::solve(&intent, &actors).expect("board intent solves");
        assert_eq!(outs.len(), 1);
        let rest: Vec<Actor> = actors
            .iter()
            .filter(|a| a.id != outs[0].id)
            .cloned()
            .collect();
        let index = Index3::build(&rest);
        let moved = mk(
            outs[0].id, "brazier", outs[0].x, outs[0].y, outs[0].z, 0.6, 1.0, 0.6,
        );
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!index.overlaps_any(&probe, outs[0].id));
        // And the placed scene still runs overlap-clean.
        let placed: Vec<Actor> = actors
            .iter()
            .map(|a| {
                if a.id == outs[0].id {
                    Actor {
                        x: outs[0].x,
                        y: outs[0].y,
                        z: outs[0].z,
                        yaw: outs[0].yaw,
                        ..a.clone()
                    }
                } else {
                    a.clone()
                }
            })
            .collect();
        assert!(crate::run(&placed).findings.is_empty());
    }

    // -- Vertical queries: ludus + siege wall ---------------------------------

    #[test]
    fn index3_column_reads_levels_bottom_up() {
        let index = Index3::build(&ludus());
        // Through the first-floor slab: ground slab, then first slab.
        let col = index.column(0, -200);
        assert!(col.contains(&0) && col.contains(&1));
        assert!(col.iter().position(|id| *id == 0) < col.iter().position(|id| *id == 1));
        // Under the balcony: ground slab, walker, balcony plate.
        let col2 = index.column(300, 300);
        assert_eq!(col2, vec![0, 6, 5]);
        assert_eq!(Grid3::build(&ludus()).column(300, 300), col2);
        // Empty sky point reads no levels.
        assert!(index.column(9000, 9000).is_empty());
    }

    #[test]
    fn column_top_empty_is_none() {
        let empty: std::collections::BTreeMap<u32, _> = std::collections::BTreeMap::new();
        assert_eq!(column_top_cm(&empty, 0, 0), None);
        let map: std::collections::BTreeMap<u32, _> = live_boxes(&overlap_scene())
            .into_iter()
            .map(|b| (b.id, b))
            .collect();
        assert_eq!(column_top_cm(&map, 9000, 9000), None);
    }

    #[test]
    fn column_top_single_slab_returns_top() {
        let scene = vec![mk(0, "slab_ground", 0.0, 0.1, 0.0, 10.0, 0.2, 10.0)];
        let map: std::collections::BTreeMap<u32, _> =
            live_boxes(&scene).into_iter().map(|b| (b.id, b)).collect();
        assert_eq!(column_top_cm(&map, 0, 0), Some(20));
        assert_eq!(column_top_cm(&map, 9000, 9000), None);
    }

    #[test]
    fn column_top_ludus_two_levels_returns_upper_top() {
        let map: std::collections::BTreeMap<u32, _> = live_boxes(&ludus())
            .into_iter()
            .map(|b| (b.id, b))
            .collect();
        assert_eq!(column_top_cm(&map, 0, -200), Some(320));
    }

    #[test]
    fn column_top_siege_wall_returns_crown_top() {
        let map: std::collections::BTreeMap<u32, _> = live_boxes(&siege_wall())
            .into_iter()
            .map(|b| (b.id, b))
            .collect();
        assert_eq!(column_top_cm(&map, -200, 0), Some(600));
        assert_eq!(column_top_cm(&map, 200, 0), Some(600));
    }

    #[test]
    fn index3_band_isolates_floors_and_walls() {
        let index = Index3::build(&ludus());
        // Ground band (0..100 cm): slab, steps, walker, crate; no guard.
        let ground = index.band(0, 100);
        assert!(ground.contains(&0) && ground.contains(&6) && ground.contains(&8));
        assert!(!ground.contains(&7));
        // Upper band (300..500 cm): first slab, top step, guard; no walker.
        let upper = index.band(300, 500);
        assert!(upper.contains(&1) && upper.contains(&7));
        assert!(!upper.contains(&6));
        assert_eq!(Grid3::build(&ludus()).band(300, 500), upper);
        // Siege wall: full-height band holds every segment; a 1 m slice at
        // the top holds the walls but not the tent.
        let walls = Index3::build(&siege_wall());
        let full = walls.band(0, 600);
        for id in [10, 11, 12, 13, 14] {
            assert!(full.contains(&id), "wall {id} missing from full band");
        }
        let crown = walls.band(500, 600);
        for id in [10, 11, 12, 13] {
            assert!(crown.contains(&id), "wall {id} missing from crown band");
        }
        assert!(!crown.contains(&16));
    }

    #[test]
    fn index3_siege_wall_queries_answer_vertically() {
        let walls = Index3::build(&siege_wall());
        // Tall probe overlapping the wall row hits the segments it covers.
        // (Strict hit: the probe must cross faces, not just touch them.)
        let hit = walls
            .query_box([-401, 0, -100], [0, 600, 100])
            .expect("valid query");
        assert!(hit.contains(&10) && hit.contains(&11));
        assert!(!hit.contains(&12) && !hit.contains(&16));
        // Gate pier probe: pier plus flanking segments, not the far tent.
        let gate = walls
            .query_box([-50, 0, -60], [50, 400, 60])
            .expect("valid query");
        assert!(gate.contains(&14));
        assert!(!gate.contains(&16));
        // Grid answers identically.
        let grid = Grid3::build(&siege_wall());
        assert_eq!(
            grid.query_box([-401, 0, -100], [0, 600, 100])
                .expect("valid query"),
            hit
        );
    }

    #[test]
    fn index3_stairs_chain_without_false_contacts() {
        // Face-touching steps share no strict volume, so they must not pair;
        // resting bodies touch faces only. A sunk intruder truly intersects.
        let scene = ludus();
        let pairs = pairs3(&scene);
        assert!(
            !pairs.contains(&(2, 3)),
            "face-touching steps must not pair"
        );
        assert!(!pairs.contains(&(0, 8)));
        let mut sunk = scene.clone();
        sunk.push(mk(21, "sunk", 0.0, 0.4, 3.5, 1.0, 1.0, 1.0));
        assert!(pairs3(&sunk).contains(&(2, 21)));
    }

    #[test]
    fn index3_giant_wall_stays_exact() {
        // A 150 m wall spans more cells than any clamp: the grid giants
        // list keeps its far-end pairs and queries exact, matching the
        // sweep and the tree. (The legacy clamped fan-out is lossy here
        // by construction; the index is not.)
        let scene = vec![
            mk(0, "long_wall", 0.0, 3.0, 0.0, 150.0, 6.0, 1.0),
            mk(1, "hut", 70.0, 1.0, 0.0, 3.0, 2.0, 3.0),
            mk(2, "far_hut", -70.0, 1.0, 40.0, 3.0, 2.0, 3.0),
        ];
        let grid = Grid3::build(&scene);
        let tree = Rstar::build(&scene);
        assert_eq!(grid.pairs(), vec![(0, 1)]);
        assert_eq!(grid.pairs_cells(), vec![(0, 1)]);
        assert_eq!(tree.pairs(), vec![(0, 1)]);
        assert_eq!(pairs3(&scene), vec![(0, 1)]);
        // Far-end query hits the wall; far-hut region does not.
        let far = grid
            .query_box([6900, 0, -100], [7100, 600, 100])
            .expect("valid query");
        assert_eq!(far, vec![0, 1]);
        assert_eq!(
            tree.query_box([6900, 0, -100], [7100, 600, 100])
                .expect("valid query"),
            far
        );
        let away = grid
            .query_box([-7100, 0, 3900], [-6900, 600, 4100])
            .expect("valid query");
        assert_eq!(away, vec![2]);
        // Incremental detach of the giant tracks the overflow list.
        let mut updated = grid.clone();
        updated.apply(&[], &[0]);
        assert!(updated.pairs().is_empty());
        assert!(updated.to_bytes() != grid.to_bytes());
    }

    // -- Bytes, digest, errors --------------------------------------------------

    #[test]
    fn index3_bytes_stable_x2_and_cross_flavor() {
        let scene = ludus();
        let a = Rstar::build(&scene);
        let b = Rstar::build(&scene);
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_eq!(a.digest(), b.digest());
        // Grid and tree share the entry layout: byte-identical blobs.
        assert_eq!(a.to_bytes(), Grid3::build(&scene).to_bytes());
        assert_eq!(12 + scene.len().min(a.len()) * 28, a.to_bytes().len());
        // Round-trip preserves pairs, queries and digest.
        let back = Rstar::from_bytes(&a.to_bytes()).expect("round-trip decodes");
        assert_eq!(back.pairs(), a.pairs());
        assert_eq!(back.digest(), a.digest());
        assert_eq!(back.column(300, 300), a.column(300, 300));
        let back_grid = Grid3::from_bytes(&a.to_bytes()).expect("grid decodes tree bytes");
        assert_eq!(back_grid.pairs(), a.pairs());
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn dirty_tiles_export_20_of_20_with_p95_and_bytes_x2() {
        // Twenty distinct 1 m props on a 2 m pitch: one center tile per box.
        let base: Vec<Actor> = (0..20)
            .map(|i| {
                let x = f32::from(u16::try_from(i).unwrap_or(0)) * 2.0;
                resting(i, &format!("p{i}"), x, 0.0)
            })
            .collect();
        let mut index = Index3::build(&base);
        assert!(index.dirty_tiles().is_empty());
        let moved: Vec<Actor> = base
            .iter()
            .map(|a| {
                let mut row = a.clone();
                row.x += 1.0;
                row
            })
            .collect();
        index.apply(&moved, &[]);
        let tiles = index.dirty_tiles();
        assert_eq!(tiles.len(), 20);
        let pitch = crate::navcheck::TILE_CM;
        let mut expect: Vec<(i32, i32)> = moved
            .iter()
            .map(|a| (to_cm(a.x).div_euclid(pitch), to_cm(a.z).div_euclid(pitch)))
            .collect();
        expect.sort_unstable_by_key(|&(x, z)| (z, x));
        expect.dedup();
        assert_eq!(tiles, expect);
        assert_eq!(tiles.len(), 20);
        let mut grid = Grid3::build(&base);
        grid.apply(&moved, &[]);
        assert_eq!(grid.dirty_tiles(), tiles);
        let mut samples: Vec<u128> = Vec::with_capacity(100);
        for _ in 0..100 {
            let start = std::time::Instant::now();
            let _ = index.dirty_tiles();
            samples.push(std::time::Instant::now().duration_since(start).as_nanos());
        }
        samples.sort_unstable();
        let p95 = samples[95];
        assert!(
            p95 <= 100_000,
            "enumerate p95 {p95}ns must stay at or under 0.1 ms"
        );
        let first = dirty_tile_bytes(&tiles);
        let second = dirty_tile_bytes(&tiles);
        assert_eq!(first, second);
        assert_eq!(first.len(), 4 + 20 * 8);
        let mut rev = tiles.clone();
        rev.reverse();
        assert_eq!(dirty_tile_bytes(&rev), first);
        assert_eq!(index.dirty_tiles(), tiles);
    }

    #[test]
    fn index3_no_float_reaches_the_digest() {
        // Two f32 spellings that round to the same integer cm digest alike:
        // the hash only ever sees integer entry bytes.
        let a = vec![resting(0, "a", 0.104, 0.0)];
        let b = vec![resting(0, "a", 0.1044, 0.0)];
        assert_eq!(to_cm(0.104), to_cm(0.1044));
        assert_eq!(Rstar::build(&a).digest(), Rstar::build(&b).digest());
        assert_eq!(Rstar::build(&a).to_bytes(), Rstar::build(&b).to_bytes());
    }

    #[test]
    fn index3_failures_are_typed_not_panics() {
        let index = Rstar::build(&ludus());
        assert_eq!(
            index.query_box([1, 1, 1], [0, 0, 0]).unwrap_err(),
            IndexError::InvertedBox
        );
        assert_eq!(
            Grid3::build(&ludus())
                .query_box([0, 5, 0], [0, 4, 0])
                .unwrap_err(),
            IndexError::InvertedBox
        );
        assert_eq!(
            Rstar::from_bytes(b"short").unwrap_err(),
            IndexError::BadMagic
        );
        let mut bad_version = index.to_bytes();
        bad_version[6] = 9;
        assert_eq!(
            Rstar::from_bytes(&bad_version).unwrap_err(),
            IndexError::BadVersion(9)
        );
        let mut truncated = index.to_bytes();
        truncated.truncate(truncated.len() - 1);
        assert_eq!(
            Rstar::from_bytes(&truncated).unwrap_err(),
            IndexError::Truncated
        );
        let mut too_many = index.to_bytes();
        let over = (IX3_MAX_ENTRIES + 1).to_le_bytes();
        too_many[8..12].copy_from_slice(&over);
        assert_eq!(
            Rstar::from_bytes(&too_many).unwrap_err(),
            IndexError::TooMany(IX3_MAX_ENTRIES + 1)
        );
        // Display strings name the failure for receipts.
        assert!(format!("{}", IndexError::BadMagic).contains("magic"));
    }

    #[test]
    fn index3_unknown_and_empty_inputs_stay_quiet() {
        let mut lost = resting(50, "lost", 0.0, 0.0);
        lost.x = f32::NAN;
        let mut flat = resting(51, "flat", 2.0, 0.0);
        flat.sy = 0.0;
        let scene = vec![resting(0, "a", 0.0, 0.0), lost, flat];
        let tree = Rstar::build(&scene);
        let grid = Grid3::build(&scene);
        // NaN position and flat size index nothing, like relation isolates.
        assert_eq!(tree.len(), 1);
        assert_eq!(grid.len(), 1);
        assert!(tree.pairs().is_empty() && grid.pairs().is_empty());
        assert_eq!(tree.column(0, 0), vec![0]);
        let mut empty = Rstar::build(&[]);
        assert!(empty.is_empty());
        assert!(empty.pairs().is_empty());
        assert!(empty
            .query_box([0, 0, 0], [1, 1, 1])
            .expect("valid query")
            .is_empty());
        assert_eq!(empty.digest(), Rstar::build(&[]).digest());
        assert!(!empty.remove(7), "missing remove is a quiet no-op");
        let mut staged = Rstar::build(&[]);
        staged.apply(&[], &[7]);
        assert!(staged.is_empty());
    }

    #[test]
    fn index3_upsert_and_remove_keep_one_row() {
        let mut tree = Rstar::build(&overlap_scene());
        assert_eq!(tree.len(), 5);
        // Move id 1 far away: still one row per id, pairs track the move.
        tree.apply(&[resting(1, "b", 30.0, 30.0)], &[]);
        let expect = Rstar::build(&[
            resting(0, "a", 0.0, 0.0),
            resting(1, "b", 30.0, 30.0),
            resting(2, "c", 5.0, 5.0),
            resting(3, "d", 5.4, 5.0),
            resting(4, "e", 40.0, 40.0),
        ]);
        assert_eq!(tree.len(), 5);
        assert_eq!(tree.pairs(), expect.pairs());
        assert_eq!(tree.pairs_xz(), expect.pairs_xz());
        assert_eq!(tree.to_bytes(), expect.to_bytes());
        // Remove twice: the second remove is a quiet no-op.
        assert!(tree.remove(1));
        assert!(!tree.remove(1));
        assert_eq!(tree.len(), 4);
        assert_eq!(
            tree.pairs(),
            Rstar::build(&[
                resting(0, "a", 0.0, 0.0),
                resting(2, "c", 5.0, 5.0),
                resting(3, "d", 5.4, 5.0),
                resting(4, "e", 40.0, 40.0),
            ])
            .pairs()
        );
    }

    #[test]
    fn index3_tree_height_stays_shallow() {
        // 1024 disjoint boxes must sit in a bounded-height tree.
        let scene: Vec<Actor> = (0..1024)
            .map(|i| {
                let x = (i % 32) as f32 * 2.0;
                let z = (i / 32) as f32 * 2.0;
                resting(i, "prop", x, z)
            })
            .collect();
        let tree = Rstar::build(&scene);
        assert!(tree.height() <= 8, "height {} too deep", tree.height());
        assert!(tree.pairs().is_empty());
        assert_eq!(tree.pairs(), Grid3::build(&scene).pairs());
    }

    // -- Wall-clock gates: ignored by default, run under --perf in release ---

    /// Deterministic scatter: `n` grounded 1 m boxes on a 2 m pitch grid.
    /// No RNG, no wall clock: same `n` builds the same scene anywhere.
    fn scatter(n: u32) -> Vec<Actor> {
        let side = (n as f64).sqrt().ceil() as u32;
        (0..n)
            .map(|i| {
                let x = (i % side) as f32 * 2.0;
                let z = (i / side) as f32 * 2.0;
                resting(i, "prop", x, z)
            })
            .collect()
    }

    /// Overlapping rows: every box laps its neighbours, so the pair phase
    /// has real narrow-phase work instead of an empty candidate set.
    fn crowded(n: u32) -> Vec<Actor> {
        (0..n)
            .map(|i| resting(i, "prop", f32::from(i as u16) * 0.4, 0.0))
            .collect()
    }

    #[test]
    #[ignore = "timing: pair-phase 5k vs legacy, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_pair_phase_5k_winner_3x() {
        let scene = crowded(5000);
        // Medians of 3: the shared pool is noisy, ratios must be stable.
        let mut legacy_runs: Vec<u128> = Vec::with_capacity(3);
        let mut shipped_runs: Vec<u128> = Vec::with_capacity(3);
        let mut legacy_xyz = 0usize;
        for _ in 0..3 {
            // Legacy pair phase: XZ sweep pairs plus the XYZ grid pass.
            let start = std::time::Instant::now();
            let rects = crate::index::build_rects(&scene);
            let legacy_xz = overlap_pairs(&rects).len();
            legacy_xyz = xyz_pairs(&scene).len();
            legacy_runs.push(std::time::Instant::now().duration_since(start).as_micros());
            // Shipped entry: one-shot sweep pairs over the snapshot.
            let start = std::time::Instant::now();
            let shipped = pairs3(&scene);
            shipped_runs.push(
                std::time::Instant::now()
                    .duration_since(start)
                    .as_micros()
                    .max(1),
            );
            assert_eq!(shipped.len(), legacy_xyz);
            let _ = legacy_xz;
        }
        legacy_runs.sort_unstable();
        shipped_runs.sort_unstable();
        let legacy_us = legacy_runs[1];
        let shipped_us = shipped_runs[1];
        // Grid contender, split into build and enumeration halves.
        let start = std::time::Instant::now();
        let grid = Grid3::build(&scene);
        let grid_build_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let grid_pairs = grid.pairs().len();
        let grid_pairs_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let grid_cells = grid.pairs_cells().len();
        let grid_cells_us = std::time::Instant::now().duration_since(start).as_micros();
        // R* contender, split the same way.
        let start = std::time::Instant::now();
        let tree = Rstar::build(&scene);
        let tree_build_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let tree_pairs = tree.pairs().len();
        let tree_pairs_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let tree_native = tree.pairs_tree().len();
        let tree_native_us = std::time::Instant::now().duration_since(start).as_micros();
        eprintln!(
            "pair-phase 5k crowded: legacy={legacy_us}us (xz pairs, xyz={legacy_xyz}) shipped={shipped_us}us grid build={grid_build_us}us pairs={grid_pairs_us}us (n={grid_pairs}) cells={grid_cells_us}us (n={grid_cells}) rstar build={tree_build_us}us pairs={tree_pairs_us}us (n={tree_pairs}) native={tree_native_us}us (n={tree_native})"
        );
        assert_eq!(grid_pairs, legacy_xyz);
        assert_eq!(grid_cells, legacy_xyz);
        assert_eq!(tree_pairs, legacy_xyz);
        assert_eq!(tree_native, legacy_xyz);
        assert!(
            legacy_us >= 3 * shipped_us,
            "shipped {shipped_us}us must beat legacy {legacy_us}us by 3x"
        );
    }

    #[test]
    #[ignore = "timing: pair-phase 50 no regression, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_pair_phase_50_no_regression() {
        let scene = crowded(50);
        let start = std::time::Instant::now();
        let rects = crate::index::build_rects(&scene);
        let _ = overlap_pairs(&rects);
        let legacy_pairs = xyz_pairs(&scene);
        let legacy_us = std::time::Instant::now()
            .duration_since(start)
            .as_micros()
            .max(1);
        let start = std::time::Instant::now();
        let shipped = pairs3(&scene);
        let shipped_us = std::time::Instant::now()
            .duration_since(start)
            .as_micros()
            .max(1);
        eprintln!("pair-phase 50 crowded: legacy={legacy_us}us shipped={shipped_us}us");
        assert_eq!(shipped, legacy_pairs);
        assert!(
            shipped_us <= 2 * legacy_us,
            "no regression at 50: shipped {shipped_us}us vs legacy {legacy_us}us"
        );
    }

    #[test]
    #[ignore = "timing: 10k queries p95 <= 1ms, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_query_10k_p95_1ms() {
        let scene = scatter(10_000);
        let tree = Rstar::build(&scene);
        let grid = Grid3::build(&scene);
        assert_eq!(tree.len(), 10_000);
        assert_eq!(grid.len(), 10_000);
        // Cross-check one query before timing: both flavors agree.
        let probe = [200, 0, 200];
        let probe_max = [400, 200, 400];
        assert_eq!(
            tree.query_box(probe, probe_max).expect("valid query"),
            grid.query_box(probe, probe_max).expect("valid query")
        );
        let mut samples: Vec<u128> = Vec::with_capacity(1000);
        for i in 0..1000 {
            // Deterministic sweep of box + column queries across the arena.
            let c = (i * 37) * 100;
            let start = std::time::Instant::now();
            let _ = tree
                .query_box([c, 0, c], [c + 200, 200, c + 200])
                .expect("valid query");
            let _ = tree.column(c + 50, c + 50);
            samples.push(std::time::Instant::now().duration_since(start).as_nanos());
        }
        samples.sort_unstable();
        let p95 = samples[950];
        // Grid side of the query comparison, same sweep.
        let mut grid_samples: Vec<u128> = Vec::with_capacity(1000);
        for i in 0..1000 {
            let c = (i * 37) * 100;
            let start = std::time::Instant::now();
            let _ = grid
                .query_box([c, 0, c], [c + 200, 200, c + 200])
                .expect("valid query");
            let _ = grid.column(c + 50, c + 50);
            grid_samples.push(std::time::Instant::now().duration_since(start).as_nanos());
        }
        grid_samples.sort_unstable();
        eprintln!(
            "query 10k rstar: p50={}ns p95={}ns max={}ns; grid: p50={}ns p95={}ns max={}ns",
            samples[500],
            p95,
            samples[999],
            grid_samples[500],
            grid_samples[950],
            grid_samples[999]
        );
        assert!(
            p95 <= 1_000_000,
            "p95 {p95}ns must stay within 1 ms at 10k objects"
        );
        // Stacking stress: a 1000-crate tower in one XZ column. Both flavors
        // must answer a mid-tower slice and the full column fast.
        let tower: Vec<Actor> = (0..1000)
            .map(|i| {
                mk(
                    30_000 + i,
                    "tower_crate",
                    60.0,
                    0.5 + i as f32,
                    60.0,
                    1.0,
                    1.0,
                    1.0,
                )
            })
            .collect();
        let tower_tree = Rstar::build(&tower);
        let tower_grid = Grid3::build(&tower);
        assert_eq!(tower_tree.column(6000, 6000).len(), 1000);
        let start = std::time::Instant::now();
        let tree_slice = tower_tree
            .query_box([5950, 49000, 5950], [6050, 51000, 6050])
            .expect("valid query");
        let tree_slice_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let grid_slice = tower_grid
            .query_box([5950, 49000, 5950], [6050, 51000, 6050])
            .expect("valid query");
        let grid_slice_us = std::time::Instant::now().duration_since(start).as_micros();
        eprintln!(
            "tower slice rstar={tree_slice_us}us (n={}) grid={grid_slice_us}us (n={})",
            tree_slice.len(),
            grid_slice.len()
        );
        assert_eq!(tree_slice, grid_slice);
        assert_eq!(tree_slice.len(), 20);
    }

    #[test]
    #[ignore = "timing: bytes x2 at 10k, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_bytes_10k_stable() {
        let scene = scatter(10_000);
        let a = Rstar::build(&scene);
        let start = std::time::Instant::now();
        let bytes = a.to_bytes();
        let enc_us = std::time::Instant::now().duration_since(start).as_micros();
        let start = std::time::Instant::now();
        let back = Rstar::from_bytes(&bytes).expect("round-trip decodes");
        let dec_us = std::time::Instant::now().duration_since(start).as_micros();
        eprintln!(
            "bytes 10k: size={} enc={enc_us}us dec={dec_us}us digest={:x}",
            bytes.len(),
            a.digest()
        );
        assert_eq!(bytes.len(), 12 + 10_000 * 28);
        assert_eq!(Rstar::build(&scene).to_bytes(), bytes);
        assert_eq!(back.digest(), a.digest());
        assert_eq!(back.pairs().len(), a.pairs().len());
    }

    #[test]
    #[ignore = "timing: place solve p95 < 50ms on 500, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_place_500_under_50ms() {
        // 500 scattered props plus a pebble that anchors the room corner
        // far from the cluster; a point-anchored intent at the free corner
        // exits the candidate scan after a handful of verifies.
        let mut actors = vec![mk(1, "pebble", -20.0, 0.15, -20.0, 0.3, 0.3, 0.3)];
        for (i, p) in scatter(500).into_iter().enumerate() {
            let mut prop = p;
            prop.id = 10 + i as u32;
            prop.name = format!("prop_{i:03}");
            prop.x += 6.0;
            prop.z -= 12.0;
            actors.push(prop);
        }
        actors.push(mk(9000, "brazier", 0.0, 0.5, 20.0, 0.6, 1.0, 0.6));
        // Corner math (GRID_CM pitch, 30 cm half-extent): bounds min comes
        // from the 0.3 m pebble (-2015 cm). Columns 1-3 overlap the pebble
        // and sit within 20 cm of the point, so the distance minimum of
        // 25 cm fails them on wants (nanoseconds); column 4 merely touches
        // the pebble faces and verifies on the first try.
        let intent = Intent {
            object: ObjectRef::Id(9000),
            wants: vec![crate::place::Want {
                relation: Relation::NextTo,
                target: Target::Point {
                    x_cm: -1985,
                    z_cm: -1985,
                },
                distance_cm: Some((25, 65)),
            }],
        };
        let first = crate::place::solve(&intent, &actors).expect("intent solves");
        assert_eq!(first.len(), 1);
        // Pinned: the scan must exit at the pinned corner (column 4, row 1
        // of the candidate grid), not wander the room.
        assert!(
            (first[0].x + 19.55).abs() < 0.06 && (first[0].z + 19.85).abs() < 0.06,
            "solution must sit at the pinned corner, got {:?}",
            first[0]
        );
        // The solution reads clear on the scene-minus-object index: the
        // solver-behind-index3 contract, at 500-object scale.
        let rest: Vec<Actor> = actors.iter().filter(|a| a.id != 9000).cloned().collect();
        let index = Index3::build(&rest);
        let moved = mk(
            9000, "brazier", first[0].x, first[0].y, first[0].z, 0.6, 1.0, 0.6,
        );
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!index.overlaps_any(&probe, 9000));
        let mut samples: Vec<u128> = Vec::with_capacity(21);
        for _ in 0..21 {
            let start = std::time::Instant::now();
            let outs = crate::place::solve(&intent, &actors).expect("intent solves");
            samples.push(std::time::Instant::now().duration_since(start).as_micros());
            assert_eq!(outs, first);
        }
        samples.sort_unstable();
        let p95 = samples[19];
        eprintln!("place 500: p50={}us p95={}us", samples[10], p95);
        assert!(p95 < 50_000, "p95 {p95}us must stay under 50 ms");
        // Index-backed probe batch: what a wired verify would cost per
        // candidate in a dense room (dense rooms need the place.rs verify
        // wiring; the probe itself is microseconds at 500 objects).
        let mut probe_samples: Vec<u128> = Vec::with_capacity(1000);
        for i in 0..1000 {
            let c = -1980 + (i % 40) * 10;
            let q = probe_of(c, 50, -1980, 30, 50, 30);
            let start = std::time::Instant::now();
            let _ = index.overlaps_any(&q, 9000);
            probe_samples.push(std::time::Instant::now().duration_since(start).as_nanos());
        }
        probe_samples.sort_unstable();
        eprintln!(
            "index probe 500: p50={}ns p95={}ns",
            probe_samples[500], probe_samples[950]
        );
        assert!(
            probe_samples[950] < 1_000_000,
            "probe p95 must stay sub-millisecond"
        );
    }

    // -- incremental dirty-subtree rebuild -------------------------------------

    /// Mirror upsert: apply the same row to the oracle scene the index saw.
    fn mirror_upsert(scene: &mut Vec<Actor>, row: Actor) {
        if let Some(slot) = scene.iter_mut().find(|a| a.id == row.id) {
            *slot = row;
        } else {
            scene.push(row);
        }
    }

    #[test]
    fn index3_sp15_revision_records_dirty_subtrees() {
        // One salsa generation per mutating edit; no-ops hold the revision.
        let mut scene = overlap_scene();
        let mut tree = Rstar::build(&scene);
        assert_eq!(tree.revision(), 0);
        assert_eq!(tree.rebuilds(), 0);
        assert!(tree.last_dirty().is_empty());
        // No-op: an identical row plus a missing removal id changes nothing.
        tree.apply(std::slice::from_ref(&scene[0]), &[4242]);
        assert_eq!(tree.revision(), 0);
        assert_eq!(tree.rebuilds(), 0);
        assert!(tree.last_dirty().is_empty());
        assert_eq!(tree.to_bytes(), Rstar::build(&scene).to_bytes());
        // Single move: one revision, exactly the dirty id, full-build equal.
        let moved = resting(1, "b", 30.0, 30.0);
        tree.apply(std::slice::from_ref(&moved), &[]);
        mirror_upsert(&mut scene, moved);
        assert_eq!(tree.revision(), 1);
        assert_eq!(tree.rebuilds(), 1);
        assert_eq!(tree.last_dirty(), vec![1]);
        let full = Rstar::build(&scene);
        assert_eq!(tree.digest(), full.digest());
        assert_eq!(tree.to_bytes(), full.to_bytes());
        assert_eq!(tree.pairs(), full.pairs());
        assert_eq!(tree.pairs_xz(), full.pairs_xz());
        // Batch: one live move, one degenerate delete (NaN drops its row)
        // and one id removal still stamp a single revision over the rebuilt
        // ids only.
        let mut gone = resting(2, "c", 5.0, 5.0);
        gone.x = f32::NAN;
        let moved0 = resting(0, "a", 1.0, 1.0);
        tree.apply(&[moved0.clone(), gone], &[3]);
        mirror_upsert(&mut scene, moved0);
        scene.retain(|a| a.id != 2 && a.id != 3);
        assert_eq!(tree.revision(), 2);
        assert_eq!(tree.rebuilds(), 2);
        assert_eq!(tree.last_dirty(), vec![0, 2, 3]);
        let full2 = Rstar::build(&scene);
        assert_eq!(tree.digest(), full2.digest());
        assert_eq!(tree.to_bytes(), full2.to_bytes());
        assert_eq!(tree.pairs(), full2.pairs());
        // Inverted queries still refuse loudly after edits, never an empty
        // pass that hides a caller bug.
        assert_eq!(
            tree.query_box([1, 1, 1], [0, 0, 0]).unwrap_err(),
            IndexError::InvertedBox
        );
        // Missing remove and identical insert stay quiet with no revision.
        assert!(!tree.remove(4242));
        assert_eq!(tree.revision(), 2);
        let same0 = live_boxes(&scene)
            .into_iter()
            .find(|b| b.id == 0)
            .expect("id 0 live");
        let before = tree.to_bytes();
        tree.insert(same0);
        assert_eq!(tree.revision(), 2);
        assert_eq!(tree.to_bytes(), before);
        // Decode restarts the session record without touching entry bytes.
        let back = Rstar::from_bytes(&before).expect("round-trip decodes");
        assert_eq!(back.revision(), 0);
        assert_eq!(back.rebuilds(), 0);
        assert!(back.last_dirty().is_empty());
        assert_eq!(back.digest(), tree.digest());
        // Grid mirrors the record: no-op holds, move stamps exactly its id,
        // and cross-flavor bytes stay identical.
        let mut gscene = overlap_scene();
        let mut grid = Grid3::build(&gscene);
        assert_eq!(grid.revision(), 0);
        grid.apply(std::slice::from_ref(&gscene[0]), &[4242]);
        assert_eq!(grid.revision(), 0);
        assert!(grid.last_dirty().is_empty());
        let gmoved = resting(1, "b", 30.0, 30.0);
        grid.apply(std::slice::from_ref(&gmoved), &[]);
        mirror_upsert(&mut gscene, gmoved);
        assert_eq!(grid.revision(), 1);
        assert_eq!(grid.rebuilds(), 1);
        assert_eq!(grid.last_dirty(), vec![1]);
        assert_eq!(grid.to_bytes(), Grid3::build(&gscene).to_bytes());
        assert_eq!(grid.to_bytes(), Rstar::build(&gscene).to_bytes());
    }

    #[test]
    fn index3_sp15_100_sequential_edits_equal_full_build() {
        // Acceptance: 10k-object scene, 100 sequential edits, every probe
        // equals the full-build probe; bytes x2 at the end.
        let mut scene = scatter(10_000);
        let mut tree = Rstar::build(&scene);
        let mut grid = Grid3::build(&scene);
        assert_eq!(tree.len(), 10_000);
        assert_eq!(grid.len(), 10_000);
        for step in 0..100u32 {
            // Moves stay under id 9000, removals drain 9000..9009, adds use
            // fresh 50000+ ids: every edit is a real entry-set change, so
            // each one must stamp exactly one revision.
            let (changed, removed) = if step % 10 == 9 {
                let drop_id = 9000 + step / 10;
                let fresh = mk(
                    50_000 + step,
                    "newcomer",
                    -40.0 + step as f32,
                    0.5,
                    -40.0,
                    1.0,
                    1.0,
                    1.0,
                );
                (vec![fresh], vec![drop_id])
            } else {
                let id = (step * 7919) % 9000;
                let slot = scene
                    .iter()
                    .position(|a| a.id == id)
                    .expect("move target live");
                let mut row = scene[slot].clone();
                row.x += 0.5 + (step % 7) as f32 * 0.5;
                row.z -= 0.25;
                (vec![row], Vec::new())
            };
            for row in &changed {
                mirror_upsert(&mut scene, row.clone());
            }
            scene.retain(|a| !removed.contains(&a.id));
            tree.apply(&changed, &removed);
            grid.apply(&changed, &removed);
            let mut dirty: Vec<u32> = changed.iter().map(|a| a.id).collect();
            dirty.extend(removed.iter().copied());
            dirty.sort_unstable();
            assert_eq!(
                tree.revision(),
                u64::from(step) + 1,
                "step {step}: revision"
            );
            assert_eq!(
                grid.revision(),
                u64::from(step) + 1,
                "step {step}: revision"
            );
            assert_eq!(tree.last_dirty(), dirty, "step {step}: dirty ids");
            assert_eq!(grid.last_dirty(), dirty, "step {step}: grid dirty ids");
            let full_tree = Rstar::build(&scene);
            let full_grid = Grid3::build(&scene);
            assert_eq!(tree.digest(), full_tree.digest(), "step {step}: digest");
            assert_eq!(
                grid.digest(),
                full_grid.digest(),
                "step {step}: grid digest"
            );
            assert_eq!(tree.to_bytes(), full_tree.to_bytes(), "step {step}: bytes");
            assert_eq!(
                tree.to_bytes(),
                grid.to_bytes(),
                "step {step}: cross-flavor bytes"
            );
            assert_eq!(tree.pairs(), full_tree.pairs(), "step {step}: pairs");
            assert_eq!(
                tree.pairs_xz(),
                full_tree.pairs_xz(),
                "step {step}: pairs_xz"
            );
            assert_eq!(grid.pairs(), full_grid.pairs(), "step {step}: grid pairs");
            let c = (step as i32 * 3700) % 18000;
            let probe_min = [c, 0, c];
            let probe_max = [c + 200, 200, c + 200];
            assert_eq!(
                tree.query_box(probe_min, probe_max).expect("valid query"),
                full_tree
                    .query_box(probe_min, probe_max)
                    .expect("valid query"),
                "step {step}: query_box"
            );
            assert_eq!(
                grid.query_box(probe_min, probe_max).expect("valid query"),
                full_grid
                    .query_box(probe_min, probe_max)
                    .expect("valid query"),
                "step {step}: grid query_box"
            );
            assert_eq!(
                tree.column(c + 50, c + 50),
                full_tree.column(c + 50, c + 50),
                "step {step}: column"
            );
            assert_eq!(
                tree.band(c, c + 100),
                full_tree.band(c, c + 100),
                "step {step}: band"
            );
            let moved_probe = probe_actor(&changed[0]).expect("changed row live");
            assert_eq!(
                tree.overlaps_any(&moved_probe, changed[0].id),
                full_tree.overlaps_any(&moved_probe, changed[0].id),
                "step {step}: overlaps_any"
            );
        }
        // Bytes x2 after the edit run: identical bytes twice, both flavors.
        assert_eq!(tree.to_bytes(), Rstar::build(&scene).to_bytes());
        assert_eq!(tree.digest(), Rstar::build(&scene).digest());
        assert_eq!(grid.to_bytes(), tree.to_bytes());
        assert_eq!(grid.digest(), tree.digest());
    }

    #[test]
    #[ignore = "timing: incremental rebuild p95 < 2ms at 10k, release --perf only"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn timing_incremental_rebuild_10k_p95_2ms() {
        let mut scene = scatter(10_000);
        let mut tree = Rstar::build(&scene);
        let mut grid = Grid3::build(&scene);
        let mut tree_samples: Vec<u128> = Vec::with_capacity(100);
        let mut grid_samples: Vec<u128> = Vec::with_capacity(100);
        for step in 0..100u32 {
            let id = (step * 7919) % 10_000;
            let slot = scene
                .iter()
                .position(|a| a.id == id)
                .expect("move target live");
            scene[slot].x += 0.5;
            let row = scene[slot].clone();
            let start = std::time::Instant::now();
            tree.apply(std::slice::from_ref(&row), &[]);
            tree_samples.push(std::time::Instant::now().duration_since(start).as_nanos());
            let start = std::time::Instant::now();
            grid.apply(std::slice::from_ref(&row), &[]);
            grid_samples.push(std::time::Instant::now().duration_since(start).as_nanos());
            assert_eq!(tree.revision(), u64::from(step) + 1);
        }
        tree_samples.sort_unstable();
        grid_samples.sort_unstable();
        eprintln!(
            "incremental rebuild 10k rstar: p50={}ns p95={}ns max={}ns rev={}; grid: p50={}ns p95={}ns max={}ns",
            tree_samples[50],
            tree_samples[95],
            tree_samples[99],
            tree.revision(),
            grid_samples[50],
            grid_samples[95],
            grid_samples[99]
        );
        assert_eq!(tree.digest(), Rstar::build(&scene).digest());
        assert_eq!(tree.to_bytes(), Rstar::build(&scene).to_bytes());
        assert_eq!(grid.to_bytes(), tree.to_bytes());
        assert!(
            tree_samples[95] < 2_000_000,
            "rebuild p95 {}ns must stay under 2 ms at 10k objects",
            tree_samples[95]
        );
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn index3_sp21_edit_updates_free_field_under_1ms() {
        // SP21 slice 1: one arena-fixture edit refreshes the incremental
        // free-space field in under 1 ms; the full rebuild stays the oracle.
        // SP16 stacking reads the field through `overlaps_any`; flat rooms
        // keep their pairs and bytes.
        let arena = vec![
            mk(0, "wall_north", 0.0, 1.0, -5.0, 9.8, 2.0, 0.2),
            mk(1, "wall_south", 0.0, 1.0, 5.0, 9.8, 2.0, 0.2),
            mk(2, "wall_west", -5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(3, "wall_east", 5.0, 1.0, 0.0, 0.2, 2.0, 10.0),
            mk(4, "gate", 3.0, 0.5, -4.5, 1.0, 1.0, 0.5),
            mk(5, "brazier", 0.0, 0.5, 2.0, 0.6, 1.0, 0.6),
        ];
        let mut tree = Rstar::build(&arena);
        assert_eq!(tree.free, rebuild_free(&tree.boxes));
        let moved = mk(5, "brazier", 1.0, 0.5, 2.0, 0.6, 1.0, 0.6);
        let start = std::time::Instant::now();
        tree.apply(std::slice::from_ref(&moved), &[]);
        let elapsed = start.elapsed().as_nanos();
        assert!(
            elapsed < 1_000_000,
            "arena edit {elapsed}ns must land under 1 ms"
        );
        let mut expect = arena.clone();
        expect[5] = moved.clone();
        let full_tree = Rstar::build(&expect);
        assert_eq!(tree.free, full_tree.free);
        assert_eq!(tree.free, rebuild_free(&tree.boxes));
        assert_eq!(tree.pairs(), full_tree.pairs());
        assert_eq!(tree.to_bytes(), full_tree.to_bytes());
        // SP16 path: the moved probe reads clear on scene-minus-object.
        let rest: Vec<Actor> = expect.iter().filter(|a| a.id != 5).cloned().collect();
        let scene_minus = Rstar::build(&rest);
        let probe = probe_actor(&moved).expect("valid probe");
        assert!(!scene_minus.overlaps_any(&probe, 5));
        assert!(!Rstar::build(&rest).overlaps_any(&probe, 5));
        // Failure case: a NaN move deletes its row and frees its tiles.
        let mut gone = moved.clone();
        gone.x = f32::NAN;
        tree.apply(std::slice::from_ref(&gone), &[]);
        assert_eq!(tree.free, rebuild_free(&tree.boxes));
        assert!(!tree.boxes.contains_key(&5));
    }
}
