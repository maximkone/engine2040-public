//! digest: shared fold for canonical words beside the full rehash.
//! Owner: engine-core (std-only). Units: hashes (u64). LE words, index order, no alloc.
//! RC9 experiment: [`MerkleDigest`] runs beside the full rehash and never replaces it.
//! The per-tick [`DigestChain`] names the first tick and stable id where two
//! runs part; it reports, it never decides.

use crate::hash::{Fnv64, FNV64_OFFSET, FNV64_PRIME};

/// Twin FNV-1a fold, one index-ordered pass over two streams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TwinFold {
    a: Fnv64,
    b: Fnv64,
}

impl TwinFold {
    /// Fresh fold at the FNV offset basis for both streams.
    pub const fn new() -> Self {
        Self {
            a: Fnv64::new(),
            b: Fnv64::new(),
        }
    }
    /// Feed one stream-A word, preserving stream order.
    pub fn push_a_u32(&mut self, word: u32) {
        self.a.write_u32(word);
    }
    /// Feed one stream-B word, preserving stream order.
    pub fn push_b_u32(&mut self, word: u32) {
        self.b.write_u32(word);
    }
    /// Two-pass combine value, bit for bit.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.a.finish() ^ self.b.finish().wrapping_mul(FNV64_PRIME)
    }
}

/// Hash of the padding leaves.
const EMPTY_LEAF: u64 = 0;

/// Binary Merkle tree over `u64` leaf hashes (RC9 experiment). Changing one
/// leaf rehashes one root path, O(log n); the root summarises every leaf.
/// Two trees of the same size find their first differing leaf by descending
/// from the roots, O(log n), whatever the leaf count. Heap layout: node 1 is
/// the root, node `i` has children `2i` and `2i + 1`, leaves start at
/// `width`. It never stands in for the real state digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleDigest {
    nodes: Vec<u64>,
    width: usize,
}

impl MerkleDigest {
    /// A tree over `leaves`, padded with `EMPTY_LEAF` up to a power of two
    /// wide: O(n), one allocation, here.
    pub fn from_leaves(leaves: &[u64]) -> Self {
        let width = leaves.len().max(1).next_power_of_two();
        let mut nodes = vec![EMPTY_LEAF; width * 2];
        nodes[width..width + leaves.len()].copy_from_slice(leaves);
        for node in (1..width).rev() {
            nodes[node] = Self::parent(nodes[2 * node], nodes[2 * node + 1]);
        }
        Self { nodes, width }
    }

    /// Number of leaf slots (a power of two, at least the requested count).
    pub fn width(&self) -> usize {
        self.width
    }

    /// The summary of every leaf.
    pub fn root(&self) -> u64 {
        self.nodes[1]
    }

    /// Leaf hash `index`, or `None` past the width.
    pub fn leaf(&self, index: usize) -> Option<u64> {
        (index < self.width).then(|| self.nodes[self.width + index])
    }

    /// Set a leaf and rehash its path, stopping where a node does not change.
    /// An index past the width does nothing and returns `false`.
    pub fn set_leaf(&mut self, index: usize, hash: u64) -> bool {
        if index >= self.width {
            return false;
        }
        let mut node = self.width + index;
        if self.nodes[node] == hash {
            return true;
        }
        self.nodes[node] = hash;
        while node > 1 {
            node /= 2;
            let parent = Self::parent(self.nodes[2 * node], self.nodes[2 * node + 1]);
            if self.nodes[node] == parent {
                break;
            }
            self.nodes[node] = parent;
        }
        true
    }

    /// First (lowest index) leaf at which two trees of the same width differ,
    /// found from the roots down; `None` when they are equal.
    pub fn first_divergence(&self, other: &Self) -> Option<usize> {
        if self.width != other.width || self.root() == other.root() {
            return None;
        }
        let mut node = 1;
        while node < self.width {
            node *= 2;
            if self.nodes[node] == other.nodes[node] {
                node += 1;
            }
        }
        Some(node - self.width)
    }

    /// Parent hash: FNV-1a over both child words, left first.
    fn parent(left: u64, right: u64) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(left);
        h.write_u64(right);
        h.finish()
    }
}

/// A stable id in a digest chain: the name a divergence reports.
///
/// The caller owns the numbering (a slot, an entity, a subject index). One id
/// means one thing for the whole chain, so a report that names it names the
/// same thing in every run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId(u64);

impl StableId {
    /// The id of a numbered thing (a slot, an entity, a subject).
    #[must_use]
    pub fn slot(slot: u32) -> Self {
        Self(u64::from(slot))
    }

    /// The raw id value, for receipts and maps outside this crate.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl core::fmt::Display for StableId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "stable:{:016x}", self.0)
    }
}

/// One record hashed inside a tick: a stable id and its digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainRecord {
    /// The tick the record was hashed at.
    pub tick: u32,
    /// The stable id of the hashed thing.
    pub id: StableId,
    /// The digest of that thing at that tick.
    pub digest: u64,
}

/// One tick of a chain: the tick number and the chain digest after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickDigest {
    /// The tick number.
    pub tick: u32,
    /// The chain digest after folding this tick.
    pub digest: u64,
}

/// Where two chains parted: the first divergent tick and the stable id of
/// the record that differs there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The first tick whose chain digest differs.
    pub tick: u32,
    /// The stable id of the record that differs at that tick, taken from the
    /// left chain.
    pub id: StableId,
}

impl core::fmt::Display for Divergence {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "divergence at tick {} id {}", self.tick, self.id)
    }
}

/// A per-tick digest chain: the tool that names a divergence.
///
/// Every tick folds its records (one digest per stable id, in stable id
/// order) into the running chain digest, so a chain digest covers every
/// earlier tick. Two chains that part at tick `T` share every earlier tick
/// digest and differ from `T` on; [`DigestChain::first_divergence`] names
/// that tick and the stable id of the record that differs there.
///
/// The chain reports, it never decides: it names the first difference, and
/// the caller decides what to do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestChain {
    /// The running chain digest.
    chain: u64,
    /// The chain digest after each recorded tick.
    ticks: Vec<TickDigest>,
    /// Every record, in tick order and then stable id order.
    records: Vec<ChainRecord>,
}

impl Default for DigestChain {
    fn default() -> Self {
        Self::new()
    }
}

impl DigestChain {
    /// An empty chain, at the FNV offset basis.
    #[must_use]
    pub fn new() -> Self {
        Self {
            chain: FNV64_OFFSET,
            ticks: Vec::new(),
            records: Vec::new(),
        }
    }

    /// An empty chain with room for `ticks` ticks and `records` records, so
    /// a caller that knows the length never grows the chain while it runs.
    #[must_use]
    pub fn with_capacity(ticks: usize, records: usize) -> Self {
        Self {
            chain: FNV64_OFFSET,
            ticks: Vec::with_capacity(ticks),
            records: Vec::with_capacity(records),
        }
    }

    /// Fold one tick: the records hashed at `tick`, in stable id order.
    ///
    /// The chain digest after the tick covers every earlier tick, so a
    /// difference in any earlier tick moves every later digest. A tick with
    /// no records is not recorded: nothing was hashed there, so there is
    /// nothing to compare.
    pub fn push_tick(&mut self, tick: u32, records: &[(StableId, u64)]) {
        if records.is_empty() {
            return;
        }
        debug_assert!(
            records.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "records arrive in stable id order"
        );
        let mut state = Fnv64::with_state(self.chain);
        state.write_u32(tick);
        for (id, digest) in records {
            state.write_u64(id.raw());
            state.write_u64(*digest);
            self.records.push(ChainRecord {
                tick,
                id: *id,
                digest: *digest,
            });
        }
        self.chain = state.finish();
        self.ticks.push(TickDigest {
            tick,
            digest: self.chain,
        });
    }

    /// The running chain digest: the digest after the last recorded tick.
    #[must_use]
    pub fn digest(&self) -> u64 {
        self.chain
    }

    /// The chain digest after each recorded tick.
    #[must_use]
    pub fn ticks(&self) -> &[TickDigest] {
        &self.ticks
    }

    /// The number of recorded ticks.
    #[must_use]
    pub fn tick_count(&self) -> usize {
        self.ticks.len()
    }

    /// Every record, in tick order and then stable id order.
    #[must_use]
    pub fn records(&self) -> &[ChainRecord] {
        &self.records
    }

    /// The first tick where two chains differ, with the stable id of the
    /// record that differs there. `None` when the chains agree.
    ///
    /// The tick is the first one whose chain digest differs, so a difference
    /// in an earlier tick is never reported as a later one. The id comes
    /// from the left chain.
    #[must_use]
    pub fn first_divergence(&self, other: &Self) -> Option<Divergence> {
        let index = crate::diverge::first_divergence(&self.ticks, &other.ticks)?;
        let index = usize::try_from(index).ok()?;
        let tick = self
            .ticks
            .get(index)
            .map(|entry| entry.tick)
            .or_else(|| other.ticks.get(index).map(|entry| entry.tick))?;
        let id = self.differing_id(other, tick)?;
        Some(Divergence { tick, id })
    }

    /// The id of the first record that differs at `tick`, from either side.
    fn differing_id(&self, other: &Self, tick: u32) -> Option<StableId> {
        let left = self.records_at(tick);
        let right = other.records_at(tick);
        let shared = left.len().min(right.len());
        for (mine, theirs) in left.iter().zip(right.iter()) {
            if mine.id != theirs.id || mine.digest != theirs.digest {
                return Some(mine.id);
            }
        }
        if left.len() != right.len() {
            let extra = if left.len() > right.len() {
                &left[shared..]
            } else {
                &right[shared..]
            };
            return extra.first().map(|record| record.id);
        }
        None
    }

    /// The records of one tick, as a window of the record list.
    fn records_at(&self, tick: u32) -> &[ChainRecord] {
        let start = self.records.partition_point(|record| record.tick < tick);
        let end = start + self.records[start..].partition_point(|record| record.tick == tick);
        &self.records[start..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;
    use crate::CounterRng;

    fn leaves(seed: u64, n: u64) -> Vec<u64> {
        let rng = CounterRng::new(seed);
        (0..n).map(|i| rng.sample(i)).collect()
    }

    #[test]
    fn width_pads_to_a_power_of_two_and_root_summarises() {
        assert_eq!(MerkleDigest::from_leaves(&[]).width(), 1);
        assert_eq!(MerkleDigest::from_leaves(&[7]).width(), 1);
        assert_eq!(MerkleDigest::from_leaves(&leaves(1, 5)).width(), 8);
        assert_eq!(MerkleDigest::from_leaves(&leaves(1, 8)).width(), 8);
        let two = MerkleDigest::from_leaves(&[3, 4]);
        assert_eq!(
            two.root(),
            MerkleDigest::parent(3, 4),
            "two leaves: one parent"
        );
        assert_eq!((two.leaf(1), two.leaf(2)), (Some(4), None));
        assert_ne!(
            two.root(),
            MerkleDigest::from_leaves(&[4, 3]).root(),
            "order counts"
        );
    }

    #[test]
    fn set_leaf_matches_a_full_rebuild() {
        let rng = CounterRng::new(2040);
        let mut now = leaves(9, 37);
        let mut tree = MerkleDigest::from_leaves(&now);
        for i in 0..400u64 {
            let at = usize::try_from(rng.sample(i) % 37).unwrap_or(0);
            let hash = rng.sample(i + 1_000);
            now[at] = hash;
            assert!(tree.set_leaf(at, hash));
            assert_eq!(tree, MerkleDigest::from_leaves(&now), "update {i}");
        }
        assert!(!tree.set_leaf(64, 1), "past the width does nothing");
        let before = tree.root();
        assert!(tree.set_leaf(3, now[3]), "same hash is a no-op");
        assert_eq!(tree.root(), before);
    }

    #[test]
    fn first_divergence_is_the_lowest_differing_leaf_20_of_20() {
        let rng = CounterRng::new(77);
        for seed in 0..20u64 {
            let n = 1 + rng.sample(seed) % 200;
            let base = leaves(seed, n);
            let mut moved = base.clone();
            let first = usize::try_from(rng.sample(seed + 500) % n).unwrap_or(0);
            // One differing leaf at `first`, and sometimes later ones.
            for at in [first, first + 3, first + 11] {
                if at < moved.len()
                    && (at == first || rng.sample(seed + at as u64).is_multiple_of(2))
                {
                    moved[at] ^= 1 << (seed % 64);
                }
            }
            let (a, b) = (
                MerkleDigest::from_leaves(&base),
                MerkleDigest::from_leaves(&moved),
            );
            assert_ne!(a.root(), b.root(), "seed {seed}");
            let scan = base.iter().zip(&moved).position(|(x, y)| x != y);
            assert_eq!(a.first_divergence(&b), scan, "seed {seed}");
            assert_eq!(b.first_divergence(&a), scan, "symmetric, seed {seed}");
            assert_eq!(a.first_divergence(&a.clone()), None);
        }
        let (small, big) = (
            MerkleDigest::from_leaves(&[1]),
            MerkleDigest::from_leaves(&[1, 2, 3]),
        );
        assert_eq!(
            small.first_divergence(&big),
            None,
            "different widths do not compare"
        );
    }

    #[test]
    fn updates_and_descent_never_allocate() {
        let base = leaves(5, 1024);
        let mut tree = MerkleDigest::from_leaves(&base);
        let other = MerkleDigest::from_leaves(&base);
        let ((), heap) = measure(|| {
            for i in 0..1024u64 {
                tree.set_leaf(usize::try_from(i).unwrap_or(0), i ^ 0xABCD);
            }
            std::hint::black_box(tree.first_divergence(&other));
            std::hint::black_box(tree.root());
        });
        assert_eq!(heap.calls, 0, "the warm path must not allocate");
    }

    /// One record digest of a seeded run: counter-based, so any tick and
    /// slot can be recomputed without replaying the whole chain.
    fn seeded_digest(seed: u64, tick: u32, slot: u32) -> u64 {
        let rng = CounterRng::new(seed);
        let counter = seed
            .wrapping_mul(1_000)
            .wrapping_add(u64::from(tick) * 8)
            .wrapping_add(u64::from(slot));
        rng.sample(counter)
    }

    /// The records of one tick of a seeded run.
    fn seeded_records(seed: u64, tick: u32) -> Vec<(StableId, u64)> {
        (0..3u32)
            .map(|slot| (StableId::slot(slot), seeded_digest(seed, tick, slot)))
            .collect()
    }

    /// A chain of `ticks` ticks, three subjects per tick, seeded.
    fn seeded_chain(seed: u64, ticks: u32) -> DigestChain {
        let mut chain = DigestChain::new();
        for tick in 0..ticks {
            chain.push_tick(tick, &seeded_records(seed, tick));
        }
        chain
    }

    /// The same chain with one record changed at `tick`.
    fn moved_chain(seed: u64, ticks: u32, tick: u32, slot: u32) -> DigestChain {
        let mut chain = DigestChain::new();
        for at in 0..ticks {
            let mut records = seeded_records(seed, at);
            if at == tick {
                let digest = records[usize::try_from(slot).unwrap_or(0)].1;
                records[usize::try_from(slot).unwrap_or(0)].1 = digest.wrapping_add(1);
            }
            chain.push_tick(at, &records);
        }
        chain
    }

    #[test]
    fn digest_chain_names_the_first_divergent_tick_and_id_on_20_of_20_seeded_divergences() {
        let rng = CounterRng::new(2040);
        for seed in 0..20u64 {
            let ticks = 12 + u32::try_from(rng.sample(seed) % 40).unwrap_or(0);
            let at = u32::try_from(rng.sample(seed + 100) % u64::from(ticks)).unwrap_or(0);
            let slot = u32::try_from(rng.sample(seed + 200) % 3).unwrap_or(0);
            let base = seeded_chain(seed, ticks);
            let moved = moved_chain(seed, ticks, at, slot);
            assert_ne!(
                base.digest(),
                moved.digest(),
                "seed {seed}: the chains differ"
            );
            assert_eq!(
                base.first_divergence(&moved),
                Some(Divergence {
                    tick: at,
                    id: StableId::slot(slot)
                }),
                "seed {seed}: the first divergent tick and id"
            );
            assert_eq!(
                moved.first_divergence(&base).map(|found| found.tick),
                Some(at),
                "seed {seed}: the tick is symmetric"
            );
            assert_eq!(
                base.first_divergence(&base.clone()),
                None,
                "seed {seed}: equal chains never diverge"
            );
        }
    }

    #[test]
    fn chain_digest_covers_every_earlier_tick() {
        let base = seeded_chain(3, 20);
        let moved = moved_chain(3, 20, 7, 1);
        for (index, (mine, theirs)) in base.ticks().iter().zip(moved.ticks()).enumerate() {
            let tick = u32::try_from(index).unwrap_or(0);
            if tick < 7 {
                assert_eq!(
                    mine.digest, theirs.digest,
                    "tick {tick}: earlier ticks agree"
                );
            } else {
                assert_ne!(mine.digest, theirs.digest, "tick {tick}: later ticks move");
            }
        }
        assert_eq!(base.tick_count(), 20);
        assert_eq!(base.records().len(), 60, "three subjects per tick");
    }

    #[test]
    fn chain_names_the_id_of_the_record_that_differs() {
        let mut base = DigestChain::new();
        let mut moved = DigestChain::new();
        base.push_tick(4, &[(StableId::slot(1), 10), (StableId::slot(2), 20)]);
        moved.push_tick(4, &[(StableId::slot(1), 10), (StableId::slot(2), 21)]);
        assert_eq!(
            base.first_divergence(&moved),
            Some(Divergence {
                tick: 4,
                id: StableId::slot(2)
            }),
            "the id of the record that differs is named"
        );
        assert_eq!(
            moved.first_divergence(&base),
            Some(Divergence {
                tick: 4,
                id: StableId::slot(2)
            }),
            "the same id is named from the other side"
        );
    }

    #[test]
    fn a_chain_that_ran_longer_diverges_at_the_shared_end() {
        let short = seeded_chain(9, 5);
        let long = seeded_chain(9, 8);
        assert_eq!(
            short.first_divergence(&long),
            Some(Divergence {
                tick: 5,
                id: StableId::slot(0)
            }),
            "the extra tick is named"
        );
        assert_eq!(
            long.first_divergence(&short).map(|found| found.tick),
            Some(5)
        );
    }

    #[test]
    fn an_empty_tick_is_not_recorded() {
        let mut chain = DigestChain::new();
        chain.push_tick(0, &[]);
        assert_eq!(
            chain.tick_count(),
            0,
            "nothing was hashed, nothing is recorded"
        );
        assert_eq!(chain.digest(), FNV64_OFFSET, "the chain never moved");
    }
}
