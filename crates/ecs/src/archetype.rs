//! archetype store: SoA columns per table, swap-remove, generational entities. Same spawns = same hash.

use std::any::TypeId;

use crate::arena::{ArenaError, PhaseArena};
use crate::snapshot::hash_bytes;
use engine_core::Fnv64;

/// Sparse marker storage plus `StorageKind`/`Added`/`Changed` (see `sparse.rs`).
/// Caller: [`World`] holds one [`SparseSet`] for markers; tables stay default.
#[path = "sparse.rs"]
pub mod sparse;

/// Sparse storage discipline and change filters over table stamps.
pub use sparse::{Added, Changed, Marker, SparseSet, StorageKind};

/// SoA position column.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

/// SoA velocity column.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

/// Change-tick stamp.
pub type Tick = u32;

/// Generational entity handle: `id` recycles, `gen` bumps on despawn.
/// Stale handles fail closed in every accessor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entity {
    pub id: u32,
    pub gen: u32,
}

// Ids and rows are `u32` handles widened to indices below; every supported
// target has a pointer at least 32 bits wide, so that widening is lossless.
const _: () = assert!(usize::BITS >= 32);

impl Entity {
    /// Index of this handle's slot in the location table.
    fn slot(self) -> usize {
        self.id as usize
    }
}

const P: u8 = 1;
const V: u8 = 2;

/// True when `mask` carries the `flag` component bit.
fn has_bit(mask: u8, flag: u8) -> bool {
    mask & flag != 0
}

/// Typed component id: one archetype bit per component type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentId(u8);

impl ComponentId {
    /// Bit for [`Position`].
    pub const POS: Self = Self(P);
    /// Bit for [`Velocity`].
    pub const VEL: Self = Self(V);
    /// Raw archetype bit.
    pub fn bit(self) -> u8 {
        self.0
    }
}

/// Typed component registry.
/// `Position` maps to bit 1, `Velocity` to bit 2; unknown types report `None`
/// instead of minting silent bits.
#[derive(Debug)]
pub struct Registry {
    count: u8,
}

impl Registry {
    /// Registry with `Position` + `Velocity` registered.
    pub fn new() -> Self {
        Self { count: 2 }
    }
    /// Number of registered components.
    pub fn len(&self) -> usize {
        usize::from(self.count)
    }
    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    /// Register a component type; known types return their fixed bit.
    pub fn register<T: 'static>(&mut self, _name: &'static str) -> Option<ComponentId> {
        self.bit::<T>()
    }
    /// Bit for a known component type, `None` for unknown types.
    pub fn bit<T: 'static>(&self) -> Option<ComponentId> {
        if TypeId::of::<T>() == TypeId::of::<Position>() {
            Some(ComponentId::POS)
        } else if TypeId::of::<T>() == TypeId::of::<Velocity>() {
            Some(ComponentId::VEL)
        } else {
            None
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Ticks between periodic stamp scans: 1000 ticks per frame at 144 Hz for one
/// hour. Owner: engine-ecs. Units: ticks. Determinism: exact integer bound;
/// when the world advances this far past [`World::last_scan`], the next arena
/// tick clamps every stamp older than [`MAX_CHANGE_AGE`] so no age wrapping
/// difference can overflow `u32` before the following scan.
pub const CHECK_TICK_THRESHOLD: Tick = 518_400_000;

/// Maximum change-tick age that still reports: `u32::MAX` minus twice the
/// scan window plus one. Owner: engine-ecs. Units: ticks. Determinism: exact
/// integer bound; [`is_newer_than`] clamps both sides to this age, so scans
/// that land on different ticks still decide identically.
pub const MAX_CHANGE_AGE: Tick = Tick::MAX - (2 * CHECK_TICK_THRESHOLD - 1);

/// True when `change` happened after `last_run`, both read against `this_run`
/// (three-way compare, Bevy `is_newer_than` shape reimplemented std-only).
/// Units: ticks. Determinism: exact integer math only (`wrapping_sub`, `min`,
/// `>`), no float, no wall clock; wrapping tick values decide identically on
/// every machine. Threading: pure function, safe on any thread.
#[inline]
pub fn is_newer_than(change: Tick, last_run: Tick, this_run: Tick) -> bool {
    let since_change = this_run.wrapping_sub(change).min(MAX_CHANGE_AGE);
    let since_run = this_run.wrapping_sub(last_run).min(MAX_CHANGE_AGE);
    since_run > since_change
}

/// Clamp one stamp to the scan window (Bevy `check_tick` shape reimplemented
/// std-only): stamps older than [`MAX_CHANGE_AGE`] move forward to
/// `this_run - MAX_CHANGE_AGE`, which decides identically under
/// [`is_newer_than`]. Units: ticks. Determinism: exact integer math only.
#[inline]
pub fn clamp_stamp(stamp: Tick, this_run: Tick) -> Tick {
    if this_run.wrapping_sub(stamp) > MAX_CHANGE_AGE {
        this_run.wrapping_sub(MAX_CHANGE_AGE)
    } else {
        stamp
    }
}

/// Legacy two-way compare, frozen: true when `a` was written after `since`
/// (wrapping compare). Kept for the pre-existing `changed_*`/`for_each_*`
/// entry points; new systems use [`is_newer_than`], which also retires stamps
/// older than [`MAX_CHANGE_AGE`] instead of reporting them fresh.
#[inline]
fn newer(a: Tick, b: Tick) -> bool {
    let d = a.wrapping_sub(b);
    d != 0 && d < Tick::MAX / 2
}

/// One archetype table: dense SoA columns for one component mask.
#[derive(Debug, Default)]
struct Table {
    entities: Vec<Entity>,
    pos: Vec<Position>,
    pos_changed: Vec<Tick>,
    vel: Vec<Velocity>,
    vel_changed: Vec<Tick>,
}

/// DOTS-layout contract (buyer-expectation pin, ideas only): one dense SoA
/// column set per component mask, same-mask rows packed contiguous, queries
/// visit exactly the matching rows and never cross tables. Store semantics
/// unchanged; this only asserts the landed shape in debug builds.
/// Determinism: table order, no allocation.
fn assert_table_for_mask(mask: u8, table: &Table) {
    let n = table.entities.len();
    if has_bit(mask, P) {
        debug_assert_eq!(table.pos.len(), n);
        debug_assert_eq!(table.pos_changed.len(), n);
    } else {
        debug_assert!(table.pos.is_empty() && table.pos_changed.is_empty());
    }
    if has_bit(mask, V) {
        debug_assert_eq!(table.vel.len(), n);
        debug_assert_eq!(table.vel_changed.len(), n);
    } else {
        debug_assert!(table.vel.is_empty() && table.vel_changed.is_empty());
    }
}

/// Per-id slot: table location plus generation for stale-handle rejection.
#[derive(Clone, Copy, Debug)]
struct Loc {
    mask: u8,
    row: u32,
    gen: u32,
    alive: bool,
}

impl Loc {
    /// Row index inside the slot's table.
    fn row_index(self) -> usize {
        self.row as usize
    }
}

/// Swap-remove one row; remaps the moved entity's row.
fn swap_remove_row(tables: &mut [Table], locs: &mut [Loc], mask: u8, row: usize) {
    let t = &mut tables[usize::from(mask)];
    let last = t.entities.len() - 1;
    t.entities.swap_remove(row);
    if has_bit(mask, P) {
        t.pos.swap_remove(row);
        t.pos_changed.swap_remove(row);
    }
    if has_bit(mask, V) {
        t.vel.swap_remove(row);
        t.vel_changed.swap_remove(row);
    }
    if row != last {
        locs[t.entities[row].slot()].row = row as u32;
    }
}

/// Archetype world: typed registry, SoA tables, generational entities, staged kill.
#[derive(Debug)]
pub struct World {
    tick: Tick,
    last_scan: Tick,
    locs: Vec<Loc>,
    free: Vec<u32>,
    tables: Vec<Table>,
    pending: Vec<Entity>,
    /// Sparse markers (visibility-like churn): no table moves, never hashed.
    markers: SparseSet,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Empty world with the four P/V tables pre-allocated.
    pub fn new() -> Self {
        Self {
            tick: 0,
            last_scan: 0,
            locs: vec![],
            free: vec![],
            tables: vec![
                Table::default(),
                Table::default(),
                Table::default(),
                Table::default(),
            ],
            pending: vec![],
            markers: SparseSet::new(),
        }
    }
    /// Current change tick.
    pub fn tick(&self) -> Tick {
        self.tick
    }
    /// Tick of the last periodic stamp scan (see [`World::check_change_ticks`]).
    pub fn last_scan(&self) -> Tick {
        self.last_scan
    }
    /// Advance the change clock one tick.
    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }
    /// Clamp every change stamp older than [`MAX_CHANGE_AGE`] to
    /// `tick - MAX_CHANGE_AGE` (Bevy `check_change_ticks` shape reimplemented
    /// std-only): table order, in place, no allocation. Returns the clamped
    /// stamp count. Units: ticks. Determinism: clamping preserves every
    /// [`is_newer_than`] decision, so [`World::state_hash`] and query results
    /// are identical before and after; scan order never affects hashed state.
    /// Budget: O(rows) time, zero heap allocation.
    pub fn check_change_ticks(&mut self) -> usize {
        let now = self.tick;
        let mut clamped = 0usize;
        for table in &mut self.tables {
            for stamp in table
                .pos_changed
                .iter_mut()
                .chain(table.vel_changed.iter_mut())
            {
                if now.wrapping_sub(*stamp) > MAX_CHANGE_AGE {
                    *stamp = now.wrapping_sub(MAX_CHANGE_AGE);
                    clamped += 1;
                }
            }
        }
        clamped += self.markers.check_change_ticks(now, MAX_CHANGE_AGE);
        self.last_scan = now;
        clamped
    }
    /// Threshold-gated [`World::check_change_ticks`]: scans only once the
    /// world advanced at least [`CHECK_TICK_THRESHOLD`] past the last scan,
    /// otherwise returns 0 having touched nothing. The arena tick calls this
    /// every tick; the full scan itself runs about hourly at 60 Hz, so the
    /// steady-state cost is one wrapping subtraction plus a branch.
    /// Determinism: same ticks give the same scan decision on every machine.
    /// Budget: O(1) when not due, O(rows) with zero allocation when due.
    pub fn check_change_ticks_if_due(&mut self) -> usize {
        if self.tick.wrapping_sub(self.last_scan) >= CHECK_TICK_THRESHOLD {
            self.check_change_ticks()
        } else {
            0
        }
    }
    /// Live entity count (staged kills count until [`World::flush`]).
    pub fn count(&self) -> usize {
        self.tables.iter().map(|t| t.entities.len()).sum()
    }
    /// Staged (not yet flushed) kill count.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    /// Pre-size every id slot, free slot, staged-kill slot and table column
    /// for `entities` more live entities. Upper bound on purpose: each of
    /// the four tables reserves the full count, so single-table workloads
    /// over-reserve. Setup path only; call before the tick so the tick path
    /// never grows. Budget: O(entities) time, one reserve per vec.
    pub fn reserve(&mut self, entities: usize) {
        self.locs.reserve(entities);
        self.free.reserve(entities);
        self.pending.reserve(entities);
        self.markers.reserve(entities);
        for table in &mut self.tables {
            table.entities.reserve(entities);
            table.pos.reserve(entities);
            table.pos_changed.reserve(entities);
            table.vel.reserve(entities);
            table.vel_changed.reserve(entities);
        }
    }
    /// Archetype mask for a spawn pair.
    fn mask_of(p: &Option<Position>, v: &Option<Velocity>) -> u8 {
        (if p.is_some() { P } else { 0 }) | (if v.is_some() { V } else { 0 })
    }
    /// Spawn into the matching table; recycles ids with their live `gen`.
    pub fn spawn(&mut self, p: Option<Position>, v: Option<Velocity>) -> Entity {
        let mask = Self::mask_of(&p, &v);
        let (id, gen) = match self.free.pop() {
            Some(id) => (id, self.locs[id as usize].gen),
            None => {
                let id = self.locs.len() as u32;
                self.locs.push(Loc {
                    mask: 0,
                    row: 0,
                    gen: 0,
                    alive: false,
                });
                (id, 0)
            }
        };
        let row = self.push_row(mask, Entity { id, gen }, p, v, self.tick, self.tick);
        self.locs[id as usize] = Loc {
            mask,
            row,
            gen,
            alive: true,
        };
        Entity { id, gen }
    }
    /// Push one row into table `mask`; returns the row index.
    fn push_row(
        &mut self,
        mask: u8,
        e: Entity,
        p: Option<Position>,
        v: Option<Velocity>,
        pc: Tick,
        vc: Tick,
    ) -> u32 {
        let t = &mut self.tables[usize::from(mask)];
        let row = t.entities.len() as u32;
        t.entities.push(e);
        if has_bit(mask, P) {
            t.pos.push(p.unwrap_or(Position { x: 0.0, y: 0.0 }));
            t.pos_changed.push(pc);
        }
        if has_bit(mask, V) {
            t.vel.push(v.unwrap_or(Velocity { x: 0.0, y: 0.0 }));
            t.vel_changed.push(vc);
        }
        row
    }
    /// Read back the stored components of one row.
    fn read_row(&self, mask: u8, row: usize) -> (Option<Position>, Tick, Option<Velocity>, Tick) {
        let t = &self.tables[usize::from(mask)];
        let cp = if has_bit(mask, P) {
            Some(t.pos[row])
        } else {
            None
        };
        let cpc = if has_bit(mask, P) {
            t.pos_changed[row]
        } else {
            self.tick
        };
        let cv = if has_bit(mask, V) {
            Some(t.vel[row])
        } else {
            None
        };
        let cvc = if has_bit(mask, V) {
            t.vel_changed[row]
        } else {
            self.tick
        };
        (cp, cpc, cv, cvc)
    }
    /// Immediate despawn; stale generations fail closed.
    pub fn despawn(&mut self, e: Entity) -> bool {
        if !self.ok(e) {
            return false;
        }
        let id = e.slot();
        let (mask, row) = (self.locs[id].mask, self.locs[id].row_index());
        swap_remove_row(&mut self.tables, &mut self.locs, mask, row);
        self.markers.remove_id(e.id);
        self.locs[id].alive = false;
        self.locs[id].gen = self.locs[id].gen.wrapping_add(1);
        self.free.push(e.id);
        true
    }
    /// Stage a kill. Applies on [`World::flush`].
    pub fn kill(&mut self, e: Entity) -> bool {
        if !self.ok(e) || self.pending.contains(&e) {
            return false;
        }
        self.pending.push(e);
        true
    }
    /// Apply all staged kills; stale handles are skipped, never fatal.
    /// Drains in canonical `(id, gen)` order so the staged call order never
    /// reaches `free` reuse or table rows: index-ordered Vec, no HashMap order.
    pub fn flush(&mut self) {
        let mut q = std::mem::take(&mut self.pending);
        q.sort_by_key(|e| (e.id, e.gen));
        for e in q {
            let _ = self.despawn(e);
        }
    }
    /// Apply all staged kills through `arena` scratch with no allocation:
    /// same canonical `(id, gen)` order and same despawn sequence as
    /// [`World::flush`], so checkpoints are identical. The worklist and the
    /// free list must already hold the kill set; otherwise returns
    /// [`ArenaError::Overflow`] with nothing staged, applied or dropped.
    /// Budget: O(staged log staged) time, zero heap allocation.
    pub fn flush_with(&mut self, arena: &mut PhaseArena) -> Result<(), ArenaError> {
        let staged = self.pending.len();
        if arena.entity_capacity() < staged {
            return Err(ArenaError::Overflow {
                needed: staged,
                capacity: arena.entity_capacity(),
            });
        }
        let mut live = 0usize;
        for e in &self.pending {
            if self.ok(*e) {
                live += 1;
            }
        }
        let free_spare = self.free.capacity() - self.free.len();
        if free_spare < live {
            return Err(ArenaError::Overflow {
                needed: self.free.len() + live,
                capacity: self.free.capacity(),
            });
        }
        arena.clear_entities();
        arena.try_extend_entities(&self.pending)?;
        self.pending.clear();
        arena.sort_entities_by_id();
        for e in arena.drain_entities() {
            let _ = self.despawn(e);
        }
        Ok(())
    }
    /// Move one entity to table `dst`, carrying stored components.
    fn move_to(&mut self, e: Entity, dst: u8, p: Option<Position>, v: Option<Velocity>) {
        let id = e.slot();
        let (src, row) = (self.locs[id].mask, self.locs[id].row_index());
        let (kept_p, cpc, kept_v, cvc) = self.read_row(src, row);
        let cp = if has_bit(src, P) { kept_p } else { p };
        let cv = if has_bit(src, V) { kept_v } else { v };
        swap_remove_row(&mut self.tables, &mut self.locs, src, row);
        let nr = self.push_row(dst, e, cp, cv, cpc, cvc);
        self.locs[id].mask = dst;
        self.locs[id].row = nr;
    }
    /// True when the handle is live and its generation matches.
    fn ok(&self, e: Entity) -> bool {
        e.slot() < self.locs.len() && self.locs[e.slot()].alive && self.locs[e.slot()].gen == e.gen
    }
    /// True when the entity is live and carries [`Position`].
    pub fn has_pos(&self, e: Entity) -> bool {
        self.ok(e) && has_bit(self.locs[e.slot()].mask, P)
    }
    /// True when the entity is live and carries [`Velocity`].
    pub fn has_vel(&self, e: Entity) -> bool {
        self.ok(e) && has_bit(self.locs[e.slot()].mask, V)
    }
    /// Copy of the entity's [`Position`], if live and present.
    pub fn get_pos(&self, e: Entity) -> Option<Position> {
        if !self.has_pos(e) {
            return None;
        }
        let l = self.locs[e.slot()];
        Some(self.tables[usize::from(l.mask)].pos[l.row_index()])
    }
    /// Copy of the entity's [`Velocity`], if live and present.
    pub fn get_vel(&self, e: Entity) -> Option<Velocity> {
        if !self.has_vel(e) {
            return None;
        }
        let l = self.locs[e.slot()];
        Some(self.tables[usize::from(l.mask)].vel[l.row_index()])
    }
    /// Overwrite [`Position`] and stamp the change tick.
    pub fn set_pos(&mut self, e: Entity, p: Position) -> bool {
        if !self.has_pos(e) {
            return false;
        }
        let l = self.locs[e.slot()];
        let t = &mut self.tables[usize::from(l.mask)];
        t.pos[l.row_index()] = p;
        t.pos_changed[l.row_index()] = self.tick;
        true
    }
    /// Overwrite [`Velocity`] and stamp the change tick.
    pub fn set_vel(&mut self, e: Entity, v: Velocity) -> bool {
        if !self.has_vel(e) {
            return false;
        }
        let l = self.locs[e.slot()];
        let t = &mut self.tables[usize::from(l.mask)];
        t.vel[l.row_index()] = v;
        t.vel_changed[l.row_index()] = self.tick;
        true
    }
    /// Add [`Velocity`], moving the entity across tables.
    pub fn add_vel(&mut self, e: Entity, v: Velocity) -> bool {
        if !self.ok(e) {
            return false;
        }
        if self.has_vel(e) {
            return self.set_vel(e, v);
        }
        let dst = self.locs[e.slot()].mask | V;
        self.move_to(e, dst, None, Some(v));
        true
    }
    /// Remove [`Position`], moving the entity across tables.
    pub fn remove_pos(&mut self, e: Entity) -> bool {
        if !self.has_pos(e) {
            return false;
        }
        let dst = self.locs[e.slot()].mask & !P;
        self.move_to(e, dst, None, None);
        true
    }
    /// Remove [`Velocity`], moving the entity across tables.
    pub fn remove_vel(&mut self, e: Entity) -> bool {
        if !self.has_vel(e) {
            return false;
        }
        let dst = self.locs[e.slot()].mask & !V;
        self.move_to(e, dst, None, None);
        true
    }
    /// Storage discipline for table kinds (P/V): always [`StorageKind::Table`].
    pub fn table_kind() -> StorageKind {
        StorageKind::Table
    }
    /// Live sparse-marker count.
    pub fn marker_count(&self) -> usize {
        self.markers.len()
    }
    /// Add the sparse [`Marker`] without moving tables. Units: ticks at
    /// `tick`. Determinism: dense order, never hashed.
    pub fn add_marker(&mut self, e: Entity) -> bool {
        if !self.ok(e) {
            return false;
        }
        let tick = self.tick;
        self.markers.insert(e, tick)
    }
    /// Remove the sparse [`Marker`]; no table move. False when absent/stale.
    pub fn remove_marker(&mut self, e: Entity) -> bool {
        if !self.ok(e) {
            return false;
        }
        self.markers.remove(e)
    }
    /// True for a live entity carrying the sparse [`Marker`].
    pub fn has_marker(&self, e: Entity) -> bool {
        self.ok(e) && self.markers.contains(e)
    }
    /// True when the marker was added after `last_run` per [`is_newer_than`].
    pub fn added_marker_since(&self, e: Entity, last_run: Tick, this_run: Tick) -> bool {
        self.ok(e) && self.markers.added_since(e, last_run, this_run)
    }
    /// True when the marker changed after `last_run` per [`is_newer_than`].
    pub fn changed_marker_since(&self, e: Entity, last_run: Tick, this_run: Tick) -> bool {
        self.ok(e) && self.markers.changed_since(e, last_run, this_run)
    }
    /// Visit added markers in dense order (the [`Added`] filter shape).
    pub fn for_each_marker_added_since(
        &self,
        last_run: Tick,
        this_run: Tick,
        f: impl FnMut(Entity),
    ) {
        self.markers.for_each_added_since(last_run, this_run, f);
    }
    /// Visit changed markers in dense order (the [`Changed`] filter shape).
    pub fn for_each_marker_changed_since(
        &self,
        last_run: Tick,
        this_run: Tick,
        f: impl FnMut(Entity),
    ) {
        self.markers.for_each_changed_since(last_run, this_run, f);
    }
    /// True when [`Position`] changed after `since`.
    pub fn changed_pos(&self, e: Entity, since: Tick) -> bool {
        if !self.has_pos(e) {
            return false;
        }
        let l = self.locs[e.slot()];
        newer(
            self.tables[usize::from(l.mask)].pos_changed[l.row_index()],
            since,
        )
    }
    /// True when [`Position`] changed after `last_run`, read against
    /// `this_run` (three-way [`is_newer_than`]: stamps older than
    /// [`MAX_CHANGE_AGE`] report stale, never fresh). Stale handles report
    /// false. Units: ticks. Determinism: exact integer math only.
    pub fn changed_pos_since(&self, e: Entity, last_run: Tick, this_run: Tick) -> bool {
        if !self.has_pos(e) {
            return false;
        }
        let Ok(slot) = usize::try_from(e.id) else {
            return false;
        };
        let l = self.locs[slot];
        let Ok(row) = usize::try_from(l.row) else {
            return false;
        };
        is_newer_than(
            self.tables[usize::from(l.mask)].pos_changed[row],
            last_run,
            this_run,
        )
    }
    /// Visit every entity carrying [`Position`].
    pub fn for_each_p(&self, mut f: impl FnMut(Entity, Position)) {
        debug_assert_eq!(self.tables.len(), 4);
        assert_table_for_mask(1, &self.tables[1]);
        assert_table_for_mask(3, &self.tables[3]);
        for m in [1u8, 3] {
            let t = &self.tables[usize::from(m)];
            for i in 0..t.entities.len() {
                f(t.entities[i], t.pos[i]);
            }
        }
    }
    /// Visit every entity carrying [`Position`] plus [`Velocity`].
    pub fn for_each_pv(&self, mut f: impl FnMut(Entity, Position, Velocity)) {
        debug_assert_eq!(self.tables.len(), 4);
        assert_table_for_mask(3, &self.tables[3]);
        let t = &self.tables[3];
        for i in 0..t.entities.len() {
            f(t.entities[i], t.pos[i], t.vel[i]);
        }
    }
    /// Visit P+V rows changed after `since`.
    pub fn for_each_pv_changed(&self, since: Tick, mut f: impl FnMut(Entity, Position, Velocity)) {
        let t = &self.tables[3];
        for i in 0..t.entities.len() {
            if newer(t.pos_changed[i], since) || newer(t.vel_changed[i], since) {
                f(t.entities[i], t.pos[i], t.vel[i]);
            }
        }
    }
    /// Visit P+V rows changed after `last_run`, read against `this_run`
    /// (three-way [`is_newer_than`]: rows older than [`MAX_CHANGE_AGE`] are
    /// skipped, never reported fresh). Units: ticks. Determinism: table
    /// order, exact integer math only, no allocation.
    pub fn for_each_pv_changed_since(
        &self,
        last_run: Tick,
        this_run: Tick,
        mut f: impl FnMut(Entity, Position, Velocity),
    ) {
        let t = &self.tables[3];
        for i in 0..t.entities.len() {
            if is_newer_than(t.pos_changed[i], last_run, this_run)
                || is_newer_than(t.vel_changed[i], last_run, this_run)
            {
                f(t.entities[i], t.pos[i], t.vel[i]);
            }
        }
    }
    /// Visit every P+V row mutably; stamps both columns with the tick.
    pub fn for_each_pv_mut(&mut self, mut f: impl FnMut(Entity, &mut Position, &mut Velocity)) {
        let tick = self.tick;
        let t = &mut self.tables[3];
        for i in 0..t.entities.len() {
            f(t.entities[i], &mut t.pos[i], &mut t.vel[i]);
            t.pos_changed[i] = tick;
            t.vel_changed[i] = tick;
        }
    }
    /// Integrate system: `pos += vel * dt` over the P+V table, one 4-wide
    /// head plus a scalar tail in index order. Finite `dt` takes the fast
    /// path: static 4-blocks and static tail rows skip the float mul, and
    /// every stamp lands in one bulk fill so the hot loop carries no stamp
    /// stores. Non-finite `dt` takes the slow path with no skip
    /// (`0 * NaN` is NaN). Same order gives the same bits and the same
    /// state hash. Budget: O(rows) time, zero allocation.
    pub fn integrate(&mut self, dt: f32) {
        let tick = self.tick;
        let t = &mut self.tables[3];
        let n = t.entities.len();
        if n == 0 {
            return;
        }
        if !dt.is_finite() {
            for i in 0..n {
                t.pos[i].x += t.vel[i].x * dt;
                t.pos[i].y += t.vel[i].y * dt;
                t.pos_changed[i] = tick;
            }
            return;
        }
        let head = n & !3;
        let mut i = 0;
        while i < head {
            let vx0 = t.vel[i].x;
            let vy0 = t.vel[i].y;
            let vx1 = t.vel[i + 1].x;
            let vy1 = t.vel[i + 1].y;
            let vx2 = t.vel[i + 2].x;
            let vy2 = t.vel[i + 2].y;
            let vx3 = t.vel[i + 3].x;
            let vy3 = t.vel[i + 3].y;
            let s0 = vx0 == 0.0 && vy0 == 0.0;
            let s1 = vx1 == 0.0 && vy1 == 0.0;
            let s2 = vx2 == 0.0 && vy2 == 0.0;
            let s3 = vx3 == 0.0 && vy3 == 0.0;
            if s0 && s1 && s2 && s3 {
                i += 4;
                continue;
            }
            if !s0 {
                t.pos[i].x += vx0 * dt;
                t.pos[i].y += vy0 * dt;
            }
            if !s1 {
                t.pos[i + 1].x += vx1 * dt;
                t.pos[i + 1].y += vy1 * dt;
            }
            if !s2 {
                t.pos[i + 2].x += vx2 * dt;
                t.pos[i + 2].y += vy2 * dt;
            }
            if !s3 {
                t.pos[i + 3].x += vx3 * dt;
                t.pos[i + 3].y += vy3 * dt;
            }
            i += 4;
        }
        while i < n {
            let vx = t.vel[i].x;
            let vy = t.vel[i].y;
            if vx != 0.0 || vy != 0.0 {
                t.pos[i].x += vx * dt;
                t.pos[i].y += vy * dt;
            }
            i += 1;
        }
        t.pos_changed[..n].fill(tick);
    }
    /// Gather every live entity into `arena` scratch in index order: the
    /// phase temporary query systems iterate without allocating. Clears the
    /// entity scratch first; refuses with [`ArenaError::Overflow`] before
    /// staging anything when the pre-size is short. Budget: O(slots) time,
    /// zero heap allocation.
    pub fn collect_live_into(&self, arena: &mut PhaseArena) -> Result<(), ArenaError> {
        let mut live = 0usize;
        for loc in &self.locs {
            if loc.alive {
                live += 1;
            }
        }
        if arena.entity_capacity() < live {
            return Err(ArenaError::Overflow {
                needed: live,
                capacity: arena.entity_capacity(),
            });
        }
        arena.clear_entities();
        // Wraps exactly like `spawn`'s id assignment, so the worklist id
        // always names the slot this loop visited.
        let mut id: u32 = 0;
        for loc in &self.locs {
            if loc.alive {
                arena.try_push_entity(Entity { id, gen: loc.gen })?;
            }
            id = id.wrapping_add(1);
        }
        Ok(())
    }
    /// Byte length [`World::checkpoint_with`] stages for this world.
    fn checkpoint_len(&self) -> usize {
        let mut len = 0usize;
        for loc in &self.locs {
            if !loc.alive {
                continue;
            }
            len += 4 + 1;
            if has_bit(loc.mask, P) {
                len += 8;
            }
            if has_bit(loc.mask, V) {
                len += 8;
            }
        }
        len
    }
    /// Checkpoint digest through `arena` scratch: the [`World::state_hash`]
    /// field sequence (slot id, mask, position bits, velocity bits) staged
    /// into the reusable byte buffer and hashed with the snapshot owner's
    /// FNV-1a64, so the digest equals [`World::state_hash`] bit for bit.
    /// Refuses with [`ArenaError::Overflow`] before staging anything when
    /// the byte pre-size is short. Budget: O(slots) time, zero allocation.
    pub fn checkpoint_with(&self, arena: &mut PhaseArena) -> Result<u64, ArenaError> {
        let needed = self.checkpoint_len();
        if arena.byte_capacity() < needed {
            return Err(ArenaError::Overflow {
                needed,
                capacity: arena.byte_capacity(),
            });
        }
        arena.clear_bytes();
        // Same wrap as `spawn`: the staged id always names this slot.
        let mut id: u32 = 0;
        for loc in &self.locs {
            if loc.alive {
                let table = &self.tables[usize::from(loc.mask)];
                // Rows always fit the address space on every supported
                // target; the error keeps the tick fallible, never panicking.
                let row = usize::try_from(loc.row).map_err(|_| ArenaError::Overflow {
                    needed: usize::MAX,
                    capacity: usize::MAX,
                })?;
                arena.try_extend_bytes(&id.to_le_bytes())?;
                arena.try_extend_bytes(&[loc.mask])?;
                if has_bit(loc.mask, P) {
                    arena.try_extend_bytes(&table.pos[row].x.to_bits().to_le_bytes())?;
                    arena.try_extend_bytes(&table.pos[row].y.to_bits().to_le_bytes())?;
                }
                if has_bit(loc.mask, V) {
                    arena.try_extend_bytes(&table.vel[row].x.to_bits().to_le_bytes())?;
                    arena.try_extend_bytes(&table.vel[row].y.to_bits().to_le_bytes())?;
                }
            }
            id = id.wrapping_add(1);
        }
        Ok(hash_bytes(arena.bytes()))
    }
    /// Deterministic state hash (quantized via raw bits, id order).
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for (id, l) in self.locs.iter().enumerate() {
            if !l.alive {
                continue;
            }
            h.write_u32(id as u32);
            h.write_u8(l.mask);
            let t = &self.tables[usize::from(l.mask)];
            let r = l.row_index();
            if has_bit(l.mask, P) {
                h.write_f32(t.pos[r].x);
                h.write_f32(t.pos[r].y);
            }
            if has_bit(l.mask, V) {
                h.write_f32(t.vel[r].x);
                h.write_f32(t.vel[r].y);
            }
        }
        h.finish()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pv(x: f32) -> (Option<Position>, Option<Velocity>) {
        (
            Some(Position { x, y: 0.0 }),
            Some(Velocity { x: 1.0, y: 0.0 }),
        )
    }
    #[test]
    fn registry_maps_known_types() {
        let r = Registry::new();
        assert_eq!(r.bit::<Position>(), Some(ComponentId::POS));
        assert_eq!(r.bit::<Velocity>(), Some(ComponentId::VEL));
        assert_eq!(r.bit::<u32>(), None);
        assert_eq!(ComponentId::POS.bit(), 1);
    }
    #[test]
    fn spawn_kill_count_deferred() {
        let mut w = World::new();
        let mut ids = vec![];
        for i in 0..3 {
            let (p, v) = pv(i as f32);
            ids.push(w.spawn(p, v));
        }
        assert_eq!(w.count(), 3);
        assert!(w.kill(ids[0]));
        assert!(!w.kill(ids[0]));
        assert_eq!((w.count(), w.pending()), (3, 1));
        w.flush();
        assert_eq!((w.count(), w.pending()), (2, 0));
        assert!(!w.despawn(ids[0]));
        assert_eq!(w.get_pos(ids[1]), Some(Position { x: 1.0, y: 0.0 }));
    }
    #[test]
    fn generation_rejects_stale_handles() {
        let mut w = World::new();
        let (p, v) = pv(0.0);
        let a = w.spawn(p, v);
        assert!(w.despawn(a));
        assert!(!w.despawn(a));
        assert!(!w.set_pos(a, Position { x: 9.0, y: 9.0 }));
        let (p2, v2) = pv(7.0);
        let b = w.spawn(p2, v2);
        assert_eq!(b.id, a.id);
        assert_ne!(b.gen, a.gen);
        assert_eq!(w.get_pos(b), Some(Position { x: 7.0, y: 0.0 }));
    }
    #[test]
    fn spawn_query_counts() {
        let mut w = World::new();
        let (p, v) = pv(0.0);
        w.spawn(p, v);
        w.spawn(Some(Position { x: 1.0, y: 1.0 }), None);
        assert_eq!(w.count(), 2);
        let mut n = 0;
        w.for_each_pv(|_, _, _| n += 1);
        assert_eq!(n, 1);
        let mut m = 0;
        w.for_each_p(|_, _| m += 1);
        assert_eq!(m, 2);
    }
    #[test]
    fn swap_remove_remaps() {
        let mut w = World::new();
        let mut ids = vec![];
        for i in 0..4 {
            let (p, v) = pv(i as f32);
            ids.push(w.spawn(p, v));
        }
        assert!(w.despawn(ids[0]));
        assert_eq!(w.count(), 3);
        assert_eq!(w.get_pos(ids[3]), Some(Position { x: 3.0, y: 0.0 }));
        assert!(!w.despawn(ids[0]));
        let mut xs = vec![];
        w.for_each_pv(|_, p, _| xs.push(p.x.to_bits()));
        xs.sort_unstable();
        assert_eq!(xs, vec![1f32.to_bits(), 2f32.to_bits(), 3f32.to_bits()]);
    }
    #[test]
    fn add_remove_moves_tables() {
        let mut w = World::new();
        let e = w.spawn(Some(Position { x: 5.0, y: 5.0 }), None);
        assert!(w.add_vel(e, Velocity { x: 2.0, y: 0.0 }));
        assert!(w.has_pos(e) && w.has_vel(e));
        assert_eq!(w.get_pos(e), Some(Position { x: 5.0, y: 5.0 }));
        assert!(w.remove_pos(e));
        assert!(!w.has_pos(e) && w.has_vel(e));
        assert!(w.remove_vel(e));
        assert_eq!(w.count(), 1);
        assert!(w.despawn(e));
        assert_eq!(w.count(), 0);
    }
    #[test]
    fn changed_filter() {
        let mut w = World::new();
        let (p, v) = pv(0.0);
        let a = w.spawn(p, v);
        let (p2, v2) = pv(9.0);
        let b = w.spawn(p2, v2);
        let t0 = w.tick();
        w.advance();
        assert!(w.set_pos(a, Position { x: 1.0, y: 0.0 }));
        assert!(!w.changed_pos(b, t0) || w.changed_pos(a, t0));
        let mut seen = vec![];
        w.for_each_pv_changed(t0, |e, _, _| seen.push(e));
        assert_eq!(seen, vec![a]);
        assert!(w.changed_pos(a, t0) && !w.changed_pos(b, t0));
        let _ = b;
    }
    #[test]
    fn hash_deterministic_layout_independent() {
        let (mut a, mut b) = (World::new(), World::new());
        let (p, v) = pv(3.0);
        a.spawn(p, v);
        let (p, v) = pv(3.0);
        let eb = b.spawn(p, v);
        assert_eq!(a.state_hash(), b.state_hash());
        b.set_pos(eb, Position { x: 5.0, y: 0.0 });
        assert_ne!(a.state_hash(), b.state_hash());
        let mut c = World::new();
        let (p, v) = pv(3.0);
        let e0 = c.spawn(p, v);
        let (p2, v2) = pv(7.0);
        let e1 = c.spawn(p2, v2);
        c.despawn(e0);
        c.despawn(e1);
        let (p3, v3) = pv(3.0);
        c.spawn(p3, v3);
        assert_eq!(c.count(), 1);
    }
    #[test]
    fn seeded_kill_order_moves_zero_hashes() {
        // Staged-kill call order never reaches hashed state. The same
        // kill set staged forward, reversed and xorshift-shuffled must flush
        // to the same hash, and the next recycled id must match as well.
        fn xorshift(mut x: u64) -> u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
        fn build(order: &[usize]) -> (u64, Entity) {
            let mut w = World::new();
            let mut ids = vec![];
            for i in 0..8 {
                let (p, v) = pv(i as f32);
                ids.push(w.spawn(p, v));
            }
            // Kill the even slots, staged in the caller's `order`.
            let victims: Vec<Entity> = (0..8).step_by(2).map(|i| ids[i]).collect();
            let mut staged = vec![];
            for o in order {
                staged.push(victims[*o % victims.len()]);
            }
            // Deduplicate the staged list the way distinct orders may repeat.
            let mut seen = vec![];
            for e in staged {
                if !seen.contains(&e) {
                    assert!(w.kill(e));
                    seen.push(e);
                }
            }
            w.flush();
            let (p, v) = pv(99.0);
            let reused = w.spawn(p, v);
            (w.state_hash(), reused)
        }
        let forward = [0, 1, 2, 3];
        let mut backward = forward;
        backward.reverse();
        // Seeded shuffle of [0,1,2,3] with a fixed key.
        let mut shuffled = forward;
        let mut rng = 0x9E37_79B9_7F4A_7C15u64 | 1;
        for i in (1..shuffled.len()).rev() {
            rng = xorshift(rng);
            shuffled.swap(i, (rng as usize) % (i + 1));
        }
        assert_ne!(shuffled, forward, "shuffle must perturb the test");
        let (hf, rf) = build(&forward);
        let (hb, rb) = build(&backward);
        let (hs, rs) = build(&shuffled);
        assert_eq!(hf, hb, "kill order moved the hash");
        assert_eq!(hf, hs, "seeded kill order moved the hash");
        assert_eq!(rf, rb, "kill order moved id reuse");
        assert_eq!(rf, rs, "seeded kill order moved id reuse");
        // A real change still moves the hash, so the test is not vacuous.
        let mut w = World::new();
        for i in 0..8 {
            let (p, v) = pv(i as f32);
            w.spawn(p, v);
        }
        assert_ne!(hf, w.state_hash());
    }
    #[test]
    #[ignore = "timing: run `node tools/proof.mjs --perf` (release, one thread)"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn perf_1k_p99_under_2ms() {
        let mut w = World::new();
        for i in 0..1000 {
            let (p, v) = pv(i as f32);
            w.spawn(p, v);
        }
        let mut ds = Vec::with_capacity(101);
        for _ in 0..101 {
            let t = std::time::Instant::now();
            w.integrate(0.016);
            ds.push(t.elapsed());
        }
        ds.sort();
        assert!(ds[99].as_micros() < 2000, "p99 {:?}", ds[99]);
    }
    #[test]
    fn integrate_zero_velocity_keeps_pos_and_stamps() {
        use engine_core::alloc_probe::measure;
        let mut w = World::new();
        let mut ids = Vec::with_capacity(8);
        let mut x = 0.0f32;
        for k in 0..8 {
            let vel = if k % 2 == 1 {
                Velocity { x: 1.0, y: 0.5 }
            } else if k == 0 {
                Velocity { x: -0.0, y: -0.0 }
            } else {
                Velocity { x: 0.0, y: 0.0 }
            };
            ids.push(w.spawn(Some(Position { x, y: -x }), Some(vel)));
            x += 1.0;
        }
        let before: Vec<Position> = ids
            .iter()
            .map(|e| w.get_pos(*e).expect("spawned row is live"))
            .collect();
        let hash_before = w.state_hash();
        w.advance();
        let tick = w.tick();
        let ((), heap) = measure(|| w.integrate(0.016));
        assert_eq!(heap.calls, 0, "integrate must not allocate");
        for (k, e) in ids.iter().enumerate() {
            let got = w.get_pos(*e).expect("spawned row stays live");
            if k % 2 == 1 {
                let exp_x = before[k].x + 1.0 * 0.016;
                let exp_y = before[k].y + 0.5 * 0.016;
                assert_eq!(got.x.to_bits(), exp_x.to_bits(), "dynamic row moved");
                assert_eq!(got.y.to_bits(), exp_y.to_bits(), "dynamic row moved");
            } else {
                assert_eq!(got.x.to_bits(), before[k].x.to_bits(), "static row kept x");
                assert_eq!(got.y.to_bits(), before[k].y.to_bits(), "static row kept y");
            }
            assert_eq!(stamp_of(&w, *e), tick, "every row stamps the tick");
        }
        assert_ne!(w.state_hash(), hash_before, "movement moves the hash");
    }
    #[test]
    fn integrate_all_static_keeps_hash_tail_and_nonfinite() {
        let mut w = World::new();
        for _ in 0..4 {
            w.spawn(
                Some(Position { x: 3.0, y: -2.0 }),
                Some(Velocity { x: 0.0, y: 0.0 }),
            );
        }
        let hash_before = w.state_hash();
        w.advance();
        w.integrate(0.016);
        assert_eq!(w.state_hash(), hash_before, "static block keeps hashed pos");
        let mut empty = World::new();
        empty.integrate(0.016);
        assert_eq!(empty.count(), 0, "empty integrate keeps count");
        let mut tail = World::new();
        let mut tail_ids = Vec::with_capacity(5);
        for k in 0..5 {
            let vel = if k == 4 {
                Velocity { x: 0.0, y: 0.0 }
            } else {
                Velocity { x: 1.0, y: 0.0 }
            };
            tail_ids.push(tail.spawn(Some(Position { x: 0.0, y: 0.0 }), Some(vel)));
        }
        tail.advance();
        tail.integrate(0.016);
        let last = tail.get_pos(tail_ids[4]).expect("tail row is live");
        assert_eq!(
            (last.x.to_bits(), last.y.to_bits()),
            (0.0f32.to_bits(), 0.0f32.to_bits()),
            "static tail keeps pos"
        );
        let mut nan = World::new();
        let s = nan.spawn(
            Some(Position { x: 1.0, y: 2.0 }),
            Some(Velocity { x: 0.0, y: 0.0 }),
        );
        nan.advance();
        nan.integrate(f32::NAN);
        let p = nan.get_pos(s).expect("nan row is live");
        assert!(p.x.is_nan() && p.y.is_nan());
    }
    const AGED_NOW: Tick = 3_400_000_000;
    const AGED_LAST: Tick = AGED_NOW - 200_000;
    /// Stored position-change stamp of one row (test-only probe).
    fn stamp_of(w: &World, e: Entity) -> Tick {
        let l = w.locs[e.id as usize];
        w.tables[l.mask as usize].pos_changed[l.row_index()]
    }
    /// 1000 P+V entities in id order: 500 stale (stamp 0) plus 500 fresh
    /// (position stamp `AGED_NOW - 10`).
    fn aged_world() -> (World, Vec<Entity>) {
        let mut w = World::new();
        w.reserve(1000);
        let mut all = Vec::with_capacity(1000);
        for i in 0..1000 {
            let f = i as f32;
            all.push(w.spawn(
                Some(Position { x: f, y: -f }),
                Some(Velocity { x: 0.5, y: 0.25 }),
            ));
        }
        w.tick = AGED_NOW - 10;
        for e in all.iter().skip(500) {
            assert!(w.set_pos(*e, Position { x: 1.0, y: 1.0 }));
        }
        w.tick = AGED_NOW;
        (w, all)
    }
    /// Digest over change decisions in id order (test oracle, not hashed state).
    fn decisions_digest(w: &World, all: &[Entity], last: Tick, now: Tick) -> u64 {
        let mut bytes = Vec::with_capacity(all.len() * 5);
        for e in all {
            bytes.extend_from_slice(&e.id.to_le_bytes());
            bytes.push(u8::from(w.changed_pos_since(*e, last, now)));
        }
        hash_bytes(&bytes)
    }
    #[test]
    fn aged_stamp_three_way_reports_fresh_only() {
        let (w, all) = aged_world();
        let (stale, fresh) = (&all[..500], &all[500..]);
        // Baseline two-way `newer` false-positives on every stale stamp, so
        // the fixture discriminates the old check from the new one.
        let base_fp = stale
            .iter()
            .filter(|e| newer(stamp_of(&w, **e), AGED_LAST))
            .count();
        assert_eq!(base_fp, 500, "fixture must catch the old check");
        // Candidate three-way: FP 0/500 with TP 500/500.
        let fp = stale
            .iter()
            .filter(|e| w.changed_pos_since(**e, AGED_LAST, AGED_NOW))
            .count();
        let tp = fresh
            .iter()
            .filter(|e| w.changed_pos_since(**e, AGED_LAST, AGED_NOW))
            .count();
        assert_eq!((fp, tp), (0, 500));
        // The row iterator agrees with the point queries row for row.
        let mut seen = vec![];
        w.for_each_pv_changed_since(AGED_LAST, AGED_NOW, |e, _, _| seen.push(e.id));
        seen.sort_unstable();
        let want: Vec<u32> = fresh.iter().map(|e| e.id).collect();
        assert_eq!(seen, want);
        // Same fixture twice: identical digest and identical state hash.
        let (w2, all2) = aged_world();
        assert_eq!(
            decisions_digest(&w, &all, AGED_LAST, AGED_NOW),
            decisions_digest(&w2, &all2, AGED_LAST, AGED_NOW)
        );
        assert_eq!(w.state_hash(), w2.state_hash());
    }
    #[test]
    fn scan_clamps_stamps_older_than_max() {
        let (mut w, all) = aged_world();
        let before = w.state_hash();
        // 1500 stamps sit at 0 (both columns of the 500 stale rows plus the
        // untouched velocity column of the 500 fresh rows); the 500 fresh
        // position stamps are 10 ticks old and stay.
        assert_eq!(w.check_change_ticks(), 1500);
        assert_eq!(w.last_scan(), AGED_NOW);
        let edge = AGED_NOW.wrapping_sub(MAX_CHANGE_AGE);
        for e in &all[..500] {
            assert_eq!(stamp_of(&w, *e), edge);
        }
        assert_eq!(stamp_of(&w, all[500]), AGED_NOW - 10);
        // Decisions are unchanged by the scan: still FP 0/500, TP 500/500.
        let fp = all[..500]
            .iter()
            .filter(|e| w.changed_pos_since(**e, AGED_LAST, AGED_NOW))
            .count();
        let tp = all[500..]
            .iter()
            .filter(|e| w.changed_pos_since(**e, AGED_LAST, AGED_NOW))
            .count();
        assert_eq!((fp, tp), (0, 500));
        // The scan moves no hashed state and a second scan clamps nothing.
        assert_eq!(w.state_hash(), before, "scan moved hashed state");
        assert_eq!(w.check_change_ticks(), 0);
    }
    #[test]
    fn three_way_wraparound_and_clamp() {
        // Just-written changes report, even across the `u32` wrap.
        assert!(is_newer_than(3, 0, 5));
        assert!(is_newer_than(0, Tick::MAX - 1, 1));
        assert!(is_newer_than(1, 0, 1));
        assert!(!is_newer_than(5, 5, 5));
        // Older than MAX never reports, however recently the system ran.
        let old = AGED_NOW.wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(1);
        assert!(!is_newer_than(old, AGED_NOW.wrapping_sub(10), AGED_NOW));
        // The clamp pins to the window edge and leaves fresh stamps alone.
        assert_eq!(
            clamp_stamp(old, AGED_NOW),
            AGED_NOW.wrapping_sub(MAX_CHANGE_AGE)
        );
        assert_eq!(
            clamp_stamp(AGED_NOW.wrapping_sub(10), AGED_NOW),
            AGED_NOW - 10
        );
        // The gated scan rests below the threshold.
        let mut w = World::new();
        assert_eq!((w.last_scan(), w.check_change_ticks_if_due()), (0, 0));
    }
    #[test]
    fn sparse_markers_leave_hash_and_tables_alone() {
        let mut w = World::new();
        w.reserve(16);
        let mut ids = Vec::with_capacity(8);
        for i in 0..8 {
            let f = i as f32;
            ids.push(w.spawn(
                Some(Position { x: f, y: -f }),
                Some(Velocity { x: 0.5, y: 0.25 }),
            ));
        }
        let before = w.state_hash();
        let mut pv = 0;
        w.for_each_pv(|_, _, _| pv += 1);
        for (k, e) in ids.iter().enumerate() {
            if k % 2 == 0 {
                assert!(w.add_marker(*e));
            }
        }
        assert_eq!(w.marker_count(), 4);
        let mut pv_after = 0;
        w.for_each_pv(|_, _, _| pv_after += 1);
        assert_eq!((pv, pv_after), (8, 8));
        assert_eq!(w.state_hash(), before);
        assert!(w.despawn(ids[0]));
        assert!(!w.has_marker(ids[0]));
        assert_ne!(w.state_hash(), before, "despawn moves the hash");
    }
    #[test]
    fn sparse_world_added_fires_once_changed_tracks_stamp() {
        let mut w = World::new();
        let e = w.spawn(Some(Position { x: 1.0, y: 0.0 }), None);
        let t0 = w.tick();
        w.advance();
        let t1 = w.tick();
        assert!(w.add_marker(e));
        assert!(w.added_marker_since(e, t0, t1));
        assert!(!w.added_marker_since(e, t1, t1));
        w.advance();
        let t2 = w.tick();
        assert!(!w.added_marker_since(e, t1, t2));
        assert!(!w.changed_marker_since(e, t2, t2));
        let mut seen = 0;
        w.for_each_marker_added_since(t0, t1, |_| seen += 1);
        assert_eq!(seen, 1);
        let mut changed = 0;
        w.for_each_marker_changed_since(t2, t2, |_| changed += 1);
        assert_eq!(changed, 0);
        assert!(w.remove_marker(e));
        assert!(!w.has_marker(e));
    }
    #[test]
    #[ignore = "timing: spike receipt, run with --ignored (release, one thread)"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn sparse_spike_receipt() {
        let mut w = World::new();
        w.reserve(10_000);
        let mut ids = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            let f = i as f32;
            ids.push(w.spawn(
                Some(Position { x: f, y: -f }),
                Some(Velocity { x: 0.5, y: 0.25 }),
            ));
        }
        let t = std::time::Instant::now();
        let mut n = 0;
        w.for_each_pv(|_, _, _| n += 1);
        let iter = t.elapsed();
        let denom = u128::try_from(n).unwrap_or(1);
        std::println!("table_iter_ns_entity={}", iter.as_nanos() / denom);
        let t = std::time::Instant::now();
        for e in ids.iter().step_by(10) {
            let _ = w.add_marker(*e);
        }
        let sadd = t.elapsed();
        let t = std::time::Instant::now();
        for e in ids.iter().step_by(10) {
            let _ = w.remove_marker(*e);
        }
        let srem = t.elapsed();
        let ops = u128::try_from(ids.len() / 10).unwrap_or(1);
        std::println!("sparse_add_ns_op={}", sadd.as_nanos() / ops);
        std::println!("sparse_remove_ns_op={}", srem.as_nanos() / ops);
        assert_eq!((n, w.marker_count()), (10_000, 0));
    }
}
