//! checked: named integer conversions for tick paths. Owner: engine-core.
//! Units: counts and raw words (integers only, never floats).
//! Determinism: pure integer math; same input gives same output everywhere.
//! Errors: none (saturating forms clamp, never wrap or panic).
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
    fn decimal_u32_never_allocates() {
        let ((), heap) = measure(|| {
            for text in ["0", "2040", "4294967296", "12.0", ""] {
                std::hint::black_box(decimal_u32(text));
            }
        });
        assert_eq!(heap.calls, 0, "the rows parse must not allocate");
    }
}
