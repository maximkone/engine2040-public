//! breach: door panels and thin walls as bonded collections, plus the ram,
//! shotgun and charge break sets that open them.
//!
//! Consumer: the door's break verbs and the match breach event. Data owner:
//! `engine-destruction`. Units: meters and ticks in, integer millimeters for
//! the debris seeds out. Determinism: fixed tool rules, one seeded spread
//! stream, index-order scans and integer tile math, so the same bake plus the
//! same hit breaks the same chunks twice alike. Errors: [`BreachError`].
//! Budget: a breach is an off-tick event and allocates its report; the debris
//! ledger applies the per-event cap and keeps the sleep islands.

use crate::bake::BREAKING_THRESHOLD;
use crate::bond_grade::{break_record, BreakRecord, GradeError};
use crate::bonded::{
    event_bytes, slot, wire, Bond, BondedCollection, BondedDesc, BreakError, BreakEvent, Chunk3,
    DEFAULT_BOND_STRAIN, MAX_BONDED_CHUNKS,
};
use crate::budget::{AdmitReport, BudgetError, DebrisLedger, ShardSeed};
use crate::csg::dist2;
use crate::structure::{MAX_BAYS, MAX_DENSITY, MAX_SPEC_LEN_M, MAX_STORIES};
use engine_core::checked::{saturating_f64_to_u64, saturating_i64_to_i32, saturating_u32_to_u16};
use engine_core::{Fnv64, Rng};

/// Nav tile edge used when a caller names no other; meters.
pub const DEFAULT_TILE_M: f32 = 0.5;
/// Millimeters per meter for debris seed positions.
pub const MM_PER_M: f64 = 1000.0;
/// Impulse of a ram over the hit chunk's own break gate.
pub const RAM_FORCE: f32 = 3.0;
/// Impulse of one shotgun pellet over the hit chunk's own break gate.
pub const PELLET_FORCE: f32 = 1.5;
/// Most pellets one shotgun call accepts.
pub const MAX_PELLETS: u32 = 16;
/// Widest pellet spread from the aim point; meters.
pub const MAX_SPREAD_M: f32 = 1.0;
/// Largest charge radius; meters.
pub const MAX_RADIUS_M: f32 = 5.0;
/// Shift of the saturating float-to-integer coordinate conversion.
const COORD_SHIFT: f64 = 1e9;
/// Same shift as the integer the subtraction uses.
const COORD_SHIFT_I64: i64 = 1_000_000_000;
/// Smallest accepted length, thickness, spread or radius; meters.
const MIN_DIM_M: f32 = 1e-3;

/// A door panel: a thin slab hung on the anchored first column (the hinge
/// stile). The last column is the free latch stile the tools aim at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorSpec {
    /// Panel width across the stiles; meters.
    pub width_m: f32,
    /// Panel height; meters.
    pub height_m: f32,
    /// Panel thickness; meters.
    pub thickness_m: f32,
    /// Chunk columns from the hinge stile to the latch stile.
    pub cols: u32,
    /// Chunk rows from the floor to the head.
    pub rows: u32,
    /// Mass per cubic meter before chunking.
    pub density: f32,
}

impl DoorSpec {
    /// A 0.9 m by 2.0 m by 4 cm wooden door in three by four chunks.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            width_m: 0.9,
            height_m: 2.0,
            thickness_m: 0.04,
            cols: 3,
            rows: 4,
            density: 500.0,
        }
    }
}

/// A thin wall: a slab fixed into both jamb columns, so a hole keeps the
/// remaining bays standing on the sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallSpec {
    /// Wall length along x; meters.
    pub length_m: f32,
    /// Wall height; meters.
    pub height_m: f32,
    /// Wall thickness; meters.
    pub thickness_m: f32,
    /// Chunk bays along the length.
    pub bays: u32,
    /// Chunk storeys from the floor to the head.
    pub storeys: u32,
    /// Mass per cubic meter before chunking.
    pub density: f32,
}

impl WallSpec {
    /// A 4 m by 2.4 m by 12 cm plaster wall in eight by four chunks.
    #[must_use]
    pub fn standard() -> Self {
        Self {
            length_m: 4.0,
            height_m: 2.4,
            thickness_m: 0.12,
            bays: 8,
            storeys: 4,
            density: 800.0,
        }
    }
}

/// What opens a door or wall panel. Forces scale with the target's own break
/// gate, so one tool acts the same on a light door and a heavy wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BreachTool {
    /// One heavy impact on the nearest chunk; the latch bonds snap.
    Ram,
    /// A seeded cone of pellets around the aim point.
    Shotgun {
        /// Pellet count; `1..=MAX_PELLETS`.
        pellets: u32,
        /// Radius of the cone; meters.
        spread_m: f32,
    },
    /// A cutting charge that severs every chunk inside a sphere.
    Charge {
        /// Sphere radius; meters, `MIN_DIM_M..=MAX_RADIUS_M`.
        radius_m: f32,
    },
}

impl BreachTool {
    /// Range-checks the payload without touching a collection.
    fn validate(&self) -> Result<(), BreachError> {
        match *self {
            Self::Ram => Ok(()),
            Self::Shotgun { pellets, spread_m } => {
                let spread_ok =
                    spread_m.is_finite() && (MIN_DIM_M..=MAX_SPREAD_M).contains(&spread_m);
                if pellets == 0 || pellets > MAX_PELLETS || !spread_ok {
                    return Err(BreachError::BadPayload);
                }
                Ok(())
            }
            Self::Charge { radius_m } => {
                let radius_ok =
                    radius_m.is_finite() && (MIN_DIM_M..=MAX_RADIUS_M).contains(&radius_m);
                if radius_ok {
                    Ok(())
                } else {
                    Err(BreachError::BadPayload)
                }
            }
        }
    }
}

/// Why a breach or a panel build was refused; input never panics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BreachError {
    /// A dimension, density, grid count or anchor layout is out of range.
    BadSpec,
    /// The grid emits more chunks than one bonded collection holds.
    TooManyChunks,
    /// The aim point is not finite.
    BadHit,
    /// A tool payload is zero, past its cap or not finite.
    BadPayload,
    /// The tool found no chunk to act on.
    NothingInReach,
    /// The nav tile edge is zero, negative or not finite.
    BadTile,
    /// The bonded descriptor or collection refused the shape.
    Bonded(BreakError),
    /// The break record refused an event or ran past its tile bound.
    Grade(GradeError),
    /// The debris ledger refused the seeds or their cap.
    Budget(BudgetError),
}

impl core::fmt::Display for BreachError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BreachError::BadSpec => write!(f, "breach panel spec is out of bounds"),
            BreachError::TooManyChunks => write!(f, "breach grid exceeds MAX_BONDED_CHUNKS"),
            BreachError::BadHit => write!(f, "breach aim point is not finite"),
            BreachError::BadPayload => write!(f, "breach tool payload is out of bounds"),
            BreachError::NothingInReach => write!(f, "breach tool reaches no chunk"),
            BreachError::BadTile => write!(f, "breach tile edge is out of bounds"),
            BreachError::Bonded(e) => write!(f, "breach bonded shape refused: {e}"),
            BreachError::Grade(e) => write!(f, "breach record refused: {e}"),
            BreachError::Budget(e) => write!(f, "breach debris refused: {e}"),
        }
    }
}

impl std::error::Error for BreachError {}

impl From<BreakError> for BreachError {
    fn from(e: BreakError) -> Self {
        BreachError::Bonded(e)
    }
}

impl From<GradeError> for BreachError {
    fn from(e: GradeError) -> Self {
        BreachError::Grade(e)
    }
}

impl From<BudgetError> for BreachError {
    fn from(e: BudgetError) -> Self {
        BreachError::Budget(e)
    }
}

/// What one breach observably produced: the collection after the tool, the
/// chunks it acted on, its break events, the canonical record with the dirty
/// nav tiles, and one debris seed per freed chunk.
#[derive(Clone, Debug)]
pub struct BreachOutcome {
    /// Collection after the tool; anchored chunks never free.
    pub field: BondedCollection,
    /// Chunks the tool acted on, ascending.
    pub struck: Vec<u32>,
    /// Emitted break events in emission order.
    pub events: Vec<BreakEvent>,
    /// Snapped bonds, freed chunks and dirty nav tiles.
    pub record: BreakRecord,
    /// One seed per freed chunk, ready for a debris ledger admit.
    pub debris: Vec<ShardSeed>,
    /// Canonical digest of the events and the dirty tiles.
    pub digest: u64,
}

impl BreachOutcome {
    /// Admits the freed chunks into the debris ledger, which applies the
    /// per-event cap, the ledger bound and sleep-island tracking.
    pub fn admit_debris(
        &self,
        ledger: &mut DebrisLedger,
        tick: u32,
    ) -> Result<AdmitReport, BreachError> {
        ledger
            .admit(tick, &self.debris)
            .map_err(BreachError::Budget)
    }
}

/// Builds a door panel as a bonded collection: row-major chunks, contact
/// bonds at the default strain, and the hinge stile anchored.
pub fn door_panel(spec: &DoorSpec) -> Result<BondedDesc, BreachError> {
    slab(
        spec.width_m,
        spec.height_m,
        spec.thickness_m,
        spec.cols,
        spec.rows,
        spec.density,
        AnchorRule::HingeStile,
    )
}

/// Builds a thin wall as a bonded collection: row-major chunks, contact
/// bonds at the default strain, and both jamb columns anchored.
pub fn thin_wall(spec: &WallSpec) -> Result<BondedDesc, BreachError> {
    slab(
        spec.length_m,
        spec.height_m,
        spec.thickness_m,
        spec.bays,
        spec.storeys,
        spec.density,
        AnchorRule::Jambs,
    )
}

/// Which columns of a flat slab stay anchored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnchorRule {
    /// Only the first column: the hinge stile of a door.
    HingeStile,
    /// The first and last columns: the jambs of a wall.
    Jambs,
}

/// One flat grid of box chunks in the x/y plane with contact bonds and the
/// anchor column rule applied. Chunk order is row-major from the floor up.
fn slab(
    width_m: f32,
    height_m: f32,
    thickness_m: f32,
    cols: u32,
    rows: u32,
    density: f32,
    rule: AnchorRule,
) -> Result<BondedDesc, BreachError> {
    if cols == 0 || rows == 0 || cols > MAX_BAYS || rows > MAX_STORIES {
        return Err(BreachError::BadSpec);
    }
    for dim in [width_m, height_m, thickness_m] {
        if !dim.is_finite() || !(MIN_DIM_M..=MAX_SPEC_LEN_M).contains(&dim) {
            return Err(BreachError::BadSpec);
        }
    }
    if !density.is_finite() || density <= 0.0 || density > MAX_DENSITY {
        return Err(BreachError::BadSpec);
    }
    let cells = u64::from(cols) * u64::from(rows);
    if cells > u64::try_from(MAX_BONDED_CHUNKS).unwrap_or(u64::MAX) {
        return Err(BreachError::TooManyChunks);
    }
    let cell_w = width_m / f32::from(saturating_u32_to_u16(cols));
    let cell_h = height_m / f32::from(saturating_u32_to_u16(rows));
    let half_t = thickness_m * 0.5;
    let mass = cell_w * cell_h * thickness_m * density;
    let mut chunks = Vec::with_capacity(slot(cols) * slot(rows));
    for row in 0..rows {
        for col in 0..cols {
            let fx = f32::from(saturating_u32_to_u16(col)) + 0.5;
            let fy = f32::from(saturating_u32_to_u16(row)) + 0.5;
            chunks.push(Chunk3 {
                x: -width_m * 0.5 + cell_w * fx,
                y: cell_h * fy,
                z: 0.0,
                hx: cell_w * 0.5,
                hy: cell_h * 0.5,
                hz: half_t,
                mass,
            });
        }
    }
    let mut bonds = Vec::new();
    let mut anchors = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col;
            if col + 1 < cols {
                bonds.push(Bond {
                    a: index,
                    b: index + 1,
                    breaking_strain: DEFAULT_BOND_STRAIN,
                });
            }
            if row + 1 < rows {
                bonds.push(Bond {
                    a: index,
                    b: index + cols,
                    breaking_strain: DEFAULT_BOND_STRAIN,
                });
            }
            let anchored = match rule {
                AnchorRule::HingeStile => col == 0,
                AnchorRule::Jambs => col == 0 || col + 1 == cols,
            };
            if anchored {
                anchors.push(index);
            }
        }
    }
    let desc = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold: BREAKING_THRESHOLD,
    };
    desc.validate()?;
    Ok(desc)
}

/// Applies one tool to a validated descriptor and returns the outcome: the
/// collection after the tool, the events, the dirty tiles and the debris.
///
/// The hit is the aim point in meters. Ram acts on the nearest chunk, the
/// shotgun draws its cone from `seed` around the aim point, and a charge
/// severs every chunk center inside its sphere. All three run once per event,
/// off the tick.
///
/// Errors: [`BreachError`] for a bad descriptor, payload, aim point or tile
/// edge, and for a tool that reaches no chunk.
pub fn breach(
    desc: &BondedDesc,
    tool: BreachTool,
    hit: [f32; 3],
    tick: u32,
    seed: u64,
    tile_m: f32,
) -> Result<BreachOutcome, BreachError> {
    desc.validate()?;
    tool.validate()?;
    if hit.iter().any(|v| !v.is_finite()) {
        return Err(BreachError::BadHit);
    }
    if !tile_m.is_finite() || tile_m <= 0.0 {
        return Err(BreachError::BadTile);
    }
    let mut field = BondedCollection::new(desc)?;
    let struck = apply_tool(&mut field, tool, hit, seed, tick)?;
    let events = field.drain_events();
    let record = break_record(desc, &events, tile_m)?;
    let debris = debris_seeds(desc, &record.freed);
    let digest = breach_digest(&events, &record.dirty_tiles);
    Ok(BreachOutcome {
        field,
        struck,
        events,
        record,
        debris,
        digest,
    })
}

/// Dispatches one tool to its rule.
fn apply_tool(
    field: &mut BondedCollection,
    tool: BreachTool,
    hit: [f32; 3],
    seed: u64,
    tick: u32,
) -> Result<Vec<u32>, BreachError> {
    match tool {
        BreachTool::Ram => {
            let chunk = nearest_chunk(field, hit).ok_or(BreachError::NothingInReach)?;
            hit_chunk(field, chunk, RAM_FORCE, tick)?;
            Ok(vec![wire(chunk)])
        }
        BreachTool::Shotgun { pellets, spread_m } => {
            shotgun(field, hit, pellets, spread_m, seed, tick)
        }
        BreachTool::Charge { radius_m } => charge(field, hit, radius_m, tick),
    }
}

/// Nearest chunk center to `hit`; ties go to the lowest index.
fn nearest_chunk(field: &BondedCollection, hit: [f32; 3]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_d = f32::INFINITY;
    for (i, c) in field.chunks().iter().enumerate() {
        let d = dist2(&[c.x, c.y, c.z], &hit);
        if d < best_d {
            best_d = d;
            best = Some(i);
        }
    }
    best
}

/// Applies an impulse scaled to `force` times the chunk's own break gate.
fn hit_chunk(
    field: &mut BondedCollection,
    chunk: usize,
    force: f32,
    tick: u32,
) -> Result<(), BreachError> {
    let mass = field
        .chunk_mass(chunk)
        .ok_or(BreachError::Bonded(BreakError::BadChunk(chunk)))?;
    let impulse = mass * field.break_threshold() * force;
    field
        .apply_impact(chunk, impulse, tick)
        .map_err(BreachError::Bonded)
}

/// Seeded pellet cone: each pellet draws a millimeter offset around the aim
/// point and hits the nearest chunk. The struck list is ascending and unique
/// even when several pellets land on one chunk.
fn shotgun(
    field: &mut BondedCollection,
    hit: [f32; 3],
    pellets: u32,
    spread_m: f32,
    seed: u64,
    tick: u32,
) -> Result<Vec<u32>, BreachError> {
    let span = to_mm(spread_m).max(0);
    let span_u = u32::try_from(span).unwrap_or(0);
    let draw_span = u64::from(span_u) * 2 + 1;
    let mut rng = Rng::new(seed).fork_named("breach-shotgun");
    let mut struck = Vec::new();
    for _ in 0..pellets {
        let dx = i32::try_from(rng.next_range(0, draw_span)).unwrap_or(0) - span;
        let dy = i32::try_from(rng.next_range(0, draw_span)).unwrap_or(0) - span;
        let point = [hit[0] + mm_to_m(dx), hit[1] + mm_to_m(dy), hit[2]];
        let chunk = nearest_chunk(field, point).ok_or(BreachError::NothingInReach)?;
        hit_chunk(field, chunk, PELLET_FORCE, tick)?;
        let id = wire(chunk);
        if !struck.contains(&id) {
            struck.push(id);
        }
    }
    struck.sort_unstable();
    Ok(struck)
}

/// Cutting charge: severs every chunk center inside the sphere, ascending by
/// index, then refreshes support through the first sever. Anchored chunks may
/// be struck but never free.
fn charge(
    field: &mut BondedCollection,
    hit: [f32; 3],
    radius_m: f32,
    tick: u32,
) -> Result<Vec<u32>, BreachError> {
    let reach = radius_m * radius_m;
    let mut targets = Vec::new();
    for (i, c) in field.chunks().iter().enumerate() {
        if dist2(&[c.x, c.y, c.z], &hit) <= reach {
            targets.push(i);
        }
    }
    if targets.is_empty() {
        return Err(BreachError::NothingInReach);
    }
    for &chunk in &targets {
        field.sever_chunk(chunk, tick)?;
    }
    Ok(targets.into_iter().map(wire).collect())
}

/// One debris seed per freed chunk, in the record's ascending order. Chunks
/// that fell as one connected lump share an island, so the ledger sleeps and
/// wakes them together.
fn debris_seeds(desc: &BondedDesc, freed: &[u32]) -> Vec<ShardSeed> {
    let islands = freed_islands(desc, freed);
    let mut seeds = Vec::with_capacity(freed.len());
    for (&chunk, island) in freed.iter().zip(islands) {
        let Some(c) = desc.chunks.get(slot(chunk)) else {
            continue;
        };
        seeds.push(ShardSeed {
            id: chunk,
            island,
            pos_mm: [to_mm(c.x), to_mm(c.y), to_mm(c.z)],
            vel_mm_per_tick: [0; 3],
        });
    }
    seeds
}

/// Island id of each freed chunk: the lowest chunk index of its connected
/// component over the descriptor bonds. Chunks that no bond joins stand
/// alone. Output order matches `freed`.
fn freed_islands(desc: &BondedDesc, freed: &[u32]) -> Vec<u32> {
    let mut is_freed = vec![false; desc.chunks.len()];
    for &chunk in freed {
        if let Some(at) = is_freed.get_mut(slot(chunk)) {
            *at = true;
        }
    }
    let mut island_of = vec![u32::MAX; desc.chunks.len()];
    let mut stack = Vec::new();
    let mut out = Vec::with_capacity(freed.len());
    let mut next = 0u32;
    for &start in freed {
        let root = slot(start);
        if island_of[root] != u32::MAX {
            out.push(island_of[root]);
            continue;
        }
        let id = next;
        next += 1;
        island_of[root] = id;
        stack.push(root);
        while let Some(current) = stack.pop() {
            for bond in &desc.bonds {
                let (a, b) = (slot(bond.a), slot(bond.b));
                let other = if a == current {
                    b
                } else if b == current {
                    a
                } else {
                    continue;
                };
                if is_freed[other] && island_of[other] == u32::MAX {
                    island_of[other] = id;
                    stack.push(other);
                }
            }
        }
        out.push(id);
    }
    out
}

/// Round meters to integer millimeters, saturating instead of wrapping. The
/// shift keeps the unsigned conversion exact for negative coordinates.
fn to_mm(v: f32) -> i32 {
    let mm = (f64::from(v) * MM_PER_M).round();
    let shifted = saturating_f64_to_u64(mm + COORD_SHIFT);
    saturating_i64_to_i32(i64::try_from(shifted).unwrap_or(0) - COORD_SHIFT_I64)
}

/// Millimeter offset back to meters; pellet offsets stay under one meter.
fn mm_to_m(v: i32) -> f32 {
    f32::from(i16::try_from(v).unwrap_or(0)) * 0.001
}

/// FNV-1a over the event bytes then the dirty tile pairs, in record order.
fn breach_digest(events: &[BreakEvent], tiles: &[[i32; 2]]) -> u64 {
    let mut h = Fnv64::new();
    h.write_len32(events.len());
    for event in events {
        h.write(&event_bytes(event));
    }
    h.write_len32(tiles.len());
    for [x, z] in tiles {
        h.write_u32(u32::from_le_bytes(x.to_le_bytes()));
        h.write_u32(u32::from_le_bytes(z.to_le_bytes()));
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::arena_default_budget;
    use std::collections::{BTreeSet, VecDeque};

    fn door() -> BondedDesc {
        door_panel(&DoorSpec::standard()).expect("a standard door builds")
    }

    fn wall() -> BondedDesc {
        thin_wall(&WallSpec::standard()).expect("a standard wall builds")
    }

    /// Charge the wall at chest height with a meter radius, the proof hit.
    fn wall_charge(desc: &BondedDesc) -> BreachOutcome {
        breach(
            desc,
            BreachTool::Charge { radius_m: 1.0 },
            [0.0, 1.2, 0.0],
            10,
            7,
            DEFAULT_TILE_M,
        )
        .expect("the charge reaches the wall")
    }

    fn add_footprint(c: &Chunk3, tile_m: f32, out: &mut BTreeSet<(i32, i32)>) {
        let tile = |v: f32| crate::bond_grade::floor_to_i32(f64::from(v) / f64::from(tile_m));
        let (x0, x1) = (tile(c.x - c.hx), tile(c.x + c.hx));
        let (z0, z1) = (tile(c.z - c.hz), tile(c.z + c.hz));
        for x in x0..=x1 {
            for z in z0..=z1 {
                out.insert((x, z));
            }
        }
    }

    fn solid_tiles(desc: &BondedDesc, freed: &[u32], tile_m: f32) -> BTreeSet<(i32, i32)> {
        let freed: BTreeSet<u32> = freed.iter().copied().collect();
        let mut out = BTreeSet::new();
        for (i, c) in desc.chunks.iter().enumerate() {
            if !freed.contains(&u32::try_from(i).expect("chunk index fits u32")) {
                add_footprint(c, tile_m, &mut out);
            }
        }
        out
    }

    /// Rebakes only the named tiles: each is cleared, then made solid again
    /// when a chunk that stayed covers it.
    fn rebake_named(
        before: &BTreeSet<(i32, i32)>,
        dirty: &[[i32; 2]],
        desc: &BondedDesc,
        freed: &[u32],
        tile_m: f32,
    ) -> BTreeSet<(i32, i32)> {
        let named: BTreeSet<(i32, i32)> = dirty.iter().map(|&[x, z]| (x, z)).collect();
        let mut after = before.clone();
        for tile in &named {
            after.remove(tile);
        }
        let freed: BTreeSet<u32> = freed.iter().copied().collect();
        let mut cover = BTreeSet::new();
        for (i, c) in desc.chunks.iter().enumerate() {
            if !freed.contains(&u32::try_from(i).expect("chunk index fits u32")) {
                add_footprint(c, tile_m, &mut cover);
            }
        }
        for tile in named {
            if cover.contains(&tile) {
                after.insert(tile);
            }
        }
        after
    }

    fn route_exists(solid: &BTreeSet<(i32, i32)>, start: (i32, i32), goal: (i32, i32)) -> bool {
        let (mut min_x, mut max_x) = (start.0.min(goal.0), start.0.max(goal.0));
        let (mut min_z, mut max_z) = (start.1.min(goal.1), start.1.max(goal.1));
        for &(x, z) in solid {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_z = min_z.min(z);
            max_z = max_z.max(z);
        }
        if solid.contains(&start) || solid.contains(&goal) {
            return false;
        }
        let mut seen = BTreeSet::new();
        seen.insert(start);
        let mut queue = VecDeque::new();
        queue.push_back(start);
        while let Some((x, z)) = queue.pop_front() {
            if (x, z) == goal {
                return true;
            }
            for next in [(x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)] {
                let (nx, nz) = next;
                if nx < min_x || nx > max_x || nz < min_z || nz > max_z {
                    continue;
                }
                if solid.contains(&next) || !seen.insert(next) {
                    continue;
                }
                queue.push_back(next);
            }
        }
        false
    }

    #[test]
    fn a_door_and_a_wall_stand_at_rest_on_their_anchors() {
        for desc in [door(), wall()] {
            assert!(!desc.anchors.is_empty());
            let mut field = BondedCollection::new(&desc).expect("the panel builds");
            field.step(1.0 / 60.0);
            assert!(field.freed_chunks().is_empty(), "nothing frees at rest");
            assert!(field.drain_events().is_empty());
            let anchors = field.anchors();
            assert_eq!(anchors.len(), desc.anchors.len());
            assert!(anchors.iter().all(|a| field.is_anchored(*a as usize)));
        }
    }

    #[test]
    fn a_charge_breaks_the_same_chunks_and_tiles_twice() {
        let desc = wall();
        let first = wall_charge(&desc);
        let second = wall_charge(&desc);
        assert!(!first.record.freed.is_empty(), "the charge opens a hole");
        assert!(!first.record.bonds.is_empty());
        assert!(!first.record.dirty_tiles.is_empty());
        assert_eq!(first.record, second.record);
        assert_eq!(first.events, second.events);
        assert_eq!(first.struck, second.struck);
        assert_eq!(first.debris, second.debris);
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.field.state_hash(), second.field.state_hash());
        assert_eq!(first.record.freed, {
            let mut freed = first.record.freed.clone();
            freed.sort_unstable();
            freed.dedup();
            freed
        });
    }

    #[test]
    fn a_charge_opens_a_route_that_its_dirty_tiles_name() {
        let desc = wall();
        let tile_m = DEFAULT_TILE_M;
        let (start, goal) = ((0, -3), (0, 3));
        let before = solid_tiles(&desc, &[], tile_m);
        assert!(!route_exists(&before, start, goal), "the wall blocks first");

        let out = wall_charge(&desc);
        let after = rebake_named(
            &before,
            &out.record.dirty_tiles,
            &desc,
            &out.record.freed,
            tile_m,
        );
        let named: BTreeSet<(i32, i32)> = out
            .record
            .dirty_tiles
            .iter()
            .map(|&[x, z]| (x, z))
            .collect();
        for tile in before.symmetric_difference(&after) {
            assert!(
                named.contains(tile),
                "tile {tile:?} changed but the break did not name it"
            );
        }
        assert!(route_exists(&after, start, goal), "the hole routes through");

        let again = wall_charge(&desc);
        let after_again = rebake_named(
            &before,
            &again.record.dirty_tiles,
            &desc,
            &again.record.freed,
            tile_m,
        );
        assert_eq!(after, after_again, "the same break rebakes the same tiles");
    }

    #[test]
    fn a_ram_snaps_the_latch_side_and_keeps_every_hinge() {
        let desc = door();
        let hit = [0.45, 1.0, 0.0];
        let first = breach(&desc, BreachTool::Ram, hit, 1, 0, DEFAULT_TILE_M).expect("ram");
        let second = breach(&desc, BreachTool::Ram, hit, 1, 0, DEFAULT_TILE_M).expect("ram");
        assert_eq!(first.struck.len(), 1);
        assert!(!first.record.bonds.is_empty(), "the latch bonds snap");
        assert!(!first.record.freed.is_empty(), "the free stile drops");
        assert!(!first.record.dirty_tiles.is_empty());
        for chunk in &first.record.freed {
            assert!(
                !desc.anchors.contains(chunk),
                "an anchor never frees: {chunk}"
            );
        }
        assert_eq!(first.record, second.record);
        assert_eq!(first.digest, second.digest);
    }

    #[test]
    fn a_shotgun_cone_is_seeded_and_scatters_across_chunks() {
        let desc = door();
        let aim = [0.0, 1.0, 0.0];
        let tool = BreachTool::Shotgun {
            pellets: 8,
            spread_m: 0.3,
        };
        let first = breach(&desc, tool, aim, 1, 42, DEFAULT_TILE_M).expect("shotgun");
        let second = breach(&desc, tool, aim, 1, 42, DEFAULT_TILE_M).expect("shotgun");
        assert!(!first.struck.is_empty());
        assert!(first.struck.windows(2).all(|w| w[0] < w[1]));
        assert!(!first.record.bonds.is_empty());
        assert!(!first.record.dirty_tiles.is_empty());
        assert_eq!(first.struck, second.struck);
        assert_eq!(first.record, second.record);
        assert_eq!(first.events, second.events);
        for chunk in &first.struck {
            assert!(*chunk < u32::try_from(desc.chunks.len()).expect("count fits u32"));
        }
    }

    #[test]
    fn a_big_charge_stays_inside_the_debris_budget() {
        let spec = WallSpec {
            length_m: 8.0,
            height_m: 6.0,
            thickness_m: 0.15,
            bays: 16,
            storeys: 8,
            density: 800.0,
        };
        let desc = thin_wall(&spec).expect("the big wall builds");
        let out = breach(
            &desc,
            BreachTool::Charge { radius_m: 5.0 },
            [0.0, 3.0, 0.0],
            3,
            0,
            DEFAULT_TILE_M,
        )
        .expect("the big charge reaches the wall");
        assert_eq!(out.debris.len(), out.record.freed.len());
        let cfg = arena_default_budget();
        let mut ledger = DebrisLedger::new(&cfg).expect("the ledger builds");
        let report = out.admit_debris(&mut ledger, 3).expect("the debris admits");
        assert_eq!(report.admitted, cfg.per_event_cap);
        assert_eq!(
            report.dropped_cap,
            u32::try_from(out.debris.len()).expect("count fits u32") - cfg.per_event_cap
        );
        assert_eq!(report.dropped_full, 0);
        assert_eq!(ledger.len(), cfg.per_event_cap as usize);
        assert!(ledger.len() <= cfg.max_shards as usize);
        assert_eq!(ledger.admitted_total(), u64::from(cfg.per_event_cap));
        assert!(out.digest != 0);
    }

    #[test]
    fn bad_breach_input_refuses_without_breaking() {
        let desc = door();
        assert_eq!(
            breach(
                &desc,
                BreachTool::Charge { radius_m: 0.0 },
                [0.0, 1.0, 0.0],
                1,
                0,
                DEFAULT_TILE_M
            )
            .unwrap_err(),
            BreachError::BadPayload
        );
        assert_eq!(
            breach(
                &desc,
                BreachTool::Charge { radius_m: 0.5 },
                [9.0, 1.0, 0.0],
                1,
                0,
                DEFAULT_TILE_M
            )
            .unwrap_err(),
            BreachError::NothingInReach
        );
        assert_eq!(
            breach(
                &desc,
                BreachTool::Ram,
                [f32::NAN, 1.0, 0.0],
                1,
                0,
                DEFAULT_TILE_M
            )
            .unwrap_err(),
            BreachError::BadHit
        );
        assert_eq!(
            breach(&desc, BreachTool::Ram, [0.0, 1.0, 0.0], 1, 0, 0.0).unwrap_err(),
            BreachError::BadTile
        );
        assert_eq!(
            breach(
                &desc,
                BreachTool::Shotgun {
                    pellets: MAX_PELLETS + 1,
                    spread_m: 0.2
                },
                [0.0, 1.0, 0.0],
                1,
                0,
                DEFAULT_TILE_M
            )
            .unwrap_err(),
            BreachError::BadPayload
        );
        assert_eq!(
            door_panel(&DoorSpec {
                cols: 0,
                ..DoorSpec::standard()
            })
            .unwrap_err(),
            BreachError::BadSpec
        );
        assert_eq!(
            door_panel(&DoorSpec {
                width_m: f32::NAN,
                ..DoorSpec::standard()
            })
            .unwrap_err(),
            BreachError::BadSpec
        );
        assert_eq!(
            thin_wall(&WallSpec {
                bays: MAX_BAYS + 1,
                ..WallSpec::standard()
            })
            .unwrap_err(),
            BreachError::BadSpec
        );
    }
}
