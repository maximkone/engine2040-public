//! digest: shared fold for canonical words beside the full rehash.
//! Owner: engine-core (std-only). Units: hashes (u64). LE words, index order, no alloc.
//! RC9 experiment: [`MerkleDigest`] runs beside the full rehash and never replaces it.

use crate::hash::{Fnv64, FNV64_PRIME};

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
    /// A tree over `leaves`, padded with [`EMPTY_LEAF`] up to a power of two
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
}
