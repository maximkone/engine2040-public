// Attribution: scalar-oracle plus reference-compare plus seed-corpus shape
// adapted from alloy-rs/ruint (MIT) `src/cmp.rs` (`impl_for_primitives!`,
// `reference_cmp`, `check_cmp`, `test_cmp`) and its `proptest-regressions/`
// seed-corpus dir; reimplemented here fresh over engine-core Q47.16 raw bits
// in `i128`, std-only over `Rng` fork streams, no donor bytes vendored.
// PORT(alloy-rs/ruint@c764018b80dce8f0b5a0262cb97846e633acee31:src/cmp.rs)
// PORT(alloy-rs/ruint@c764018b80dce8f0b5a0262cb97846e633acee31:proptest-regressions/)

//! Fixed vs scalar oracle vectors plus a deterministic seed corpus.
//! Owner: engine-core (test-only vectors). Units: Fixed raw bits are Q47.16
//! `i64`, scalars are integers, seeds are `u64`. Determinism: `Rng` fork
//! streams only, no wall clock; the same seeds replay x2 identical.
//! Errors: none; failures panic via `assert!` with the mismatching values.
use engine_core::{fixed::Fixed, Rng};
use std::cmp::Ordering;

/// Fixed seeds shared by both corpus tests; fixed so runs replay x2.
const SEEDS: [u64; 8] = [2040, 2041, 2042, 2043, 2044, 7, 99, 1234];
/// Runner case count, ported from the donor proptest default (256).
const RUNNER_CASES: u32 = 256;

/// One deterministic `Fixed` from the stream; covers the full raw range.
fn draw_fixed(rng: &mut Rng) -> Fixed {
    Fixed::from_bits(i64::from_le_bytes(rng.next_u64().to_le_bytes()))
}

/// One deterministic scalar from the stream; covers the full `i64` range.
fn draw_scalar(rng: &mut Rng) -> i64 {
    i64::from_le_bytes(rng.next_u64().to_le_bytes())
}

/// Runner case `index` in `0..RUNNER_CASES`: the seed is the index, so every
/// run draws the same pair from a forked stream that skips the low-bit
/// aliasing of consecutive `Rng::new` seeds.
fn runner_pair(index: u32) -> (Fixed, Fixed) {
    let mut rng = Rng::new(2040).fork(u64::from(index));
    (draw_fixed(&mut rng), draw_fixed(&mut rng))
}

/// Runner scalar case: one `Fixed` plus one `i64` scalar from the same stream.
fn runner_scalar(index: u32) -> (Fixed, i64) {
    let mut rng = Rng::new(2040).fork(u64::from(index).wrapping_add(7919));
    (draw_fixed(&mut rng), draw_scalar(&mut rng))
}

/// Donor `check_cmp` shape: derived compare and all four operators agree
/// with the reference oracle on one pair.
fn check_pair(a: Fixed, b: Fixed) {
    let oracle = Fixed::reference_cmp(a, b);
    assert_eq!(a.cmp(&b), oracle, "cmp differs for {a:?} vs {b:?}");
    assert_eq!(a < b, oracle.is_lt(), "lt differs for {a:?} vs {b:?}");
    assert_eq!(a > b, oracle.is_gt(), "gt differs for {a:?} vs {b:?}");
    assert_eq!(a <= b, !oracle.is_gt(), "le differs for {a:?} vs {b:?}");
    assert_eq!(a >= b, !oracle.is_lt(), "ge differs for {a:?} vs {b:?}");
}

/// Scalar oracle check: `PartialEq`/`PartialOrd` vs the `i128` reference.
fn check_scalar(fixed: Fixed, scalar: i64) {
    let oracle = fixed.reference_cmp_scalar(scalar);
    assert_eq!(
        fixed.partial_cmp(&scalar),
        Some(oracle),
        "scalar cmp differs for {fixed:?} vs {scalar}"
    );
    assert_eq!(
        (fixed == scalar),
        (oracle == Ordering::Equal),
        "scalar eq differs for {fixed:?} vs {scalar}"
    );
    assert_eq!(
        (fixed < scalar),
        (oracle == Ordering::Less),
        "scalar lt differs for {fixed:?} vs {scalar}"
    );
    assert_eq!(
        (fixed > scalar),
        (oracle == Ordering::Greater),
        "scalar gt differs for {fixed:?} vs {scalar}"
    );
}

#[test]
fn oracle_fixed_vectors_match_reference() {
    let one_half = Fixed::from_bits(98304);
    let neg_half = Fixed::from_bits(-98304);
    let vectors = [
        Fixed::ZERO,
        Fixed::ONE,
        Fixed::NEG_ONE,
        Fixed::HALF,
        one_half,
        neg_half,
        Fixed::from_int(2),
        Fixed::from_int(-2),
        Fixed::PI,
        Fixed::MIN,
        Fixed::MAX,
    ];
    for (i, &a) in vectors.iter().enumerate() {
        for &b in &vectors[i..] {
            check_pair(a, b);
            check_pair(b, a);
        }
    }
    assert_eq!(
        Fixed::reference_cmp(Fixed::ZERO, Fixed::ONE),
        Ordering::Less
    );
    assert_eq!(
        Fixed::reference_cmp(Fixed::ONE, Fixed::ZERO),
        Ordering::Greater
    );
    assert_eq!(
        Fixed::reference_cmp(Fixed::ZERO, Fixed::ZERO),
        Ordering::Equal
    );
    assert_eq!(Fixed::reference_cmp(Fixed::MIN, Fixed::MAX), Ordering::Less);
    assert_eq!(
        Fixed::reference_cmp(Fixed::MAX, Fixed::MAX),
        Ordering::Equal
    );
    assert_eq!(one_half.cmp(&Fixed::ONE), Ordering::Greater);
    assert_eq!(neg_half.cmp(&Fixed::NEG_ONE), Ordering::Less);
}

#[test]
fn oracle_scalar_edges_match_reference() {
    let one_half = Fixed::from_bits(98304);
    assert_eq!(Fixed::ONE.reference_cmp_scalar(1), Ordering::Equal);
    assert_eq!(Fixed::HALF.reference_cmp_scalar(1), Ordering::Less);
    assert_eq!(Fixed::HALF.reference_cmp_scalar(0), Ordering::Greater);
    assert_eq!(one_half.reference_cmp_scalar(1), Ordering::Greater);
    assert_eq!(one_half.reference_cmp_scalar(2), Ordering::Less);
    assert_eq!(Fixed::NEG_ONE.reference_cmp_scalar(-1), Ordering::Equal);
    assert_eq!(Fixed::ZERO.reference_cmp_scalar(0), Ordering::Equal);
    // Scalar-edge: scaling `i64::MAX/MIN` would overflow `i64`, yet the
    // `i128` oracle still orders without wrapping.
    assert_eq!(Fixed::MAX.reference_cmp_scalar(i64::MAX), Ordering::Less);
    assert_eq!(Fixed::MIN.reference_cmp_scalar(i64::MIN), Ordering::Greater);
    assert!(Fixed::ONE == 1i64);
    assert!(Fixed::ONE != 2i64);
    assert!(one_half > 1i64);
    assert!(one_half < 2i64);
    assert!(Fixed::ZERO == 0i64);
    assert!(Fixed::NEG_ONE == -1i64);
    assert!(Fixed::MAX < i64::MAX);
    assert!(Fixed::MIN > i64::MIN);
    // Every macro-generated scalar type agrees with the `i64` oracle path.
    assert!(Fixed::ONE == 1u8);
    assert!(Fixed::ONE == 1u16);
    assert!(Fixed::ONE == 1u32);
    assert!(Fixed::ONE == 1u64);
    assert!(Fixed::ONE == 1u128);
    assert!(Fixed::ONE == 1usize);
    assert!(Fixed::ONE == 1i8);
    assert!(Fixed::ONE == 1i16);
    assert!(Fixed::ONE == 1i32);
    assert!(Fixed::ONE == 1i128);
    assert!(Fixed::ONE == 1isize);
    assert!(Fixed::HALF < 1u8);
    assert!(Fixed::NEG_ONE < 0u8);
    assert!(Fixed::ZERO == 0u128);
    // Huge positives never equal and always order less, without wrapping.
    assert!(Fixed::MAX != u128::MAX);
    assert_eq!(Fixed::MAX.partial_cmp(&u128::MAX), Some(Ordering::Less));
    assert!(Fixed::MAX < u128::MAX);
    for scalar in [0i64, 1, -1, 2, -2, i64::MAX, i64::MIN, 1234, -1234] {
        check_scalar(Fixed::ONE, scalar);
        check_scalar(Fixed::HALF, scalar);
        check_scalar(one_half, scalar);
        check_scalar(Fixed::MIN, scalar);
        check_scalar(Fixed::MAX, scalar);
    }
}

#[test]
fn seed_corpus_fixed_cmp_matches_reference() {
    let mut first: Vec<Ordering> = Vec::new();
    for &seed in &SEEDS {
        let mut rng = Rng::new(seed).fork_named("fixed-cmp");
        let (a, b) = (draw_fixed(&mut rng), draw_fixed(&mut rng));
        check_pair(a, b);
        first.push(a.cmp(&b));
    }
    for index in 0..RUNNER_CASES {
        let (a, b) = runner_pair(index);
        check_pair(a, b);
        first.push(a.cmp(&b));
    }
    // Replay x2: the same seeds draw the same pairs and the same orderings.
    let mut second: Vec<Ordering> = Vec::new();
    for &seed in &SEEDS {
        let mut rng = Rng::new(seed).fork_named("fixed-cmp");
        let (a, b) = (draw_fixed(&mut rng), draw_fixed(&mut rng));
        check_pair(a, b);
        second.push(a.cmp(&b));
    }
    for index in 0..RUNNER_CASES {
        let (a, b) = runner_pair(index);
        check_pair(a, b);
        second.push(a.cmp(&b));
    }
    assert_eq!(first, second, "seed corpus must replay x2 identical");
    assert!(
        first.iter().any(|&o| o == Ordering::Less) && first.iter().any(|&o| o == Ordering::Greater),
        "corpus must cover both sides of the oracle"
    );
}

#[test]
fn seed_corpus_scalar_matches_reference() {
    let mut first: Vec<Ordering> = Vec::new();
    for &seed in &SEEDS {
        let mut rng = Rng::new(seed).fork_named("fixed-scalar");
        let (fixed, scalar) = (draw_fixed(&mut rng), draw_scalar(&mut rng));
        check_scalar(fixed, scalar);
        first.push(fixed.reference_cmp_scalar(scalar));
    }
    for index in 0..RUNNER_CASES {
        let (fixed, scalar) = runner_scalar(index);
        check_scalar(fixed, scalar);
        first.push(fixed.reference_cmp_scalar(scalar));
    }
    // Deterministic equal cover: random full-range draws almost never hit
    // an exact integer, so pin small exact pairs; both passes add the same
    // order so the replay stays x2 identical.
    for scalar in [0i64, 1, -1, 2, -2] {
        let fixed = Fixed::from_int(scalar);
        check_scalar(fixed, scalar);
        first.push(fixed.reference_cmp_scalar(scalar));
    }
    let mut second: Vec<Ordering> = Vec::new();
    for &seed in &SEEDS {
        let mut rng = Rng::new(seed).fork_named("fixed-scalar");
        let (fixed, scalar) = (draw_fixed(&mut rng), draw_scalar(&mut rng));
        check_scalar(fixed, scalar);
        second.push(fixed.reference_cmp_scalar(scalar));
    }
    for index in 0..RUNNER_CASES {
        let (fixed, scalar) = runner_scalar(index);
        check_scalar(fixed, scalar);
        second.push(fixed.reference_cmp_scalar(scalar));
    }
    for scalar in [0i64, 1, -1, 2, -2] {
        let fixed = Fixed::from_int(scalar);
        check_scalar(fixed, scalar);
        second.push(fixed.reference_cmp_scalar(scalar));
    }
    assert_eq!(first, second, "scalar corpus must replay x2 identical");
    assert!(
        first.iter().any(|&o| o == Ordering::Less)
            && first.iter().any(|&o| o == Ordering::Greater)
            && first.iter().any(|&o| o == Ordering::Equal),
        "scalar corpus must cover less, greater, and equal"
    );
}
