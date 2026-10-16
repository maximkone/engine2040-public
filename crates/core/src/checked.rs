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
}
