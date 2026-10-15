//! cast: named float-to-int conversions for tick paths. Owner: physics.
//! Units: metres in, integer cells out. Determinism: saturating wrappers
//! match the old saturating cast bits; NaN maps to zero, out-of-range
//! clamps to `i32::MIN`/`MAX`. Errors: [`CastError`] for checked entry.
//!
//! Pattern from rust-embedded/heapless `src/len_type.rs` (MIT): bounded
//! integer types convert through `TryFrom` with a typed error, and call
//! sites choose checked or saturating handling. Adapted, no vendored bytes.
//! PORT(rust-embedded/heapless:src/len_type.rs)

use core::fmt;

/// Typed conversion failure for the checked float-to-int entry points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastError {
    /// Input is NaN or infinite, so no integer applies.
    NonFinite,
    /// Finite input lies outside the `i32` range.
    OutOfRange,
}

impl fmt::Display for CastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => write!(f, "non-finite float cannot become i32"),
            Self::OutOfRange => write!(f, "float outside i32 range"),
        }
    }
}

/// First float past the `i32` top (2^31) and lowest float inside the
/// `i32` range (-2^31). Built from bits, so both are exact with no
/// lossy decimal literal and no truncating cast.
const F32_I32_TOP: f32 = f32::from_bits(0x4F00_0000);
const F32_I32_BOTTOM: f32 = f32::from_bits(0xCF00_0000);

/// Bit-exact conversion of a finite integral float inside the `i32`
/// range. Reads the IEEE-754 exponent and mantissa through integer
/// shifts only, so no truncating cast appears on the tick path.
fn integral_to_i32(w: f32) -> i32 {
    if w == 0.0 {
        return 0;
    }
    let bits = w.to_bits();
    let neg = (bits >> 31) != 0;
    let exp_u8 = u8::try_from((bits >> 23) & 0xFF).expect("8-bit exponent field fits u8");
    if exp_u8 == 0 || exp_u8 == 255 {
        return if neg { i32::MIN } else { i32::MAX };
    }
    let exp = i32::from(exp_u8) - 127;
    if exp < 0 {
        return 0;
    }
    let mant = (bits & 0x7F_FFFF_u32) | 0x80_0000_u32;
    let mag_u64 = if exp >= 23 {
        u64::from(mant) << u32::try_from(exp - 23).expect("branch takes exp >= 23 only")
    } else {
        u64::from(mant) >> u32::try_from(23 - exp).expect("branch takes exp < 23 only")
    };
    let mag_i64 = i64::try_from(mag_u64).expect("i32-range magnitude fits i64");
    let signed = if neg { -mag_i64 } else { mag_i64 };
    i32::try_from(signed).expect("callers range-check into i32 before converting")
}

/// Checked `floor` to `i32`: `Err(NonFinite)` for NaN/infinite,
/// `Err(OutOfRange)` for finite values past the `i32` range.
pub fn try_floor_to_i32(v: f32) -> Result<i32, CastError> {
    if !v.is_finite() {
        return Err(CastError::NonFinite);
    }
    let w = v.floor();
    // `w` is finite here (checked above), so `contains` matches the old
    // `w >= TOP || w < BOTTOM` bits exactly.
    if !(F32_I32_BOTTOM..F32_I32_TOP).contains(&w) {
        return Err(CastError::OutOfRange);
    }
    Ok(integral_to_i32(w))
}

/// Checked `ceil` to `i32`, same error contract as [`try_floor_to_i32`].
pub fn try_ceil_to_i32(v: f32) -> Result<i32, CastError> {
    if !v.is_finite() {
        return Err(CastError::NonFinite);
    }
    let w = v.ceil();
    // Same finite-input contract as the `floor` entry above.
    if !(F32_I32_BOTTOM..F32_I32_TOP).contains(&w) {
        return Err(CastError::OutOfRange);
    }
    Ok(integral_to_i32(w))
}

/// Saturating `floor` to `i32`: NaN becomes zero, infinite and
/// out-of-range clamp, matching the old saturating cast bits.
pub fn floor_to_i32(v: f32) -> i32 {
    try_floor_to_i32(v).unwrap_or_else(|e| match e {
        CastError::NonFinite => {
            if v.is_nan() {
                0
            } else if v > 0.0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
        CastError::OutOfRange => {
            if v > 0.0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
    })
}

/// Saturating `ceil` to `i32`, same fallback contract as [`floor_to_i32`].
pub fn ceil_to_i32(v: f32) -> i32 {
    try_ceil_to_i32(v).unwrap_or_else(|e| match e {
        CastError::NonFinite => {
            if v.is_nan() {
                0
            } else if v > 0.0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
        CastError::OutOfRange => {
            if v > 0.0 {
                i32::MAX
            } else {
                i32::MIN
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_match_small_values() {
        assert_eq!(floor_to_i32(1.9), 1);
        assert_eq!(floor_to_i32(-1.1), -2);
        assert_eq!(floor_to_i32(-2.0), -2);
        assert_eq!(ceil_to_i32(1.1), 2);
        assert_eq!(ceil_to_i32(-1.9), -1);
        assert_eq!(ceil_to_i32(2.0), 2);
    }

    #[test]
    fn saturating_fallback_never_panics() {
        assert_eq!(floor_to_i32(f32::NAN), 0);
        assert_eq!(ceil_to_i32(f32::NAN), 0);
        assert_eq!(floor_to_i32(f32::INFINITY), i32::MAX);
        assert_eq!(floor_to_i32(f32::NEG_INFINITY), i32::MIN);
        assert_eq!(ceil_to_i32(f32::INFINITY), i32::MAX);
        assert_eq!(ceil_to_i32(f32::NEG_INFINITY), i32::MIN);
        assert_eq!(floor_to_i32(1.0e20), i32::MAX);
        assert_eq!(floor_to_i32(-1.0e20), i32::MIN);
        assert_eq!(ceil_to_i32(1.0e20), i32::MAX);
        assert_eq!(ceil_to_i32(-1.0e20), i32::MIN);
    }

    #[test]
    fn checked_entry_reports_each_failure() {
        assert_eq!(try_floor_to_i32(1.0), Ok(1));
        assert_eq!(try_ceil_to_i32(1.1), Ok(2));
        assert_eq!(try_floor_to_i32(f32::NAN), Err(CastError::NonFinite));
        assert_eq!(try_floor_to_i32(f32::INFINITY), Err(CastError::NonFinite));
        assert_eq!(try_floor_to_i32(1.0e20), Err(CastError::OutOfRange));
        assert_eq!(try_ceil_to_i32(-1.0e20), Err(CastError::OutOfRange));
    }

    #[test]
    fn checked_entry_holds_i32_edges() {
        // Edges built from bits: exact, with no lossy decimal literal.
        let below_top = f32::from_bits(0x4EFF_FFFF); // largest f32 below 2^31
        let top = f32::from_bits(0x4F00_0000); // 2^31, past `i32::MAX`
        let bottom = f32::from_bits(0xCF00_0000); // -2^31, equals `i32::MIN`
        assert_eq!(try_floor_to_i32(below_top), Ok(2_147_483_520));
        assert_eq!(try_floor_to_i32(bottom), Ok(i32::MIN));
        assert_eq!(try_floor_to_i32(top), Err(CastError::OutOfRange));
        assert_eq!(try_ceil_to_i32(top), Err(CastError::OutOfRange));
    }
}
