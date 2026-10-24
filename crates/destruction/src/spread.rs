//! spread: crack spread as a sorted flood over the baked bond graph.
//!
//! Consumer: debris budgets drain the break list; the arena apply path calls
//! in (no intra-crate caller yet).
//! Units: strain Q16 fixed-point, ticks on events. Determinism: canonical
//! bond order, sorted worklist, integer-only math; reruns are bit-identical.
//! Errors: [`SpreadError`]. Budget: 0.5 ms per spread event (bounded scans
//! over at most 4096 bonds; one neighbor scratch plus the per-event output
//! vec allocate per call, nothing per break or per pop).
//!
//! Mirror: bond/event/error shapes repeat `bonded.rs` so this file stays
//! std-only and `rustc --test` clean; they can be swapped for `super::bonded`
//! imports plus `From` impls once a caller needs the bonded types here.

use crate::bonded::{flagged_indices, slot, wire, BondedDesc, BreakError};
use crate::cluster::{Cascade, ClusterBondGraph};
use engine_core::Fnv64;
use std::collections::VecDeque;

/// Fixed-point scale for strain: Q16, matches `engine-core` `Fixed::SCALE`.
pub const STRAIN_SCALE: i64 = 65536;
/// Largest chunk count one spread graph holds; mirrors `bonded` bounds.
pub const MAX_SPREAD_CHUNKS: usize = 1024;
/// Largest bond count; mirrors `bonded` bounds.
pub const MAX_SPREAD_BONDS: usize = 4096;
/// Largest accepted strain value; mirrors the `bonded` limit range.
pub const MAX_STRAIN_F32: f32 = 1e6;
/// [`MAX_STRAIN_F32`] in Q16.
pub const MAX_STRAIN_Q16: i64 = 65_536_000_000;
/// Strain carried to each intact neighbor per broken bond: 3/4, integer math.
pub const SPREAD_NUM: i64 = 3;
/// Denominator of the neighbor carry fraction.
pub const SPREAD_DEN: i64 = 4;

/// Converts a strain value to Q16 fixed-point; rejects non-finite,
/// negative and absurd inputs instead of panicking.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "a finite strain at or below MAX_STRAIN_F32 (1e6) times the Q16 scale stays at or below MAX_STRAIN_Q16 (6.55e10), far inside i64 and f32's exact integer range"
)]
pub fn strain_to_fixed(strain: f32) -> Result<i64, SpreadError> {
    if !strain.is_finite() || !(0.0..=MAX_STRAIN_F32).contains(&strain) {
        return Err(SpreadError::BadStrain);
    }
    Ok((strain * STRAIN_SCALE as f32) as i64)
}

/// Converts Q16 strain back to float for break-event payloads; the flood
/// itself never reads this, it only reports through it.
#[expect(
    clippy::cast_precision_loss,
    reason = "an event payload, bounded by MAX_STRAIN_Q16 (6.55e10); the low bits it drops are below the report's precision by contract"
)]
pub fn strain_to_f32(strain_q16: i64) -> f32 {
    strain_q16 as f32 / STRAIN_SCALE as f32
}

/// One bond between two chunk indices with its Q16 strain limit.
/// TEMPORARY MIRROR of `bonded::Bond` (float limit becomes Q16 here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpreadBond {
    /// First chunk index.
    pub a: u32,
    /// Second chunk index, never equal to `a`.
    pub b: u32,
    /// Strain above which the bond snaps; non-negative Q16.
    pub threshold_q16: i64,
}

/// Typed break event: bond snaps and chunk frees are separate variants.
/// TEMPORARY MIRROR of `bonded::BreakEvent`: identical variants, fields
/// and order so logs read the same once the orchestrator unifies them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpreadEvent {
    /// Bond `bond` snapped at `tick` once strain passed its limit.
    BondBroken {
        /// Match tick of the snap.
        tick: u32,
        /// Bond index in canonical bond order.
        bond: u32,
        /// Strain that snapped it.
        strain: f32,
    },
    /// Chunk `chunk` lost its last path to an anchor at `tick` and falls.
    ChunkFreed {
        /// Match tick of the free.
        tick: u32,
        /// Chunk index.
        chunk: u32,
    },
}

/// Canonical 13-byte form, byte-identical to `bonded::event_bytes`: tag
/// plus tick plus id plus payload (strain bits for bonds, zero for frees).
pub fn spread_event_bytes(event: &SpreadEvent) -> [u8; 13] {
    let mut out = [0u8; 13];
    match *event {
        SpreadEvent::BondBroken { tick, bond, strain } => {
            out[0] = 0;
            out[1..5].copy_from_slice(&tick.to_le_bytes());
            out[5..9].copy_from_slice(&bond.to_le_bytes());
            out[9..13].copy_from_slice(&strain.to_bits().to_le_bytes());
        }
        SpreadEvent::ChunkFreed { tick, chunk } => {
            out[0] = 1;
            out[1..5].copy_from_slice(&tick.to_le_bytes());
            out[5..9].copy_from_slice(&chunk.to_le_bytes());
        }
    }
    out
}

/// FNV-1a64 over the canonical event bytes; identical spreads hash equal.
pub fn spread_digest(events: &[SpreadEvent]) -> u64 {
    let mut h = Fnv64::new();
    for event in events {
        h.write(&spread_event_bytes(event));
    }
    h.finish()
}

/// Anchor-connected chunks of a bonded descriptor through the sparse topology.
///
/// Propagation-leg query: builds the single [`ClusterBondGraph`] for `desc`
/// and returns its anchor-connected set in index order with its bit-exact
/// digest. Units: chunk indices. Determinism: index-order flood, same input
/// gives same set and digest on every PC. Errors: [`BreakError`] for a bad
/// descriptor, never a panic. Budget: off-tick query only; the flood
/// allocates (analysis, never the tick).
pub fn cluster_support_and_digest(desc: &BondedDesc) -> Result<(Vec<u32>, u64), BreakError> {
    let graph = ClusterBondGraph::new(desc)?;
    Ok((graph.intact_set(), graph.digest()))
}

/// Anchor-connected chunks and the swing of one damage cascade through the
/// sparse topology.
///
/// Propagation-leg query: builds the single [`ClusterBondGraph`] for `desc`,
/// replays the bonds that already snapped, spreads `impact` from `origin`
/// along intact bonds ([`crate::cluster::CASCADE_FALLOFF_PERMILLE`] per hop,
/// never crossing a snapped one) and returns the cascade, the anchor-connected
/// set still standing and the bit-exact digest. Units: chunk indices, integer
/// impact points against an integer `toughness`. Errors: [`BreakError`] for a
/// bad descriptor, bond or impact, never a panic. Off-tick query only: the
/// walk allocates (analysis, never the tick). Consumer: the door's break
/// verbs, DX11.
pub fn cluster_cascade_and_digest(
    desc: &BondedDesc,
    snapped: &[u32],
    origin: u32,
    impact: u32,
    toughness: u32,
) -> Result<(Cascade, Vec<u32>, u64), BreakError> {
    let mut graph = ClusterBondGraph::new(desc)?;
    for bond in snapped {
        graph.snap_bond(*bond)?;
    }
    let cascade = graph.cascade(origin, impact, toughness)?;
    Ok((cascade, graph.intact_set(), graph.digest()))
}

/// Typed validation failures; external input maps here, never to a panic.
/// TEMPORARY MIRROR of the `bonded::BreakError` subset this flood needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadError {
    /// No chunks supplied.
    EmptyChunks,
    /// More than `MAX_SPREAD_CHUNKS` chunks supplied.
    TooManyChunks,
    /// More than `MAX_SPREAD_BONDS` bonds supplied.
    TooManyBonds,
    /// Chunk or anchor index out of range, culprit carried.
    BadChunk(usize),
    /// Bond endpoint, limit or duplicate at the given bond index.
    BadBond(usize),
    /// Seed strain negative (thresholds reject through `BadBond`).
    BadStrain,
}

impl core::fmt::Display for SpreadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpreadError::EmptyChunks => write!(f, "spread graph needs chunks"),
            SpreadError::TooManyChunks => write!(f, "spread graph exceeds MAX_SPREAD_CHUNKS"),
            SpreadError::TooManyBonds => write!(f, "spread graph exceeds MAX_SPREAD_BONDS"),
            SpreadError::BadChunk(i) => write!(f, "spread chunk {i} is out of bounds"),
            SpreadError::BadBond(i) => write!(f, "spread bond {i} is out of bounds"),
            SpreadError::BadStrain => write!(f, "spread strain is out of bounds"),
        }
    }
}

impl std::error::Error for SpreadError {}

/// Baked bond graph in canonical order: bonds sorted by endpoint pair so
/// insertion order never affects ids, events or hashes. Anchors seed the
/// support search, so anchored chunks stay until their bonds fail.
#[derive(Clone, Debug)]
pub struct SpreadGraph {
    chunk_count: usize,
    bonds: Vec<SpreadBond>,
    adjacency: Vec<Vec<u32>>,
    anchored: Vec<bool>,
}

impl SpreadGraph {
    /// Builds the graph, validating chunks, bonds and anchors, then sorting
    /// bonds into canonical `(min, max)` endpoint order. Shuffled insertion
    /// of the same pairs yields the same graph, ids and spreads.
    pub fn new(
        chunk_count: usize,
        bonds: Vec<SpreadBond>,
        anchors: Vec<u32>,
    ) -> Result<Self, SpreadError> {
        if chunk_count == 0 {
            return Err(SpreadError::EmptyChunks);
        }
        if chunk_count > MAX_SPREAD_CHUNKS {
            return Err(SpreadError::TooManyChunks);
        }
        if bonds.len() > MAX_SPREAD_BONDS {
            return Err(SpreadError::TooManyBonds);
        }
        for (i, bond) in bonds.iter().enumerate() {
            let ends_ok = slot(bond.a) < chunk_count && slot(bond.b) < chunk_count;
            if !ends_ok || bond.a == bond.b {
                return Err(SpreadError::BadBond(i));
            }
            if bond.threshold_q16 < 0 || bond.threshold_q16 > MAX_STRAIN_Q16 {
                return Err(SpreadError::BadBond(i));
            }
        }
        // Duplicate pairs reject like `bonded` does, on input indices.
        let mut pairs: Vec<(u32, u32, usize)> = bonds
            .iter()
            .enumerate()
            .map(|(i, bond)| (bond.a.min(bond.b), bond.a.max(bond.b), i))
            .collect();
        pairs.sort_unstable();
        for window in pairs.windows(2) {
            if window[0].0 == window[1].0 && window[0].1 == window[1].1 {
                return Err(SpreadError::BadBond(window[1].2));
            }
        }
        for anchor in &anchors {
            if slot(*anchor) >= chunk_count {
                return Err(SpreadError::BadChunk(slot(*anchor)));
            }
        }
        // Canonical order erases insertion order; the input index breaks
        // full ties so the sort is total without relying on stability.
        let mut order: Vec<usize> = (0..bonds.len()).collect();
        order.sort_by(|&l, &r| {
            let bl = &bonds[l];
            let br = &bonds[r];
            (bl.a.min(bl.b), bl.a.max(bl.b), bl.threshold_q16, l).cmp(&(
                br.a.min(br.b),
                br.a.max(br.b),
                br.threshold_q16,
                r,
            ))
        });
        let mut sorted = Vec::with_capacity(bonds.len());
        for i in order {
            sorted.push(bonds[i]);
        }
        let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); chunk_count];
        for (id, bond) in sorted.iter().enumerate() {
            adjacency[slot(bond.a)].push(wire(id));
            adjacency[slot(bond.b)].push(wire(id));
        }
        let mut anchored = vec![false; chunk_count];
        for anchor in &anchors {
            anchored[slot(*anchor)] = true;
        }
        Ok(Self {
            chunk_count,
            bonds: sorted,
            adjacency,
            anchored,
        })
    }

    /// Chunk count.
    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    /// Bond count in canonical order.
    pub fn bond_count(&self) -> usize {
        self.bonds.len()
    }

    /// Bonds in canonical order.
    pub fn bonds(&self) -> &[SpreadBond] {
        &self.bonds
    }

    /// Anchor indices in index order.
    pub fn anchors(&self) -> Vec<u32> {
        flagged_indices(&self.anchored)
    }

    /// True when the chunk is anchored; out-of-range ids read false.
    pub fn is_anchored(&self, chunk: usize) -> bool {
        self.anchored.get(chunk).copied().unwrap_or(false)
    }

    /// Q16 threshold of one bond; `None` when out of range.
    pub fn threshold_of(&self, bond: usize) -> Option<i64> {
        self.bonds.get(bond).map(|b| b.threshold_q16)
    }
}

/// Sorted flood from one hit bond: the seed strain snaps bonds past their
/// Q16 limits, each snap carries 3/4 of its strain to intact neighbors in
/// ascending bond-id order, then chunks without an unbroken-bond path to
/// an anchor free in index order. Bond events emit in canonical bond order
/// (like bonded index order), frees in chunk order. Only `+ - * /` on
/// integers; no floats, no hash-order iteration, no wall clock.
pub fn spread_cracks(
    graph: &SpreadGraph,
    hit_bond: usize,
    seed_strain_q16: i64,
    tick: u32,
) -> Result<Vec<SpreadEvent>, SpreadError> {
    if hit_bond >= graph.bonds.len() {
        return Err(SpreadError::BadBond(hit_bond));
    }
    if seed_strain_q16 < 0 {
        return Err(SpreadError::BadStrain);
    }
    let n = graph.bonds.len();
    let mut acc = vec![0i64; n];
    let mut broken = vec![false; n];
    let mut queued = vec![false; n];
    // One neighbor scratch reused for every broken bond, so the flood
    // allocates once per call (plus the output vec), never once per break.
    let mut neighbors: Vec<usize> = Vec::new();
    acc[hit_bond] = seed_strain_q16;
    // Sorted worklist of bond ids; ascending pop order is the flood order.
    // A deque pops the head in O(1) without shifting the whole worklist per
    // pop; inserts stay sorted by binary search, so the pop order (and every
    // carry, break and event) matches the old shifting Vec exactly.
    let mut pending: VecDeque<u32> = VecDeque::from([wire(hit_bond)]);
    queued[hit_bond] = true;
    while let Some(current) = pending.pop_front() {
        let current = slot(current);
        queued[current] = false;
        if broken[current] {
            continue;
        }
        if acc[current] > graph.bonds[current].threshold_q16 {
            broken[current] = true;
            let carry = i64::try_from(
                ((i128::from(acc[current]) * i128::from(SPREAD_NUM)) / i128::from(SPREAD_DEN))
                    .clamp(0, i128::from(i64::MAX)),
            )
            .unwrap_or(i64::MAX);
            if carry > 0 {
                let (a, c) = (slot(graph.bonds[current].a), slot(graph.bonds[current].b));
                merge_neighbors_into(
                    &graph.adjacency[a],
                    &graph.adjacency[c],
                    current,
                    &mut neighbors,
                );
                for &next in &neighbors {
                    if broken[next] {
                        continue;
                    }
                    acc[next] = acc[next].saturating_add(carry);
                    if !queued[next] {
                        let pos = pending.binary_search(&wire(next)).unwrap_or_else(|p| p);
                        pending.insert(pos, wire(next));
                        queued[next] = true;
                    }
                }
                neighbors.clear();
            }
        }
    }
    let mut events = Vec::new();
    for (id, _) in graph.bonds.iter().enumerate() {
        if broken[id] {
            events.push(SpreadEvent::BondBroken {
                tick,
                bond: wire(id),
                strain: strain_to_f32(acc[id]),
            });
        }
    }
    events.extend(frees_after(graph, &broken, tick));
    Ok(events)
}

/// Frees after the flood: a support search from the anchors over the intact
/// bonds. Anchored chunks seed it, so they stay until their own bonds fail.
/// Frees come out in chunk order, after every bond event.
fn frees_after(graph: &SpreadGraph, broken: &[bool], tick: u32) -> Vec<SpreadEvent> {
    let mut supported = vec![false; graph.chunk_count];
    let mut stack = Vec::new();
    for (i, anchored) in graph.anchored.iter().enumerate() {
        if *anchored {
            supported[i] = true;
            stack.push(i);
        }
    }
    while let Some(current) = stack.pop() {
        for bond in &graph.adjacency[current] {
            let id = slot(*bond);
            if broken[id] {
                continue;
            }
            let ends = graph.bonds[id];
            let (x, y) = (slot(ends.a), slot(ends.b));
            let next = if x == current {
                y
            } else if y == current {
                x
            } else {
                continue;
            };
            if !supported[next] {
                supported[next] = true;
                stack.push(next);
            }
        }
    }
    let mut frees = Vec::new();
    for (chunk, is_supported) in supported.iter().enumerate() {
        if !is_supported {
            frees.push(SpreadEvent::ChunkFreed {
                tick,
                chunk: wire(chunk),
            });
        }
    }
    frees
}

/// Merges two ascending bond-id lists into one ascending neighbor list,
/// skipping the bond being broken. Index order keeps the flood sorted.
/// Writes into caller-owned `out` (cleared first) so the flood reuses one
/// scratch instead of allocating per broken bond.
fn merge_neighbors_into(left: &[u32], right: &[u32], skip: usize, out: &mut Vec<usize>) {
    out.clear();
    out.reserve(left.len() + right.len());
    let (mut l, mut r) = (0usize, 0usize);
    while l < left.len() && r < right.len() {
        let (x, y) = (left[l], right[r]);
        if x <= y {
            if slot(x) != skip {
                out.push(slot(x));
            }
            l += 1;
        } else {
            if slot(y) != skip {
                out.push(slot(y));
            }
            r += 1;
        }
    }
    while l < left.len() {
        if slot(left[l]) != skip {
            out.push(slot(left[l]));
        }
        l += 1;
    }
    while r < right.len() {
        if slot(right[r]) != skip {
            out.push(slot(right[r]));
        }
        r += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(strain: f32) -> i64 {
        strain_to_fixed(strain).expect("test strain must be valid")
    }

    fn bond(a: u32, b: u32, limit: f32) -> SpreadBond {
        SpreadBond {
            a,
            b,
            threshold_q16: q(limit),
        }
    }

    /// Branched baked graph: 0 anchored, bonds (0,1) (1,2) (1,3) (2,4)
    /// (3,4), every limit 1.0. A 3.0 seed on (0,1) cascades everywhere.
    fn branched() -> (Vec<SpreadBond>, Vec<u32>) {
        (
            vec![
                bond(0, 1, 1.0),
                bond(1, 2, 1.0),
                bond(1, 3, 1.0),
                bond(2, 4, 1.0),
                bond(3, 4, 1.0),
            ],
            vec![0],
        )
    }

    fn branched_graph() -> SpreadGraph {
        let (bonds, anchors) = branched();
        SpreadGraph::new(5, bonds, anchors).expect("branched graph must build")
    }

    /// Three-chunk chain: 0 anchored, 1 middle, 2 tip.
    fn chain_graph() -> SpreadGraph {
        SpreadGraph::new(3, vec![bond(0, 1, 1.0), bond(1, 2, 1.0)], vec![0])
            .expect("chain graph must build")
    }

    #[test]
    fn casts_keep_the_spread_digest() {
        // Pinned from the code before the CQ10 cast pass on `spread.rs`: the event bytes and
        // their digest must not move.
        let graph = branched_graph();
        let events = spread_cracks(&graph, 0, q(3.0), 7).expect("valid spread");
        assert_eq!(spread_digest(&events), 11716394524248646560u64);
    }

    #[test]
    fn spread_replays_identical_twice() {
        let graph = branched_graph();
        let first = spread_cracks(&graph, 0, q(3.0), 7).expect("valid spread");
        let second = spread_cracks(&graph, 0, q(3.0), 7).expect("valid spread");
        assert_eq!(first, second);
        assert_eq!(spread_digest(&first), spread_digest(&second));
        assert_eq!(first.len(), 9, "5 snaps plus 4 frees: {first:?}");
        assert!(
            first
                .iter()
                .take(5)
                .all(|e| matches!(e, SpreadEvent::BondBroken { .. })),
            "bond events lead in canonical order: {first:?}"
        );
        assert!(
            first
                .iter()
                .skip(5)
                .all(|e| matches!(e, SpreadEvent::ChunkFreed { .. })),
            "free events trail in chunk order: {first:?}"
        );
    }

    /// Guard: the flood stays inside the repo's long-function limit
    /// (`tools/rust-quality.mjs` counts `fn` spans over 100 lines), so a
    /// later refactor cannot re-grow `spread_cracks` past the `long_fns`
    /// ratchet row the same way one already did.
    #[test]
    fn spread_cracks_stays_within_long_fn_limit() {
        const LIMIT: usize = 100;
        let src: &str = include_str!("spread.rs");
        let start = src
            .find("pub fn spread_cracks(")
            .expect("spread.rs keeps its flood entry point");
        let open = start + src[start..].find('{').expect("fn has a body");
        let mut depth = 0usize;
        let mut end = open;
        for (k, c) in src[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        let lines = src[start..=end].lines().count();
        assert_eq!(depth, 0, "spread_cracks body must close");
        assert!(
            lines <= LIMIT,
            "spread_cracks is {lines} lines (limit {LIMIT}): split it into named steps"
        );
    }

    /// Order proof: 8 insertion orders of the same pairs spread equal.
    #[test]
    fn insertion_shuffled_8_ways_same_output() {
        let (bonds, anchors) = branched();
        let mothers: Vec<Vec<SpreadBond>> = {
            let n = bonds.len();
            let mut orders = Vec::new();
            orders.push(bonds.clone());
            let mut rev = bonds.clone();
            rev.reverse();
            orders.push(rev);
            for k in [1usize, 2] {
                let mut rot = bonds.clone();
                rot.rotate_left(k % n);
                orders.push(rot);
            }
            let mut halves = bonds.clone();
            halves.rotate_left(n / 2);
            halves.reverse();
            orders.push(halves);
            let mut evens = Vec::new();
            let mut odds = Vec::new();
            for (i, b) in bonds.iter().enumerate() {
                if i % 2 == 0 {
                    evens.push(*b);
                } else {
                    odds.push(*b);
                }
            }
            evens.extend(odds);
            orders.push(evens);
            for seed in [7u64, 0x9E3779B97F4A7C15] {
                let mut shuffled = bonds.clone();
                lcg_shuffle(&mut shuffled, seed);
                orders.push(shuffled);
            }
            orders
        };
        assert_eq!(mothers.len(), 8);
        let baseline = {
            let graph =
                SpreadGraph::new(5, mothers[0].clone(), anchors.clone()).expect("baseline builds");
            spread_cracks(&graph, 0, q(3.0), 7).expect("baseline spreads")
        };
        for (k, order) in mothers.iter().enumerate() {
            let graph =
                SpreadGraph::new(5, order.clone(), anchors.clone()).expect("shuffle builds");
            // Canonical ids erase insertion: pairs stay sorted either way.
            let pairs: Vec<(u32, u32)> = graph
                .bonds()
                .iter()
                .map(|b| (b.a.min(b.b), b.a.max(b.b)))
                .collect();
            let mut sorted = pairs.clone();
            sorted.sort_unstable();
            assert_eq!(pairs, sorted, "order {k} must canonicalize");
            let events = spread_cracks(&graph, 0, q(3.0), 7).expect("shuffle spreads");
            assert_eq!(events, baseline, "insertion order {k} must not matter");
        }
    }

    #[test]
    fn anchored_chunks_stay_until_their_bonds_fail() {
        // Tip seed with a contained carry: the tip bond snaps at 1.25, its
        // 0.9375 carry holds the root bond, only the tip frees and falls.
        let graph = chain_graph();
        let tip = spread_cracks(&graph, 1, q(1.25), 9).expect("valid spread");
        assert_eq!(
            tip,
            vec![
                SpreadEvent::BondBroken {
                    tick: 9,
                    bond: 1,
                    strain: 1.25
                },
                SpreadEvent::ChunkFreed { tick: 9, chunk: 2 },
            ]
        );
        assert!(graph.is_anchored(0) && !graph.is_anchored(2));
        // Root seed with cascade strain: the middle follows the tip, while
        // the anchored head never frees no matter how hard the hit lands.
        let graph = chain_graph();
        let root = spread_cracks(&graph, 0, q(2.0), 9).expect("valid spread");
        assert_eq!(
            root,
            vec![
                SpreadEvent::BondBroken {
                    tick: 9,
                    bond: 0,
                    strain: 2.0
                },
                SpreadEvent::BondBroken {
                    tick: 9,
                    bond: 1,
                    strain: 1.5
                },
                SpreadEvent::ChunkFreed { tick: 9, chunk: 1 },
                SpreadEvent::ChunkFreed { tick: 9, chunk: 2 },
            ]
        );
    }

    #[test]
    fn partial_spread_holds_below_limit() {
        // Seed 1.125 on the tip: the tip snaps, its 0.84375 carry holds the
        // middle bond, so only the tip frees and the rest stays supported.
        let graph = chain_graph();
        let events = spread_cracks(&graph, 1, q(1.125), 4).expect("valid spread");
        assert_eq!(
            events,
            vec![
                SpreadEvent::BondBroken {
                    tick: 4,
                    bond: 1,
                    strain: 1.125
                },
                SpreadEvent::ChunkFreed { tick: 4, chunk: 2 },
            ]
        );
    }

    #[test]
    fn sub_threshold_seed_breaks_nothing() {
        let graph = chain_graph();
        // Seed exactly at the limit holds: snaps need strictly more strain.
        let held = spread_cracks(&graph, 0, q(1.0), 4).expect("valid spread");
        assert!(held.is_empty(), "limit strain must hold: {held:?}");
        // One Q16 LSB over the limit snaps the root bond; the whole tail
        // loses its anchor path, so both tail chunks free in index order.
        let snapped = spread_cracks(&graph, 0, 65536 + 1, 4).expect("valid spread");
        assert_eq!(
            snapped,
            vec![
                SpreadEvent::BondBroken {
                    tick: 4,
                    bond: 0,
                    strain: strain_to_f32(65537)
                },
                SpreadEvent::ChunkFreed { tick: 4, chunk: 1 },
                SpreadEvent::ChunkFreed { tick: 4, chunk: 2 },
            ]
        );
    }

    #[test]
    fn anchorless_graph_frees_everything_without_a_snap() {
        let graph =
            SpreadGraph::new(2, vec![bond(0, 1, 1.0)], Vec::new()).expect("anchorless builds");
        let events = spread_cracks(&graph, 0, q(0.5), 3).expect("valid spread");
        assert_eq!(
            events,
            vec![
                SpreadEvent::ChunkFreed { tick: 3, chunk: 0 },
                SpreadEvent::ChunkFreed { tick: 3, chunk: 1 },
            ]
        );
    }

    #[test]
    fn event_bytes_match_dx1_layout() {
        // Same canonical bytes `bonded::event_bytes` emits for these shapes.
        assert_eq!(
            spread_event_bytes(&SpreadEvent::BondBroken {
                tick: 9,
                bond: 1,
                strain: 1.5,
            }),
            [0, 9, 0, 0, 0, 1, 0, 0, 0, 0, 0, 192, 63]
        );
        assert_eq!(
            spread_event_bytes(&SpreadEvent::ChunkFreed { tick: 9, chunk: 2 }),
            [1, 9, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0]
        );
        let a = SpreadEvent::BondBroken {
            tick: 9,
            bond: 1,
            strain: 1.5,
        };
        assert_eq!(spread_event_bytes(&a).len(), 13);
        assert_ne!(
            spread_event_bytes(&a),
            spread_event_bytes(&SpreadEvent::ChunkFreed { tick: 9, chunk: 2 })
        );
    }

    #[test]
    fn strain_fixed_point_is_exact() {
        assert_eq!(strain_to_fixed(0.0), Ok(0));
        assert_eq!(strain_to_fixed(1.0), Ok(65536));
        assert_eq!(strain_to_fixed(1.5), Ok(98304));
        assert_eq!(strain_to_f32(65536), 1.0);
        assert_eq!(strain_to_f32(98304), 1.5);
        for bad in [f32::NAN, f32::INFINITY, -0.5, MAX_STRAIN_F32 + 1.0] {
            assert_eq!(strain_to_fixed(bad), Err(SpreadError::BadStrain));
        }
    }

    #[test]
    fn bad_construction_rejected() {
        let (bonds, anchors) = branched();
        assert_eq!(
            SpreadGraph::new(0, bonds.clone(), anchors.clone()).unwrap_err(),
            SpreadError::EmptyChunks
        );
        assert_eq!(
            SpreadGraph::new(MAX_SPREAD_CHUNKS + 1, bonds.clone(), anchors.clone()).unwrap_err(),
            SpreadError::TooManyChunks
        );
        assert_eq!(
            SpreadGraph::new(
                5,
                vec![bond(0, 1, 1.0); MAX_SPREAD_BONDS + 1],
                anchors.clone()
            )
            .unwrap_err(),
            SpreadError::TooManyBonds
        );
        // Endpoint past the end, self-bond, duplicate pair, bad limit.
        let bad_bonds = [
            vec![bond(0, 9, 1.0)],
            vec![bond(1, 1, 1.0)],
            vec![bond(0, 1, 1.0), bond(1, 0, 2.0)],
            vec![SpreadBond {
                a: 0,
                b: 1,
                threshold_q16: MAX_STRAIN_Q16 + 1,
            }],
        ];
        for bad in bad_bonds {
            assert!(SpreadGraph::new(5, bad, anchors.clone()).is_err());
        }
        assert_eq!(
            SpreadGraph::new(5, bonds.clone(), vec![7]).unwrap_err(),
            SpreadError::BadChunk(7)
        );
    }

    #[test]
    fn bad_spread_calls_rejected() {
        let graph = branched_graph();
        assert_eq!(
            spread_cracks(&graph, 99, q(3.0), 1).unwrap_err(),
            SpreadError::BadBond(99)
        );
        assert_eq!(
            spread_cracks(&graph, 0, -1, 1).unwrap_err(),
            SpreadError::BadStrain
        );
        assert_eq!(graph.threshold_of(99), None);
        assert!(!graph.is_anchored(99));
        assert_eq!(graph.chunk_count(), 5);
        assert_eq!(graph.bond_count(), 5);
        assert_eq!(graph.anchors(), vec![0]);
    }

    #[test]
    fn cluster_bridge_posts_support_and_rejects_bad() {
        use crate::bake::BREAKING_THRESHOLD;
        use crate::bonded::{Bond, BondedDesc, Chunk3};
        fn chunk(x: f32) -> Chunk3 {
            Chunk3 {
                x,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }
        }
        let desc = BondedDesc {
            chunks: vec![chunk(0.0), chunk(1.0), chunk(2.0)],
            bonds: vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 2,
                    breaking_strain: 1.0,
                },
            ],
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        let (supported, digest) = cluster_support_and_digest(&desc).expect("chain bridges");
        assert_eq!(supported, vec![0, 1, 2]);
        let (_, again) = cluster_support_and_digest(&desc).expect("bridges twice");
        assert_eq!(digest, again);
        let looped = BondedDesc {
            bonds: vec![Bond {
                a: 1,
                b: 1,
                breaking_strain: 1.0,
            }],
            ..desc.clone()
        };
        assert!(cluster_support_and_digest(&looped).is_err());
        let dangling = BondedDesc {
            anchors: vec![40],
            ..desc.clone()
        };
        assert!(cluster_support_and_digest(&dangling).is_err());
        // DX16: the cascade bridge is the same graph's propagation query: an
        // impact 4 on the middle shard with toughness 2 swaps shards 1 and 2
        // and leaves only the anchored shard standing.
        let (cascade, standing, digest) =
            cluster_cascade_and_digest(&desc, &[], 1, 4, 2).expect("cascade bridges");
        assert_eq!(cascade.broken_chunks, vec![1, 2]);
        assert_eq!(cascade.snapped_bonds, vec![0, 1]);
        assert_eq!(standing, vec![0]);
        let (_, again, repeat) = cluster_cascade_and_digest(&desc, &[], 1, 4, 2).expect("twice");
        assert_eq!(again, standing);
        assert_eq!(digest, repeat);
        // A bond that snapped before the hit stops the wave: shard 1 swaps
        // alone and shard 2 keeps both its own bonds intact.
        let (gapped, _, _) =
            cluster_cascade_and_digest(&desc, &[1], 1, 4, 2).expect("replays the gap");
        assert_eq!(gapped.broken_chunks, vec![1]);
        assert_eq!(gapped.snapped_bonds, vec![0], "only the new snap reports");
        assert!(cluster_cascade_and_digest(&desc, &[], 40, 4, 2).is_err());
        assert!(cluster_cascade_and_digest(&desc, &[9], 1, 4, 2).is_err());
    }

    /// Deterministic Fisher-Yates with an LCG; std-only, no RNG crate.
    fn lcg_shuffle<T>(v: &mut [T], mut s: u64) {
        for i in (1..v.len()).rev() {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (s >> 33) as usize % (i + 1);
            v.swap(i, j);
        }
    }
}
