//! Sparse marker store: opt-in add/remove-heavy kinds stay out of tables.
//!
//! Owner: engine-ecs. Units: ticks for stamps, counts for lengths.
//! Determinism: dense order only, exact integer math, no hash order, no wall
//! clock; markers never enter [`super::World::state_hash`], so the same P/V
//! fixture hashes identically with or without markers. Errors: loud `false`
//! for stale handles or a full `u32` index, never panics.
//! PORT(bevyengine/bevy@v0.19.1-b56fc29d3016e641754765244b5ba3f9cc504671:crates/bevy_ecs/src/storage/mod.rs,crates/bevy_ecs/src/storage/sparse_set.rs,crates/bevy_ecs/src/storage/table/mod.rs,crates/bevy_ecs/src/storage/table/column.rs)
//! verified via gh api 2026-10-15 (table/ holds column.rs+mod.rs, not table.rs);
//! method only: Table default fast iteration, SparseSet fast add/remove at
//! slower query cost, sparse kinds do not fragment tables. Caller:
//! `crates/ecs/src/archetype.rs` holds one [`SparseSet`] for markers.

use core::marker::PhantomData;

use super::{clamp_stamp, is_newer_than, Entity, Tick};

/// Empty dense index: slot has no marker.
const NONE: u32 = u32::MAX;

/// Per-kind storage discipline: dense tables iterate fast, sparse sets edit fast.
/// Units: none. Determinism: pure tag, no state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StorageKind {
    /// Dense SoA table: default fast-iteration path for P/V.
    #[default]
    Table,
    /// Sparse set: opt-in for add/remove-heavy marker/effect kinds.
    SparseSet,
}

impl StorageKind {
    /// True for the sparse opt-in.
    pub fn is_sparse(self) -> bool {
        matches!(self, Self::SparseSet)
    }
}

/// Visibility-like marker: unit effect toggled nearly every frame.
/// Units: none. Determinism: presence only, never hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marker;

impl Marker {
    /// Storage discipline for this kind: always sparse.
    pub fn storage_kind() -> StorageKind {
        StorageKind::SparseSet
    }
}

/// `Added<T>` filter over an added stamp: true once after the add.
/// Units: ticks. Determinism: exact [`is_newer_than`] math only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Added<T> {
    last_run: Tick,
    this_run: Tick,
    kind: PhantomData<T>,
}

impl<T> Added<T> {
    /// Filter for adds after `last_run`, read against `this_run`.
    pub fn new(last_run: Tick, this_run: Tick) -> Self {
        Self {
            last_run,
            this_run,
            kind: PhantomData,
        }
    }
    /// True when `added` is newer than the last run.
    pub fn matches(&self, added: Tick) -> bool {
        is_newer_than(added, self.last_run, self.this_run)
    }
}

/// `Changed<T>` filter over a changed stamp: true only on a newer write.
/// Units: ticks. Determinism: exact [`is_newer_than`] math only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Changed<T> {
    last_run: Tick,
    this_run: Tick,
    kind: PhantomData<T>,
}

impl<T> Changed<T> {
    /// Filter for writes after `last_run`, read against `this_run`.
    pub fn new(last_run: Tick, this_run: Tick) -> Self {
        Self {
            last_run,
            this_run,
            kind: PhantomData,
        }
    }
    /// True when `changed` is newer than the last run.
    pub fn matches(&self, changed: Tick) -> bool {
        is_newer_than(changed, self.last_run, self.this_run)
    }
}

/// Unit-marker sparse set with per-entity added/changed ticks.
/// Units: counts plus ticks. Determinism: dense order, stable for one op
/// order; swap-remove remap keeps dense packed. Budget: O(1) insert/remove,
/// O(rows) scan, zero allocation after `reserve`.
#[derive(Clone, Debug, Default)]
pub struct SparseSet {
    dense: Vec<u32>,
    dense_gen: Vec<u32>,
    dense_added: Vec<Tick>,
    dense_changed: Vec<Tick>,
    sparse: Vec<u32>,
}

impl SparseSet {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }
    /// Live marker count.
    pub fn len(&self) -> usize {
        self.dense.len()
    }
    /// True when no marker is stored.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }
    /// Pre-size dense and sparse columns. Setup path only.
    pub fn reserve(&mut self, entities: usize) {
        self.dense.reserve(entities);
        self.dense_gen.reserve(entities);
        self.dense_added.reserve(entities);
        self.dense_changed.reserve(entities);
        self.sparse.reserve(entities);
    }
    /// Dense index for one id, `None` when absent or stale gen.
    fn index_of(&self, e: Entity) -> Option<usize> {
        let slot = usize::try_from(e.id).unwrap_or(usize::MAX);
        let idx = *self.sparse.get(slot)?;
        if idx == NONE {
            return None;
        }
        let at = usize::try_from(idx).unwrap_or(usize::MAX);
        if self.dense.get(at) != Some(&e.id) || self.dense_gen.get(at) != Some(&e.gen) {
            return None;
        }
        Some(at)
    }
    /// True for a live marker with a matching generation.
    pub fn contains(&self, e: Entity) -> bool {
        self.index_of(e).is_some()
    }
    /// Insert or touch the marker: new id/gen pushes with both stamps at
    /// `tick` and returns true; a present handle only moves `changed` and
    /// returns false. Stale reads fail closed with false.
    pub fn insert(&mut self, e: Entity, tick: Tick) -> bool {
        if let Some(at) = self.index_of(e) {
            self.dense_changed[at] = tick;
            return false;
        }
        let slot = usize::try_from(e.id).unwrap_or(usize::MAX);
        if slot == usize::MAX {
            return false;
        }
        if slot >= self.sparse.len() {
            self.sparse.resize(slot + 1, NONE);
        }
        let at_u32 = u32::try_from(self.dense.len()).unwrap_or(NONE);
        if at_u32 == NONE {
            return false;
        }
        self.dense.push(e.id);
        self.dense_gen.push(e.gen);
        self.dense_added.push(tick);
        self.dense_changed.push(tick);
        self.sparse[slot] = at_u32;
        true
    }
    /// Remove one marker; swap-removes the dense slot and remaps the moved
    /// id. Returns false for absent or stale handles.
    pub fn remove(&mut self, e: Entity) -> bool {
        let at = match self.index_of(e) {
            Some(at) => at,
            None => return false,
        };
        let last = self.dense.len() - 1;
        self.dense.swap_remove(at);
        self.dense_gen.swap_remove(at);
        self.dense_added.swap_remove(at);
        self.dense_changed.swap_remove(at);
        let slot = usize::try_from(e.id).unwrap_or(usize::MAX);
        if slot != usize::MAX && slot < self.sparse.len() {
            self.sparse[slot] = NONE;
        }
        if at != last {
            let moved = self.dense[at];
            let moved_slot = usize::try_from(moved).unwrap_or(usize::MAX);
            let moved_u32 = u32::try_from(at).unwrap_or(NONE);
            if moved_slot != usize::MAX && moved_slot < self.sparse.len() && moved_u32 != NONE {
                self.sparse[moved_slot] = moved_u32;
            }
        }
        true
    }
    /// Drop the sparse slot for one id regardless of generation (despawn path).
    pub fn remove_id(&mut self, id: u32) {
        let slot = usize::try_from(id).unwrap_or(usize::MAX);
        if slot == usize::MAX || slot >= self.sparse.len() {
            return;
        }
        let idx = self.sparse[slot];
        if idx == NONE {
            return;
        }
        let at = usize::try_from(idx).unwrap_or(usize::MAX);
        if at == usize::MAX || at >= self.dense.len() || self.dense[at] != id {
            self.sparse[slot] = NONE;
            return;
        }
        let last = self.dense.len() - 1;
        self.dense.swap_remove(at);
        self.dense_gen.swap_remove(at);
        self.dense_added.swap_remove(at);
        self.dense_changed.swap_remove(at);
        self.sparse[slot] = NONE;
        if at != last {
            let moved = self.dense[at];
            let moved_slot = usize::try_from(moved).unwrap_or(usize::MAX);
            let moved_u32 = u32::try_from(at).unwrap_or(NONE);
            if moved_slot != usize::MAX && moved_slot < self.sparse.len() && moved_u32 != NONE {
                self.sparse[moved_slot] = moved_u32;
            }
        }
    }
    /// Added stamp for one live marker, `None` for absent/stale handles.
    pub fn added_tick(&self, e: Entity) -> Option<Tick> {
        self.index_of(e).map(|at| self.dense_added[at])
    }
    /// Changed stamp for one live marker, `None` for absent/stale handles.
    pub fn changed_tick(&self, e: Entity) -> Option<Tick> {
        self.index_of(e).map(|at| self.dense_changed[at])
    }
    /// True when the marker was added after `last_run` per [`is_newer_than`].
    pub fn added_since(&self, e: Entity, last_run: Tick, this_run: Tick) -> bool {
        match self.added_tick(e) {
            Some(stamp) => Added::<Marker>::new(last_run, this_run).matches(stamp),
            None => false,
        }
    }
    /// True when the marker changed after `last_run` per [`is_newer_than`].
    pub fn changed_since(&self, e: Entity, last_run: Tick, this_run: Tick) -> bool {
        match self.changed_tick(e) {
            Some(stamp) => Changed::<Marker>::new(last_run, this_run).matches(stamp),
            None => false,
        }
    }
    /// Visit added markers in dense order.
    pub fn for_each_added_since(&self, last_run: Tick, this_run: Tick, mut f: impl FnMut(Entity)) {
        let keep = Added::<Marker>::new(last_run, this_run);
        for i in 0..self.dense.len() {
            if keep.matches(self.dense_added[i]) {
                f(Entity {
                    id: self.dense[i],
                    gen: self.dense_gen[i],
                });
            }
        }
    }
    /// Visit changed markers in dense order.
    pub fn for_each_changed_since(
        &self,
        last_run: Tick,
        this_run: Tick,
        mut f: impl FnMut(Entity),
    ) {
        let keep = Changed::<Marker>::new(last_run, this_run);
        for i in 0..self.dense.len() {
            if keep.matches(self.dense_changed[i]) {
                f(Entity {
                    id: self.dense[i],
                    gen: self.dense_gen[i],
                });
            }
        }
    }
    /// Clamp every stamp older than the scan window; returns clamped count.
    /// Units: ticks. Determinism: preserves every [`is_newer_than`] decision.
    pub fn check_change_ticks(&mut self, this_run: Tick, max_age: Tick) -> usize {
        let mut clamped = 0usize;
        for stamp in self
            .dense_added
            .iter_mut()
            .chain(self.dense_changed.iter_mut())
        {
            let next = clamp_stamp(*stamp, this_run);
            if next != *stamp {
                *stamp = next;
                clamped += 1;
            }
        }
        let _ = max_age;
        clamped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::MAX_CHANGE_AGE;

    #[test]
    fn sparse_storage_kind_is_opt_in() {
        assert_eq!(Marker::storage_kind(), StorageKind::SparseSet);
        assert!(Marker::storage_kind().is_sparse());
        assert!(!StorageKind::default().is_sparse());
    }

    #[test]
    fn sparse_add_remove_keeps_dense_packed() {
        let mut s = SparseSet::new();
        s.reserve(4);
        let a = Entity { id: 0, gen: 0 };
        let b = Entity { id: 1, gen: 0 };
        assert!(s.insert(a, 5));
        assert!(s.insert(b, 6));
        assert!(!s.insert(a, 7));
        assert!(s.contains(a) && s.contains(b));
        assert_eq!(s.len(), 2);
        assert!(s.remove(a));
        assert!(!s.contains(a));
        assert!(s.contains(b));
        assert!(!s.remove(a));
        let stale = Entity { id: 1, gen: 9 };
        assert!(!s.contains(stale));
        assert!(!s.remove(stale));
        assert_eq!(s.added_tick(b), Some(6));
    }

    #[test]
    fn sparse_added_fires_once() {
        let mut s = SparseSet::new();
        let e = Entity { id: 3, gen: 0 };
        assert!(s.insert(e, 10));
        assert!(s.added_since(e, 9, 10));
        assert!(!s.added_since(e, 10, 11));
        let f = Added::<Marker>::new(9, 10);
        assert!(f.matches(10));
        assert!(!Added::<Marker>::new(10, 11).matches(10));
    }

    #[test]
    fn sparse_changed_uses_is_newer_than() {
        let mut s = SparseSet::new();
        let e = Entity { id: 4, gen: 0 };
        assert!(s.insert(e, 20));
        assert!(!s.insert(e, 21));
        assert!(s.changed_since(e, 20, 21));
        assert!(!s.changed_since(e, 21, 22));
        let old = 20u32.wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(1);
        assert!(!Changed::<Marker>::new(21, 22).matches(old));
        assert!(Changed::<Marker>::new(20, 21).matches(21));
        let mut seen = 0;
        s.for_each_changed_since(20, 21, |_| seen += 1);
        assert_eq!(seen, 1);
        let mut added = 0;
        s.for_each_added_since(21, 22, |_| added += 1);
        assert_eq!(added, 0);
    }

    #[test]
    fn sparse_scan_clamps_without_moving_decisions() {
        let mut s = SparseSet::new();
        let e = Entity { id: 5, gen: 0 };
        assert!(s.insert(e, 0));
        let now: Tick = 3_400_000_000;
        assert!(s.check_change_ticks(now, MAX_CHANGE_AGE) > 0);
        assert!(!s.added_since(e, now.wrapping_sub(200_000), now));
        assert_eq!(s.check_change_ticks(now, MAX_CHANGE_AGE), 0);
    }

    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn sparse_bench_receipt() {
        let n = 2000usize;
        let mut s = SparseSet::new();
        s.reserve(n);
        let count = u32::try_from(n).unwrap_or(0);
        let ids: Vec<Entity> = (0..count).map(|id| Entity { id, gen: 0 }).collect();
        let t = std::time::Instant::now();
        for (k, e) in ids.iter().enumerate() {
            s.insert(*e, u32::try_from(k).unwrap_or(0));
        }
        let insert = t.elapsed();
        let t = std::time::Instant::now();
        for e in &ids {
            let _ = s.remove(*e);
        }
        let remove = t.elapsed();
        let denom = u128::try_from(n).unwrap_or(1);
        std::println!("sparse_insert_ns_op={}", insert.as_nanos() / denom);
        std::println!("sparse_remove_ns_op={}", remove.as_nanos() / denom);
        assert!(s.is_empty());
    }
}
