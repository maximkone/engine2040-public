//! Kernel: Q16 fixed-point math (`engine_core::fixed`).
//!
//! Bounds are named on every harness. The inputs stay inside the raw
//! `i64` Q47.16 domain; products and quotients are checked in `i128`.

use engine_core::fixed::Fixed;

/// Kernel: `Fixed::checked_mul` (`round_q16` ties away from zero).
/// Bound: every pair of raw `i64` Q47.16 values.
#[kani::proof]
pub fn q16_checked_mul_matches_i128() {
    const HALF: i128 = 1 << 15;
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    let product = i128::from(a) * i128::from(b);
    let rounded = if product >= 0 {
        (product + HALF) >> 16
    } else {
        -((HALF - product) >> 16)
    };
    let got = Fixed::from_bits(a).checked_mul(Fixed::from_bits(b));
    let want = i64::try_from(rounded).ok();
    assert_eq!(got.map(Fixed::to_bits), want);
}

/// Kernel: `Fixed::checked_div` (truncation toward zero, no panic).
/// Bound: raw dividend and nonzero divisor in `+/-2^32`.
#[kani::proof]
pub fn q16_checked_div_is_total_and_truncates() {
    const LIMIT: i64 = 1 << 32;
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    kani::assume((-LIMIT..=LIMIT).contains(&a));
    kani::assume((-LIMIT..=LIMIT).contains(&b));
    kani::assume(b != 0);
    let got = Fixed::from_bits(a).checked_div(Fixed::from_bits(b));
    let want = i64::try_from((i128::from(a) << 16) / i128::from(b)).ok();
    assert_eq!(got.map(Fixed::to_bits), want);
}

/// Kernel: `Fixed::sqrt` over `isqrt` (floor square root, integer only).
/// Bound: positive raw Q47.16 values up to `2^20` (16.0); negative and
/// zero give zero.
#[kani::proof]
#[kani::unwind(22)]
pub fn q16_sqrt_is_floor_square_root() {
    let raw: i64 = kani::any();
    kani::assume(raw > 0 && raw <= 1_i64 << 20);
    let root = Fixed::from_bits(raw).sqrt().to_bits();
    assert!(root >= 0);
    let n = u128::from(raw.unsigned_abs()) * 65_536;
    let r = u128::from(root.unsigned_abs());
    assert!(r * r <= n);
    assert!((r + 1) * (r + 1) > n);
}

/// Kernel: `Fixed::lerp` endpoint law.
/// Bound: every pair of raw `i64` endpoints and every raw `i64` factor,
/// with the factor clamped to `0.0..=1.0`.
#[kani::proof]
pub fn q16_lerp_returns_exact_endpoints() {
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    let t: i64 = kani::any();
    let got = Fixed::from_bits(a).lerp(Fixed::from_bits(b), Fixed::from_bits(t));
    if t <= 0 {
        assert_eq!(got.to_bits(), a);
    }
    if t >= Fixed::ONE.to_bits() {
        assert_eq!(got.to_bits(), b);
    }
}

/// Kernel: `Fixed::reference_cmp_scalar` (i128 ordering oracle).
/// Bound: every raw `i64` value and every `i64` integer scalar.
#[kani::proof]
pub fn q16_reference_cmp_scalar_orders_like_i128() {
    let raw: i64 = kani::any();
    let scalar: i64 = kani::any();
    let want = i128::from(raw).cmp(&(i128::from(scalar) * i128::from(Fixed::SCALE)));
    assert_eq!(Fixed::from_bits(raw).reference_cmp_scalar(scalar), want);
}
