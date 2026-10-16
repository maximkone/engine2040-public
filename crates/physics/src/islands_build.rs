//! islands_build: deterministic island topology plus hashes.
//!
//! Canonical union-find over contact pairs plus the order-independent space
//! hash. Same pairs in any order give the same islands and the same bytes.
//! Re-exported through [`crate::islands`] so existing callers keep one path.
#![forbid(unsafe_code)]

use engine_core::hash::{FNV64_PRIME, GOLDEN_GAMMA};
use engine_core::Fnv64;

/// Union-find root per body. Skips self/out-of-range pairs, then unions by
/// smaller root: no rank noise, fully deterministic.
///
/// No sort or dedup pass: union by minimum makes edge order and duplicates
/// irrelevant (each component root is its minimum index), so the output
/// equals the sorted-edge path without its buffer.
pub fn roots(n: usize, pairs: &[(usize, usize)]) -> Vec<usize> {
    let mut p: Vec<usize> = (0..n).collect();
    for &(a, b) in pairs {
        if a == b || a >= n || b >= n {
            continue;
        }
        let mut ra = a;
        while p[ra] != ra {
            p[ra] = p[p[ra]];
            ra = p[ra];
        }
        let mut rb = b;
        while p[rb] != rb {
            p[rb] = p[p[rb]];
            rb = p[rb];
        }
        if ra != rb {
            if ra < rb {
                p[rb] = ra;
            } else {
                p[ra] = rb;
            }
        }
    }
    for i in 0..n {
        let mut x = i;
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        p[i] = x;
    }
    p
}

/// Canonical islands: every body appears in exactly one island; isolated bodies
/// yield singleton islands. Sorted output, deterministic for any input order.
pub fn build_islands(n: usize, pairs: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let r = roots(n, pairs);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (r[i], i));
    let mut out: Vec<Vec<usize>> = Vec::new();
    for i in order {
        match out.last_mut() {
            Some(last) if r[last[0]] == r[i] => last.push(i),
            _ => out.push(vec![i]),
        }
    }
    out.sort_by_key(|v| v[0]);
    out
}

/// FNV-1a over one island's body hashes in member order (canonical islands
/// arrive ascending, so bytes are deterministic for any pair input order).
/// Body words fold byte-wise through the shared owner; the member count
/// folds as one whole word plus a prime step, exactly like before.
pub fn island_hash(body_hash: &[u64], members: &[usize]) -> u64 {
    let mut h = Fnv64::new();
    for &i in members {
        h.write_u64(body_hash.get(i).copied().unwrap_or(0));
    }
    let mut state = h.finish();
    state ^= count_as_u64(members.len());
    state = state.wrapping_mul(FNV64_PRIME);
    state
}

/// Count to hash word with saturating semantics: every reachable island
/// count fits, so this matches the old conversion exactly; an absurd count
/// clamps instead of wrapping, so a corrupt length can never alias a split.
pub(crate) fn count_as_u64(count: usize) -> u64 {
    u64::try_from(count).unwrap_or(u64::MAX)
}

/// Order-independent space hash: xor of per-island hashes plus island count.
/// Xor commutes, so island visit order never changes the result; the count
/// fold keeps a split/merge from aliasing. Mix constants come from the
/// shared hash owner, so the bytes match the previous revision exactly.
pub fn combined_hash(n: usize, pairs: &[(usize, usize)], body_hash: &[u64]) -> u64 {
    let islands = build_islands(n, pairs);
    let mut h: u64 = 0;
    for members in &islands {
        h ^= island_hash(body_hash, members).wrapping_mul(GOLDEN_GAMMA);
    }
    h ^= count_as_u64(islands.len()).wrapping_mul(FNV64_PRIME);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_slice_topology_hashes_stable_x2() {
        let fwd = build_islands(4, &[(0, 1), (2, 3), (1, 2)]);
        assert_eq!(fwd, vec![vec![0, 1, 2, 3]]);
        let rev = build_islands(4, &[(2, 1), (3, 2), (1, 0), (0, 1), (1, 1), (9, 9)]);
        assert_eq!(fwd, rev, "reversed ends, dupes, self/oob ignored");
        let body: Vec<u64> = vec![11, 22, 33, 44];
        let pairs = [(0usize, 1usize), (1, 2)];
        let h1 = combined_hash(4, &pairs, &body);
        assert_eq!(
            h1,
            combined_hash(4, &pairs, &body),
            "same input, same hash x2"
        );
        let moved: Vec<u64> = vec![11, 22, 34, 44];
        assert_ne!(
            h1,
            combined_hash(4, &pairs, &moved),
            "body change moves hash"
        );
    }

    /// Reference: normalize ends, sort, dedup, then union by minimum.
    fn sorted_roots(n: usize, pairs: &[(usize, usize)]) -> Vec<usize> {
        let mut p: Vec<usize> = (0..n).collect();
        let mut norm: Vec<(usize, usize)> = pairs
            .iter()
            .filter(|&&(a, b)| a != b && a < n && b < n)
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .collect();
        norm.sort_unstable();
        norm.dedup();
        for (a, b) in norm {
            let (mut ra, mut rb) = (a, b);
            while p[ra] != ra {
                p[ra] = p[p[ra]];
                ra = p[ra];
            }
            while p[rb] != rb {
                p[rb] = p[p[rb]];
                rb = p[rb];
            }
            if ra != rb {
                p[ra.max(rb)] = ra.min(rb);
            }
        }
        (0..n)
            .map(|i| {
                let mut x = i;
                while p[x] != x {
                    x = p[x];
                }
                x
            })
            .collect()
    }

    #[test]
    fn roots_are_component_minimums_whatever_the_edge_order() {
        assert_eq!(roots(4, &[(0, 1), (2, 3), (1, 2)]), vec![0, 0, 0, 0]);
        assert_eq!(roots(5, &[(0, 1), (1, 2)]), vec![0, 0, 0, 3, 4]);
        assert_eq!(roots(3, &[]), vec![0, 1, 2]);
        assert_eq!(roots(0, &[]), Vec::<usize>::new());
        assert_eq!(roots(2, &[(1, 1), (9, 9)]), vec![0, 1]);
        let fwd = roots(4, &[(0, 1), (2, 3), (1, 2)]);
        let rev = roots(4, &[(2, 1), (3, 2), (1, 0), (0, 1), (1, 1), (9, 9)]);
        assert_eq!(fwd, rev);
        let a = roots(6, &[(0, 1), (3, 4), (1, 2)]);
        assert_eq!(a, roots(6, &[(1, 2), (3, 4), (0, 1)]));
        assert_eq!(a, roots(6, &[(4, 3), (2, 1), (1, 0)]));
        assert_eq!(a, vec![0, 0, 0, 3, 3, 5]);
    }

    #[test]
    fn roots_match_the_sorted_reference_on_seeded_pair_sets() {
        let mut rng = engine_core::Rng::new(2040).fork_named("island-roots");
        for round in 0..200 {
            let n = usize::try_from(rng.next_range(0, 40)).unwrap_or(0);
            let count = usize::try_from(rng.next_range(0, 80)).unwrap_or(0);
            // Ends may run past `n`, repeat and point at themselves.
            let span = u64::try_from(n + 3).unwrap_or(3);
            let pairs: Vec<(usize, usize)> = (0..count)
                .map(|_| {
                    let end = |rng: &mut engine_core::Rng| {
                        usize::try_from(rng.next_range(0, span)).unwrap_or(0)
                    };
                    (end(&mut rng), end(&mut rng))
                })
                .collect();
            let want = sorted_roots(n, &pairs);
            assert_eq!(
                roots(n, &pairs),
                want,
                "round {round}: n={n} pairs={pairs:?}"
            );
            let reversed: Vec<(usize, usize)> = pairs.iter().rev().map(|&(a, b)| (b, a)).collect();
            assert_eq!(roots(n, &reversed), want, "round {round}: reversed");
            assert!(
                want.iter().enumerate().all(|(i, &r)| r <= i),
                "roots are minimums"
            );
        }
    }
}
