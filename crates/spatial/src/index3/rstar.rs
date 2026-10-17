use super::*;

// ---------------------------------------------------------------------------
// Rstar: integer-cm R*-tree contender (Guttman/Beckmann pattern, std-only)
// ---------------------------------------------------------------------------

/// Leaf capacity: shallow trees at 10k entries, cheap linear scans inside.
pub(super) const RSTAR_MAX: usize = 8;
/// Minimum fill after a split (40% of capacity, Guttman floor).
pub(super) const RSTAR_MIN: usize = 3;
/// Forced-reinsert count on first overflow per level (Beckmann ~30%).
pub(super) const RSTAR_REINSERT: usize = 2;

/// Integer-cm minimum bounding rectangle. Volumes use saturating i64 so
/// degenerate giants refuse to wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Mbr {
    pub(super) min_x: i32,
    pub(super) max_x: i32,
    pub(super) min_y: i32,
    pub(super) max_y: i32,
    pub(super) min_z: i32,
    pub(super) max_z: i32,
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
pub(super) struct Rentry {
    pub(super) bx: Box3,
    pub(super) child: u32,
}

#[derive(Debug, Clone)]
pub(super) struct Rnode {
    pub(super) mbr: Mbr,
    pub(super) entries: Vec<Rentry>,
    pub(super) leaf: bool,
    pub(super) level: usize,
}

/// Exact union over leaf entry boxes.
pub(super) fn leaf_mbr(entries: &[Rentry]) -> Option<Mbr> {
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
pub(super) fn inner_mbr(entries: &[Rentry]) -> Option<Mbr> {
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

pub(super) fn slot_of(child: usize, m: &Mbr) -> Rentry {
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
pub(super) enum InsertOut {
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
    pub(super) nodes: Vec<Rnode>,
    pub(super) root: usize,
    pub(super) boxes: BTreeMap<u32, Box3>,
    pub(super) revision: u64,
    pub(super) rebuilds: u64,
    pub(super) last_dirty: Vec<u32>,
    /// Incremental free-space field: XZ tile to covering-box count at
    /// nav pitch. Placement broadphase and reachability read it; the
    /// full rebuild is the oracle.
    pub(super) free: BTreeMap<(i32, i32), u32>,
    /// Ids whose footprint the 40k-tile cap truncated: non-empty means the
    /// field is incomplete, so a field miss stops proving clear space and
    /// `overlaps_any` walks the exact tree instead.
    pub(super) clipped: BTreeSet<u32>,
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
            clipped: BTreeSet::new(),
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
        let (tiles, clipped) = free_tiles_of(&bx);
        free_add(&mut self.free, &tiles);
        if clipped {
            self.clipped.insert(bx.id);
        }
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
        let (tiles, _) = free_tiles_of(&bx);
        free_remove(&mut self.free, &tiles);
        self.clipped.remove(&id);
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
    /// tree walk; occupied tiles fall through to the exact test. A field
    /// miss proves clear space only when no box was clipped.
    pub fn overlaps_any(&self, probe: &Box3, ignore_id: u32) -> bool {
        if probe.min_x < probe.max_x && probe.min_y < probe.max_y && probe.min_z < probe.max_z {
            let pitch = crate::navcheck::TILE_CM.max(1);
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
                if !any_hit && self.clipped.is_empty() {
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
            clipped: BTreeSet::new(),
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

pub(super) fn sort_for_split(entries: &mut [Rentry], axis: usize, by_max: bool) {
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
pub(super) fn group_mbr(group: &[Rentry]) -> Mbr {
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
