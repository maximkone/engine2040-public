//! checked: named integer conversions for tick paths. Owner: engine-core.
//! Units: counts and raw words (integers only, never floats).
//! Determinism: pure integer math; same input gives same output everywhere.
//! Errors: [`CheckedError`]; callers pick checked or saturating handling.
//! Budget: O(1) per call, no allocation.

use std::fmt;

/// Typed conversion failure for the checked integer entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedError {
    /// Value lies outside the target type range.
    OutOfRange,
}

impl fmt::Display for CheckedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => write!(f, "integer outside target range"),
        }
    }
}

impl std::error::Error for CheckedError {}

/// Checked `u64` to `u32`: `Err(OutOfRange)` past `u32::MAX`.
pub fn try_u64_to_u32(v: u64) -> Result<u32, CheckedError> {
    u32::try_from(v).map_err(|_| CheckedError::OutOfRange)
}

/// Saturating `u64` to `u32`: clamps past `u32::MAX`, never wraps.
pub fn saturating_u64_to_u32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Checked `i64` to `i32`: `Err(OutOfRange)` outside `i32::MIN..=MAX`.
pub fn try_i64_to_i32(v: i64) -> Result<i32, CheckedError> {
    i32::try_from(v).map_err(|_| CheckedError::OutOfRange)
}

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

/// Checked `usize` to `u32` for indices and counts: `Err(OutOfRange)`
/// when the word is wider than 32 bits and the value does not fit.
pub fn try_usize_to_u32(v: usize) -> Result<u32, CheckedError> {
    u32::try_from(v).map_err(|_| CheckedError::OutOfRange)
}

/// Saturating `usize` to `u32`: clamps at `u32::MAX`, never wraps.
pub fn saturating_usize_to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Checked `usize` to `i32` for bone and body indices: `Err(OutOfRange)`
/// when the value does not fit. Tick code indexes small tables with
/// `usize` and narrows to `i32` words; the old `i as i32` wrapped past
/// `i32::MAX`.
pub fn try_usize_to_i32(v: usize) -> Result<i32, CheckedError> {
    i32::try_from(v).map_err(|_| CheckedError::OutOfRange)
}

/// Saturating `usize` to `i32`: clamps at `i32::MAX`, never wraps.
pub fn saturating_usize_to_i32(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;

    #[test]
    fn narrowing_round_trips_small_values() {
        assert_eq!(try_u64_to_u32(0), Ok(0));
        assert_eq!(try_u64_to_u32(60), Ok(60));
        assert_eq!(try_u64_to_u32(u64::from(u32::MAX)), Ok(u32::MAX));
        assert_eq!(try_i64_to_i32(0), Ok(0));
        assert_eq!(try_i64_to_i32(-3), Ok(-3));
        assert_eq!(try_i64_to_i32(i64::from(i32::MIN)), Ok(i32::MIN));
        assert_eq!(try_i64_to_i32(i64::from(i32::MAX)), Ok(i32::MAX));
        assert_eq!(try_usize_to_u32(0), Ok(0));
        assert_eq!(try_usize_to_u32(1024), Ok(1024));
        assert_eq!(try_usize_to_u32(4_294_967_295), Ok(u32::MAX));
        assert_eq!(saturating_u64_to_u32(60), 60);
        assert_eq!(saturating_i64_to_i32(-3), -3);
        assert_eq!(saturating_usize_to_u32(7), 7);
        assert_eq!(try_usize_to_i32(0), Ok(0));
        assert_eq!(try_usize_to_i32(31), Ok(31));
        assert_eq!(saturating_usize_to_i32(31), 31);
    }

    #[test]
    fn checked_entry_reports_out_of_range() {
        assert_eq!(
            try_u64_to_u32(u64::from(u32::MAX) + 1),
            Err(CheckedError::OutOfRange)
        );
        assert_eq!(try_u64_to_u32(u64::MAX), Err(CheckedError::OutOfRange));
        assert_eq!(
            try_i64_to_i32(i64::from(i32::MAX) + 1),
            Err(CheckedError::OutOfRange)
        );
        assert_eq!(
            try_i64_to_i32(i64::from(i32::MIN) - 1),
            Err(CheckedError::OutOfRange)
        );
        assert_eq!(
            format!("{}", CheckedError::OutOfRange),
            "integer outside target range"
        );
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn checked_usize_reports_out_of_range_on_64bit() {
        assert_eq!(try_usize_to_u32(usize::MAX), Err(CheckedError::OutOfRange));
        assert_eq!(saturating_usize_to_u32(usize::MAX), u32::MAX);
    }

    #[test]
    fn saturating_clamps_where_wrapping_would_wrap() {
        // One past the top wraps to zero or `MIN` under truncation; the
        // saturating forms hold the bound instead.
        assert_eq!(saturating_u64_to_u32(u64::from(u32::MAX) + 1), u32::MAX);
        assert_eq!(saturating_u64_to_u32(u64::MAX), u32::MAX);
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MAX) + 1), i32::MAX);
        assert_eq!(saturating_i64_to_i32(i64::MAX), i32::MAX);
        assert_eq!(saturating_i64_to_i32(i64::from(i32::MIN) - 1), i32::MIN);
        assert_eq!(saturating_i64_to_i32(i64::MIN), i32::MIN);
    }

    #[test]
    fn saturating_matches_old_cast_bits_in_range() {
        // Oracle: on values the tick already range-checks, the new helpers
        // keep the exact bits the old narrowing casts produced, so adopting
        // them cannot move the golden hash. (`as` below is the legacy-bits
        // oracle; production code uses the helpers, never a cast.)
        for v in [0u64, 1, 60, 1024, 1_000_000, u64::from(u32::MAX)] {
            assert_eq!(saturating_u64_to_u32(v), v as u32);
            assert_eq!(try_u64_to_u32(v), Ok(v as u32));
        }
        for v in [
            0i64,
            -1,
            160,
            -100_000,
            1_000_000,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        ] {
            assert_eq!(saturating_i64_to_i32(v), v as i32);
            assert_eq!(try_i64_to_i32(v), Ok(v as i32));
        }
    }

    #[test]
    fn conversions_never_allocate() {
        let ((), heap) = measure(|| {
            for v in [0u64, 60, u64::from(u32::MAX), u64::MAX] {
                std::hint::black_box(saturating_u64_to_u32(v));
                let _ = std::hint::black_box(try_u64_to_u32(v));
            }
            for v in [0i64, -3, i64::from(i32::MIN), i64::MAX] {
                std::hint::black_box(saturating_i64_to_i32(v));
                let _ = std::hint::black_box(try_i64_to_i32(v));
            }
            for v in [0usize, 7, 4_294_967_295] {
                std::hint::black_box(saturating_usize_to_u32(v));
                let _ = std::hint::black_box(try_usize_to_u32(v));
            }
        });
        assert_eq!(heap.calls, 0, "checked conversions must not allocate");
    }
}
