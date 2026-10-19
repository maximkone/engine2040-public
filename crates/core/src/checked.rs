//! checked: named integer conversions for tick paths. Owner: engine-core.
//! Units: counts and raw words (integers only, never floats).
//! Determinism: pure integer math; same input gives same output everywhere.
//! Errors: none (saturating forms clamp, never wrap or panic; the wrapping
//! form says so in its name).
//! Budget: O(1) per call, no allocation.

/// Saturating `i64` to `i32`: clamps to `MIN`/`MAX` by sign, never wraps.
pub fn saturating_i64_to_i32(v: i64) -> i32 {
    match i32::try_from(v) {
        Ok(w) => w,
        Err(_) => {
            if v < 0 {
                i32::MIN
            } else {
                i32::MAX
            }
        }
    }
}

/// Saturating `usize` to `u32`: clamps at `u32::MAX`, never wraps.
pub fn saturating_usize_to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Saturating `u32` to `u16`: clamps at `u16::MAX`, never truncates. The bytes
/// read keeps the conversion const and honest about the bound.
#[must_use]
pub const fn saturating_u32_to_u16(v: u32) -> u16 {
    if v > 65_535 {
        u16::MAX
    } else {
        let [lo, hi, ..] = v.to_le_bytes();
        u16::from_le_bytes([lo, hi])
    }
}

/// Saturating `u32` to `i32`: clamps at `i32::MAX`, never wraps.
#[must_use]
pub const fn saturating_u32_to_i32(v: u32) -> i32 {
    if v > 2_147_483_647 {
        i32::MAX
    } else {
        i32::from_le_bytes(v.to_le_bytes())
    }
}

/// Low 64 bits of an `i128` (two's complement wrap). The one named truncation
/// for fixed-point products that wrap by contract; the checked forms use
/// `i64::try_from` and refuse instead.
#[must_use]
#[inline]
pub const fn wrap_i128_to_i64(v: i128) -> i64 {
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = v.to_le_bytes();
    i64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7])
}

/// `u64` to `f64`, rounded to nearest even once: the same bits as `as f64`.
/// The high word times 2^32 is exact, so the sum is the only rounding.
#[must_use]
pub fn f64_from_u64(v: u64) -> f64 {
    let hi = u32::try_from(v >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(v & u64::from(u32::MAX)).unwrap_or(u32::MAX);
    f64::from(hi) * 4_294_967_296.0 + f64::from(lo)
}

/// `f64` to `f32`, rounded to nearest even (overflow gives infinity, NaN stays
/// NaN). The one float narrowing; presentation and cook-time data only.
#[must_use]
#[expect(clippy::cast_possible_truncation, reason = "narrowing")]
pub fn f32_from_f64(v: f64) -> f32 {
    v as f32
}

/// `u64` to `f32` through `f64`: the same bits as `as f32` up to 2^53 (a
/// tick rate is far below), one rounding more past it.
#[must_use]
pub fn f32_from_u64(v: u64) -> f32 {
    f32_from_f64(f64_from_u64(v))
}

/// `f64` to `u64` by truncation toward zero, saturating: NaN and negatives give
/// 0, anything past `u64::MAX` (and infinity) gives `u64::MAX`. Same results as
/// `as u64`, read from the bits so no cast is needed.
#[must_use]
pub fn saturating_f64_to_u64(v: f64) -> u64 {
    match crate::time::f64_to_u64_checked(v.to_bits()) {
        Ok(n) => n,
        Err(crate::time::TimeError::Overflow) => u64::MAX,
        Err(crate::time::TimeError::NonFiniteDt) if v.is_infinite() && v > 0.0 => u64::MAX,
        Err(_) => 0,
    }
}

/// Narrowing `u64` to `u32` for constant tables, checked while the constant is
/// built: a value past `u32` fails the build instead of wrapping the way a bare
/// `as u32` at a call site would. Bytes are read out loud because a raw cast
/// would hide the bound; widening back is a plain `u64::from`.
///
/// # Panics
///
/// Fails the enclosing constant's evaluation when `v` does not fit `u32`.
#[must_use]
pub const fn u32_from_u64(v: u64) -> u32 {
    let [hi0, hi1, hi2, hi3, b0, b1, b2, b3] = v.to_be_bytes();
    assert!(
        (hi0 | hi1 | hi2 | hi3) == 0,
        "u32_from_u64: value does not fit u32"
    );
    u32::from_be_bytes([b0, b1, b2, b3])
}

/// Parse `text` as an unsigned decimal integer: ASCII digits only, no sign,
/// no dot, no whitespace. A value past `u32::MAX` refuses instead of wrapping.
///
/// The one owner for the whole-number fields of the rows files every data
/// reader walks (`engine-combat` tables, `engine-arena` zones), so a second
/// hand-rolled digit check cannot drift away from this one.
#[must_use]
pub fn decimal_u32(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut value: u32 = 0;
    for digit in text.bytes() {
        // `is_ascii_digit` above keeps the subtraction in `0..=9`.
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(digit - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;

    #[test]
    fn narrowing_round_trips_small_values() {
        assert_eq!(saturating_i64_to_i32(-3), -3);
        assert_eq!(saturating_i64_to_i32(0), 0);
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MIN)), i32::MIN);
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MAX)), i32::MAX);
        assert_eq!(saturating_usize_to_u32(0), 0);
        assert_eq!(saturating_usize_to_u32(7), 7);
        assert_eq!(saturating_usize_to_u32(1024), 1024);
    }

    #[test]
    fn saturating_clamps_where_wrapping_would_wrap() {
        // One past the top wraps to zero or `MIN` under truncation; the
        // saturating forms hold the bound instead.
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MAX) + 1), i32::MAX);
        assert_eq!(saturating_i64_to_i32(i64::MAX), i32::MAX);
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MIN) - 1), i32::MIN);
        assert_eq!(saturating_i64_to_i32(i64::MIN), i32::MIN);
        assert_eq!(saturating_usize_to_u32(usize::MAX), u32::MAX);
    }

    #[test]
    fn conversions_never_allocate() {
        let ((), heap) = measure(|| {
            for v in [0i64, -3, i64::from(i32::MIN), i64::MAX] {
                std::hint::black_box(saturating_i64_to_i32(v));
            }
            for v in [0usize, 7, usize::MAX] {
                std::hint::black_box(saturating_usize_to_u32(v));
            }
        });
        assert_eq!(heap.calls, 0, "checked conversions must not allocate");
    }

    #[test]
    fn saturating_u32_narrowing_holds_the_bounds() {
        assert_eq!(saturating_u32_to_u16(0), 0);
        assert_eq!(saturating_u32_to_u16(1_000), 1_000);
        assert_eq!(saturating_u32_to_u16(65_535), u16::MAX);
        assert_eq!(saturating_u32_to_u16(65_536), u16::MAX);
        assert_eq!(saturating_u32_to_u16(u32::MAX), u16::MAX);
        assert_eq!(saturating_u32_to_i32(0), 0);
        assert_eq!(saturating_u32_to_i32(2_147_483_647), i32::MAX);
        assert_eq!(saturating_u32_to_i32(2_147_483_648), i32::MAX);
        assert_eq!(saturating_u32_to_i32(u32::MAX), i32::MAX);
    }

    #[test]
    fn u32_narrowing_is_checked_and_holds_the_tick_rate() {
        // The tick rate and the stillness window are `u32` constants derived
        // from the one `u64` rate. The narrowing is a named checked owner, so
        // a value past `u32` fails the build instead of wrapping silently the
        // way a bare `as u32` at each call site would.
        assert_eq!(u32_from_u64(0), 0);
        assert_eq!(u32_from_u64(60), 60);
        assert_eq!(u32_from_u64(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(u64::from(crate::TICK_HZ_U32), crate::TICK_HZ, "one rate");
        assert_eq!(crate::STUCK_WINDOW_TICKS, 3 * crate::TICK_HZ_U32);
        assert_eq!(crate::STUCK_WINDOW_TICKS, 180);
    }

    #[test]
    fn decimal_u32_takes_digits_only_and_refuses_overflow() {
        // Contract of the rows-file owner: plain ASCII digits, no sign and no
        // dot, `u32::MAX` as the top, `None` for everything else.
        assert_eq!(decimal_u32("0"), Some(0));
        assert_eq!(decimal_u32("2040"), Some(2040));
        assert_eq!(decimal_u32("007"), Some(7), "leading zeros are digits");
        assert_eq!(decimal_u32("4294967295"), Some(u32::MAX));
        assert_eq!(decimal_u32("4294967296"), None, "one past the top refuses");
        assert_eq!(decimal_u32(""), None, "empty refuses");
        assert_eq!(decimal_u32("+1"), None, "sign refuses");
        assert_eq!(decimal_u32("-1"), None, "sign refuses");
        assert_eq!(decimal_u32("1.5"), None, "dot refuses");
        assert_eq!(decimal_u32(" 1"), None, "space refuses");
        assert_eq!(decimal_u32("1_0"), None, "separator refuses");
        assert_eq!(decimal_u32("\u{661}\u{662}"), None, "non-ASCII refuses");
    }

    #[test]
    fn wrap_i128_keeps_the_low_64_bits() {
        assert_eq!(wrap_i128_to_i64(0), 0);
        assert_eq!(wrap_i128_to_i64(-1), -1);
        assert_eq!(wrap_i128_to_i64(i128::from(i64::MAX)), i64::MAX);
        assert_eq!(wrap_i128_to_i64(i128::from(i64::MIN)), i64::MIN);
        assert_eq!(
            wrap_i128_to_i64(i128::from(i64::MAX) + 1),
            i64::MIN,
            "wraps"
        );
        assert_eq!(wrap_i128_to_i64((7_i128 << 64) | 5), 5, "high word dropped");
        assert_eq!(wrap_i128_to_i64(i128::MIN), 0);
    }

    /// Edges and a spread of mixed values, shared by the float conversion tests.
    fn spread() -> impl Iterator<Item = u64> {
        let edges = [0, 1, 2, 3, 0xFF_FFFF, 0x100_0000, 0x100_0001, 0x100_0003];
        let rng = crate::CounterRng::new(2040);
        edges
            .into_iter()
            .chain([
                u64::from(u32::MAX),
                u64::MAX,
                u64::MAX - 1,
                1 << 53,
                (1 << 53) + 1,
            ])
            .chain((0..4096).map(move |i| rng.sample(i) >> (i % 64)))
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "the test pins the helpers to the bits of the casts they replace"
    )]
    fn float_helpers_match_the_cast_bits() {
        for v in spread() {
            assert_eq!(f64_from_u64(v).to_bits(), (v as f64).to_bits(), "u64 {v}");
            if v < 1 << 53 {
                assert_eq!(
                    f32_from_u64(v).to_bits(),
                    (v as f32).to_bits(),
                    "u64 to f32 {v}"
                );
            }
        }
        assert!(f32_from_f64(f64::NAN).is_nan());
        assert_eq!(f32_from_f64(1e300), f32::INFINITY);
    }

    #[test]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the test pins the helper to the results of the cast it replaces"
    )]
    fn saturating_f64_to_u64_matches_the_cast() {
        let edges = [
            0.0,
            -0.0,
            0.5,
            0.999_999_9,
            1.0,
            1.5,
            -1.0,
            4_294_967_296.5,
            9.007_199_254_740_993e15,
            1.844_674_407_370_955e19,
            1.844_674_407_370_955_2e19,
            1e20,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE,
        ];
        for d in edges.into_iter().chain(spread().map(f64::from_bits)) {
            assert_eq!(saturating_f64_to_u64(d), d as u64, "f64 {d:?}");
        }
    }

    #[test]
    fn decimal_u32_never_allocates() {
        let ((), heap) = measure(|| {
            for text in ["0", "2040", "4294967296", "12.0", ""] {
                std::hint::black_box(decimal_u32(text));
            }
        });
        assert_eq!(heap.calls, 0, "the rows parse must not allocate");
    }
}
