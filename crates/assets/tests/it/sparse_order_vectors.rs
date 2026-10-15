// Donor: KhronosGroup/glTF-Validator Apache-2.0,
// https://github.com/KhronosGroup/glTF-Validator (validate_accessors
// sparse loop idea, fresh Rust). Attribution comment: sparse indices must
// be strictly increasing; re-expressed fresh in std-only Rust here.
// No donor code copied.

//! glTF sparse order reject vectors: non-increasing indices fail.
//!
//! Owner: `engine-assets` gate (`crates/assets/src/gltf.rs` shape family).
//! Consumer: the glTF cook gate keeps out-of-order sparse substitutions
//! before mesh import. Units: index values in, typed verdicts out.
//! Deterministic: fixed literals only, no RNG, no clock, no IO.
//! Errors: non-increasing pairs map to `SparseOrderError::NonIncreasing`.

use std::fmt;

/// Why sparse order validation failed; every malformed vector maps to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SparseOrderError {
    /// Indices are not strictly increasing; names the first bad position.
    NonIncreasing {
        /// Position of the current index in the slice.
        position: usize,
        /// Previous index value.
        prev: u32,
        /// Current index value at `position`.
        current: u32,
    },
}

impl fmt::Display for SparseOrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonIncreasing {
                position,
                prev,
                current,
            } => write!(
                f,
                "glTF sparse indices not increasing at {position}: {prev} then {current}"
            ),
        }
    }
}

impl std::error::Error for SparseOrderError {}

/// Reject sparse substitution indices that are not strictly increasing.
///
/// Empty and single-element slices pass. The first pair with
/// `current <= prev` maps to [`SparseOrderError::NonIncreasing`].
/// Deterministic on the inputs.
fn check_sparse_order(indices: &[u32]) -> Result<(), SparseOrderError> {
    for (offset, pair) in indices.windows(2).enumerate() {
        let prev = pair[0];
        let current = pair[1];
        if current <= prev {
            return Err(SparseOrderError::NonIncreasing {
                position: offset + 1,
                prev,
                current,
            });
        }
    }
    Ok(())
}

#[test]
fn sparse_decreasing_rejects() {
    // 3 then 1 steps backwards, so the vector must fail.
    assert_eq!(
        check_sparse_order(&[3, 1, 2]),
        Err(SparseOrderError::NonIncreasing {
            position: 1,
            prev: 3,
            current: 1
        })
    );
}

#[test]
fn sparse_duplicate_rejects() {
    // Equal neighbours are not increasing, so the vector must fail.
    assert_eq!(
        check_sparse_order(&[2, 2, 1]),
        Err(SparseOrderError::NonIncreasing {
            position: 1,
            prev: 2,
            current: 2
        })
    );
}

#[test]
fn sparse_increasing_accepts() {
    // Both ordered fixtures must pass.
    assert_eq!(check_sparse_order(&[0, 1, 2]), Ok(()));
    assert_eq!(check_sparse_order(&[0, 2, 3]), Ok(()));
}

#[test]
fn sparse_edge_cases_accept() {
    // Empty, single, and wide gaps still pass.
    assert_eq!(check_sparse_order(&[]), Ok(()));
    assert_eq!(check_sparse_order(&[7]), Ok(()));
    assert_eq!(check_sparse_order(&[0, 100, 4294967295]), Ok(()));
}

#[test]
fn sparse_order_is_deterministic_x2() {
    // Same inputs give the same verdict twice.
    for indices in [
        &[3, 1, 2][..],
        &[2, 2, 1][..],
        &[0, 1, 2][..],
        &[0, 2, 3][..],
    ] {
        assert_eq!(check_sparse_order(indices), check_sparse_order(indices));
    }
    let text = format!(
        "{}",
        SparseOrderError::NonIncreasing {
            position: 1,
            prev: 3,
            current: 1,
        }
    );
    assert!(!text.is_empty());
    println!("sparse_order_vectors: passed, reject class = NonIncreasing");
}
