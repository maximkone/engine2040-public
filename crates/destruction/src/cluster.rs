//! cluster: sparse bond topology over a bonded descriptor.
//!
//! Consumer: `bonded.rs` owns the runtime collection; this module reads its
//! frozen [`BondedDesc`] bond list plus anchor set and answers topology
//! questions (neighbors, intact/broken sets, anchor connectivity) in index
//! order for the damage-propagation leg. Destruction never imports arena.
//! Units: chunk and bond indices; no lengths, no ticks. Determinism:
//! index-order loops with `+ - * /` only, per-chunk adjacency sorted
//! ascending, bit-exact digest; the same descriptor builds the same graph on
//! every PC. Errors: [`BreakError`]. Budget: construction scans the bond
//! list once; connectivity floods allocate (analysis only, never the tick).
//!
//! Shape only from PORT(flax-mcp
//! `plugins/world/src/Physics/Core/Packs/Destruction/Destruction.ClusterGraph.cs`
//! + `Destructible/Destructible.cs`, MIT, Maxim's own code): chunk count,
//! sparse adjacency, broken flags, anchor set, intact-vs-broken plus anchor
//! connectivity. Reimplemented integer-only with no bytes taken and no
//! vendored file; break propagation and the swap director land in later
//! pieces beside this topology, never as a second graph.

use crate::bonded::{flagged_indices, slot, wire, BondedDesc, BreakError};
use engine_core::Fnv64;

/// Sparse bond topology of one bonded descriptor: sorted adjacency over the
/// descriptor bonds in index order, the anchor set, and the intact/broken
/// bond flags. Fresh graphs start fully intact; later pieces snap bonds
/// through this same graph, never a copy.
#[derive(Clone, Debug)]
pub struct ClusterBondGraph {
    chunk_count: usize,
    bond_count: usize,
    /// Offsets: chunk `i` owns `adj[adj_start[i]..adj_start[i + 1]]`.
    adj_start: Vec<u32>,
    /// `(neighbor, bond)` pairs sorted ascending per chunk; built once.
    adj: Vec<(u32, u32)>,
    /// Anchored chunks; fixed in place, seeding every connectivity flood.
    anchored: Vec<bool>,
    /// Snapped bonds; fresh graphs hold none snapped.
    broken: Vec<bool>,
}

impl ClusterBondGraph {
    /// Builds the topology of a validated descriptor: adjacency from its
    /// bonds in index order, anchors from its anchor list, every bond
    /// intact. Construction input maps to [`BreakError`], never a panic.
    pub fn new(desc: &BondedDesc) -> Result<Self, BreakError> {
        desc.validate()?;
        let chunk_count = desc.chunks.len();
        let mut anchored = vec![false; chunk_count];
        for anchor in &desc.anchors {
            anchored[slot(*anchor)] = true;
        }
        let (adj_start, adj) = build_topology(chunk_count, &desc.bonds);
        Ok(Self {
            chunk_count,
            bond_count: desc.bonds.len(),
            adj_start,
            adj,
            anchored,
            broken: vec![false; desc.bonds.len()],
        })
    }

    /// Chunk count of the described collection.
    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    /// Bond count of the described collection.
    pub fn bond_count(&self) -> usize {
        self.bond_count
    }

    /// Anchor indices in ascending order.
    pub fn anchors(&self) -> Vec<u32> {
        flagged_indices(&self.anchored)
    }

    /// `(neighbor, bond)` pairs of one chunk in ascending order; out-of-range
    /// ids read empty, never panic.
    pub fn neighbors(&self, chunk: usize) -> &[(u32, u32)] {
        if chunk >= self.chunk_count {
            return &[];
        }
        let span = slot(self.adj_start[chunk])..slot(self.adj_start[chunk + 1]);
        &self.adj[span]
    }

    /// Bond count incident to one chunk; out-of-range ids read zero.
    pub fn degree(&self, chunk: usize) -> usize {
        self.neighbors(chunk).len()
    }

    /// True when the chunk is anchored; out-of-range ids read false.
    pub fn is_anchored(&self, chunk: usize) -> bool {
        self.anchored.get(chunk).copied().unwrap_or(false)
    }

    /// True when the bond has snapped; out-of-range ids read false.
    pub fn is_bond_broken(&self, bond: usize) -> bool {
        self.broken.get(bond).copied().unwrap_or(false)
    }

    /// Intact bond indices in ascending order.
    pub fn intact_bonds(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for (index, snapped) in self.broken.iter().enumerate() {
            if !snapped {
                out.push(wire(index));
            }
        }
        out
    }

    /// Snapped bond indices in ascending order; empty on a fresh graph.
    pub fn broken_bonds(&self) -> Vec<u32> {
        flagged_indices(&self.broken)
    }

    /// True when the chunk reaches an anchor over intact bonds at the last
    /// flood; out-of-range ids read false.
    pub fn is_supported(&self, chunk: usize) -> bool {
        if chunk >= self.chunk_count {
            return false;
        }
        self.connectivity()[chunk]
    }

    /// Anchor-connected chunk indices in ascending order: the intact set a
    /// fresh graph posts before any break. Allocates: analysis, never the tick.
    pub fn intact_set(&self) -> Vec<u32> {
        flagged_indices(&self.connectivity())
    }

    /// Anchor-connected chunk count at the last flood.
    pub fn supported_count(&self) -> usize {
        self.connectivity().iter().filter(|on| **on).count()
    }

    /// FNV-1a64 over chunk/bond counts, every adjacency pair in span order,
    /// and the anchor plus broken flags. Index order keeps reruns identical.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u32(wire(self.chunk_count));
        hash.write_u32(wire(self.bond_count));
        for offset in &self.adj_start {
            hash.write_u32(*offset);
        }
        for (neighbor, bond) in &self.adj {
            hash.write_u32(*neighbor);
            hash.write_u32(*bond);
        }
        for anchored in &self.anchored {
            hash.write_u32(u32::from(*anchored));
        }
        for snapped in &self.broken {
            hash.write_u32(u32::from(*snapped));
        }
        hash.finish()
    }

    /// Anchor connectivity over intact bonds: breadth-first layers out of the
    /// anchors, neighbors visited in ascending span order so parent choice
    /// is canonical. Allocates: analysis and tests only, never the tick.
    fn connectivity(&self) -> Vec<bool> {
        let mut supported = vec![false; self.chunk_count];
        let mut work: Vec<usize> = Vec::new();
        for (index, anchored) in self.anchored.iter().enumerate() {
            if *anchored {
                supported[index] = true;
                work.push(index);
            }
        }
        let mut layer_start = 0;
        while layer_start < work.len() {
            let layer_end = work.len();
            work[layer_start..layer_end].sort_unstable();
            for cursor in layer_start..layer_end {
                let current = work[cursor];
                let span = slot(self.adj_start[current])..slot(self.adj_start[current + 1]);
                for (neighbor, bond) in &self.adj[span] {
                    if self.broken[slot(*bond)] {
                        continue;
                    }
                    let next = slot(*neighbor);
                    if !supported[next] {
                        supported[next] = true;
                        work.push(next);
                    }
                }
            }
            layer_start = layer_end;
        }
        supported
    }
}

/// Sorted adjacency in compressed form: offsets per chunk plus
/// `(neighbor, bond)` pairs ascending within each chunk's span, so every
/// traversal visits neighbors in one canonical order. The bonded builder is
/// private and its module frozen this piece, so the topology owns this copy.
fn build_topology(
    chunk_count: usize,
    bonds: &[crate::bonded::Bond],
) -> (Vec<u32>, Vec<(u32, u32)>) {
    let mut offsets = vec![0u32; chunk_count + 1];
    for bond in bonds {
        offsets[slot(bond.a) + 1] += 1;
        offsets[slot(bond.b) + 1] += 1;
    }
    for index in 0..chunk_count {
        offsets[index + 1] += offsets[index];
    }
    let mut fill = offsets.clone();
    let mut adj = vec![(0u32, 0u32); slot(offsets[chunk_count])];
    for (bond_index, bond) in bonds.iter().enumerate() {
        let id = wire(bond_index);
        let first = slot(bond.a);
        adj[slot(fill[first])] = (bond.b, id);
        fill[first] += 1;
        let second = slot(bond.b);
        adj[slot(fill[second])] = (bond.a, id);
        fill[second] += 1;
    }
    for index in 0..chunk_count {
        adj[slot(offsets[index])..slot(offsets[index + 1])].sort_unstable();
    }
    (offsets, adj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::bonded::{Bond, Chunk3};

    /// Twelve shards in a row: chunk 0 anchored, bonds link neighbors only.
    fn chain_twelve() -> BondedDesc {
        let mut chunks = Vec::new();
        for index in 0..12 {
            chunks.push(Chunk3 {
                x: index as f32,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            });
        }
        let mut bonds = Vec::new();
        for index in 0..11 {
            bonds.push(Bond {
                a: wire(index),
                b: wire(index + 1),
                breaking_strain: 1.0,
            });
        }
        BondedDesc {
            chunks,
            bonds,
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        }
    }

    #[test]
    fn cluster_topology_intact_until_break() {
        let desc = chain_twelve();
        let graph = ClusterBondGraph::new(&desc).expect("valid chain builds");
        // Anchor-connected fixture posts the full intact set before any break.
        let intact: Vec<u32> = (0..12).collect();
        assert_eq!(graph.intact_set(), intact);
        assert_eq!(graph.supported_count(), 12);
        assert_eq!(graph.intact_bonds().len(), 11);
        assert!(graph.broken_bonds().is_empty());
        // Middle shard sees exactly its two bond neighbors in index order.
        assert_eq!(graph.neighbors(5), &[(4, 4), (6, 5)]);
        assert_eq!(graph.degree(0), 1);
        assert_eq!(graph.degree(11), 1);
        assert_eq!(graph.anchors(), vec![0]);
        // Same descriptor twice builds the same topology and digest.
        let again = ClusterBondGraph::new(&desc).expect("rebuilds");
        assert_eq!(again.intact_set(), graph.intact_set());
        assert_eq!(again.digest(), graph.digest());
    }

    #[test]
    fn cluster_topology_rejects_bad_descriptors_without_panic() {
        let good = chain_twelve();
        // Self-bond at index 0 is rejected, nothing is built.
        let looped = BondedDesc {
            bonds: vec![Bond {
                a: 3,
                b: 3,
                breaking_strain: 1.0,
            }],
            ..good.clone()
        };
        assert_eq!(
            ClusterBondGraph::new(&looped).unwrap_err(),
            BreakError::BadBond(0)
        );
        // Anchor past the end is rejected as a bad chunk.
        let dangling = BondedDesc {
            anchors: vec![40],
            ..good.clone()
        };
        assert_eq!(
            ClusterBondGraph::new(&dangling).unwrap_err(),
            BreakError::BadChunk(40)
        );
        // Anchorless graphs build but nothing is supported: out-of-range
        // reads stay false and empty without panicking.
        let anchorless = BondedDesc {
            anchors: Vec::new(),
            ..good.clone()
        };
        let graph = ClusterBondGraph::new(&anchorless).expect("anchorless builds");
        assert!(graph.intact_set().is_empty());
        assert_eq!(graph.supported_count(), 0);
        assert!(!graph.is_supported(0) && !graph.is_supported(99));
        assert!(!graph.is_anchored(99) && !graph.is_bond_broken(99));
        assert!(graph.neighbors(99).is_empty());
        assert_eq!(graph.degree(99), 0);
    }
}
