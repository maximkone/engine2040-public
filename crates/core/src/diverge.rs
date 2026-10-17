//! diverge: the one first-difference scan between two per-tick chains.
//! Owner: engine-core. Units: chain indices are fixed ticks (u32).
//! Determinism: pure compares, same input gives same output everywhere.
//! Errors: none (equal chains read as `None`).
//! Budget: O(n) compares, no allocation.

use crate::checked::saturating_usize_to_u32;

/// First index where `left` and `right` differ, or `None` when they agree.
/// A length mismatch counts as a difference at the shared prefix end.
pub fn first_divergence<T: PartialEq>(left: &[T], right: &[T]) -> Option<u32> {
    let index = left
        .iter()
        .zip(right)
        .position(|(a, b)| a != b)
        .or_else(|| (left.len() != right.len()).then(|| left.len().min(right.len())))?;
    Some(saturating_usize_to_u32(index))
}

#[cfg(test)]
mod tests {
    use super::first_divergence;

    #[test]
    fn equal_chains_have_no_divergence() {
        assert_eq!(first_divergence(&[1u64, 2, 3], &[1, 2, 3]), None);
        assert_eq!(first_divergence::<u64>(&[], &[]), None);
    }

    #[test]
    fn first_differing_index_is_named() {
        assert_eq!(first_divergence(&[1u64, 2, 3, 4], &[1, 2, 9, 9]), Some(2));
    }

    #[test]
    fn length_mismatch_diverges_at_the_shared_end() {
        assert_eq!(first_divergence(&[1u64, 2, 3], &[1, 2]), Some(2));
        assert_eq!(first_divergence(&[1u64], &[1, 2, 3]), Some(1));
    }
}
