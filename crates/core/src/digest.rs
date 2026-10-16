//! digest: shared fold for canonical words beside the full rehash.
//! Owner: engine-core (std-only). Units: hashes (u64). LE words, index order, no alloc.

use crate::hash::{Fnv64, FNV64_PRIME};

/// Experiment flag beside the full rehash. Off keeps the default path. Caller: `crates/ecs/src/snapshot.rs`.
pub const RC9_COMBINED_FOLD_ENABLED: bool = false;

/// Twin FNV-1a fold, one index-ordered pass over two streams. Caller: `crates/ecs/src/snapshot.rs`.
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

/// Hash one table in index order, O(changed) via cached roots. Caller: `crates/ecs/src/snapshot.rs`.
#[must_use]
pub fn fold_component_words(words: &[u32]) -> u64 {
    let mut h = Fnv64::new();
    for &w in words {
        h.write_u32(w);
    }
    h.finish()
}

/// Fold hashes into one archetype root in index order. Caller: `crates/ecs/src/snapshot.rs`.
#[must_use]
pub fn fold_archetype_root(component_hashes: &[u64]) -> u64 {
    let mut h = Fnv64::new();
    for &c in component_hashes {
        h.write_u64(c);
    }
    h.finish()
}

/// Domain tag for the tick-root fold: the word for "tickroot", so a tick root
/// never equals an archetype root over the same words.
const TICK_ROOT_DOMAIN: u64 = 0x7469_636b_726f_6f74;

/// Fold roots into one tick root in index order. Domain-tagged, never an alias
/// of [`fold_archetype_root`]. Caller: `crates/ecs/src/snapshot.rs`.
#[must_use]
pub fn fold_tick_root(archetype_roots: &[u64]) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(TICK_ROOT_DOMAIN);
    for &c in archetype_roots {
        h.write_u64(c);
    }
    h.finish()
}

/// First divergent archetype and component. Caller: `crates/ecs/src/snapshot.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstDivergence {
    pub archetype: usize,
    pub component: usize,
}

/// Name first divergence; comps cover roots. Caller: `crates/ecs/src/snapshot.rs`.
#[must_use]
pub fn find_first_divergence(
    roots_a: &[u64],
    roots_b: &[u64],
    comps_a: &[&[u64]],
    comps_b: &[&[u64]],
) -> Option<FirstDivergence> {
    if roots_a.len() != roots_b.len() {
        let archetype = roots_a.len().min(roots_b.len());
        return Some(FirstDivergence {
            archetype,
            component: 0,
        });
    }
    for (archetype, (&ra, &rb)) in roots_a.iter().zip(roots_b.iter()).enumerate() {
        if ra != rb {
            let ca = comps_a[archetype];
            let cb = comps_b[archetype];
            for (component, (&x, &y)) in ca.iter().zip(cb.iter()).enumerate() {
                if x != y {
                    return Some(FirstDivergence {
                        archetype,
                        component,
                    });
                }
            }
            return Some(FirstDivergence {
                archetype,
                component: ca.len().min(cb.len()),
            });
        }
    }
    None
}

#[cfg(test)]
mod digest_fold_tests {
    use super::*;
    use crate::hash::FNV64_OFFSET;
    use crate::Rng;

    fn seeded_comps(seed: u64) -> Vec<Vec<u64>> {
        let mut rng = Rng::new(seed);
        let mut comps = Vec::new();
        for _ in 0..8 {
            let mut row = Vec::new();
            for _ in 0..8 {
                row.push(rng.next_u64());
            }
            comps.push(row);
        }
        comps
    }

    #[test]
    fn digest_fold_parity_and_localization_x2() {
        assert_eq!(fold_component_words(&[]), FNV64_OFFSET);
        assert_ne!(fold_component_words(&[1, 2]), fold_component_words(&[2, 1]));
        assert_ne!(fold_archetype_root(&[1, 2]), fold_archetype_root(&[2, 1]));
        for seed in 2040..2060u64 {
            let comps = seeded_comps(seed);
            let roots: Vec<u64> = comps.iter().map(|c| fold_archetype_root(c)).collect();
            let full = fold_tick_root(&roots);
            assert_eq!(full, fold_tick_root(&roots));
            let ai = (seed % 8) as usize;
            let ci = ((seed >> 3) % 8) as usize;
            let mut moved_comps = comps.clone();
            moved_comps[ai][ci] = moved_comps[ai][ci].wrapping_add(1);
            let mr: Vec<u64> = moved_comps.iter().map(|c| fold_archetype_root(c)).collect();
            let moved = fold_tick_root(&mr);
            assert_ne!(moved, full);
            let mut cached = roots.clone();
            cached[ai] = fold_archetype_root(&moved_comps[ai]);
            assert_eq!(fold_tick_root(&cached), moved);
            let sa: Vec<&[u64]> = comps.iter().map(Vec::as_slice).collect();
            let sb: Vec<&[u64]> = moved_comps.iter().map(Vec::as_slice).collect();
            let want = FirstDivergence {
                archetype: ai,
                component: ci,
            };
            assert_eq!(find_first_divergence(&roots, &mr, &sa, &sb), Some(want));
            assert_eq!(find_first_divergence(&roots, &roots, &sa, &sa), None);
        }
    }

    #[test]
    fn tick_root_domain_separated_from_archetype_root() {
        for seed in 2040..2060u64 {
            let comps = seeded_comps(seed);
            let roots: Vec<u64> = comps.iter().map(|c| fold_archetype_root(c)).collect();
            assert_ne!(
                fold_tick_root(&roots),
                fold_archetype_root(&roots),
                "tick root must not alias the archetype fold (seed {seed})"
            );
        }
        assert_ne!(fold_tick_root(&[]), fold_archetype_root(&[]));
    }
}
