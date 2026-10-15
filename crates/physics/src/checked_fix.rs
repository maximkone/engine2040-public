//! checked_fix: CQ10 tick-path narrowing on the core checked owner.
//! Units: mm, mN and Q16 gain words in, `i32` words out.
//! Determinism: integer math only; inputs the tick already range-checks keep
//! the exact bits the old narrowing casts produced, so the hash never moves.
//! Errors: none (saturating); absurd inputs clamp instead of wrapping.
//! Budget: O(1) per call, no allocation.
//! Consumer: grapple drive and integrate paths (Q16 force downscale and
//! clamp-then-narrow); this module lands the tested helpers first.

use engine_core::checked::saturating_i64_to_i32;

/// Clamp an `i64` word to `i32` bounds, then narrow without wrapping.
/// Mirrors the clamp-then-cast in the bone integrator: every tick-clamped
/// input keeps its exact bits, and an unclamped absurd input stops at the
/// bound instead of wrapping around it.
pub fn clamp_narrow_to_i32(v: i64, lo: i32, hi: i32) -> i32 {
    let lo64 = i64::from(lo);
    let hi64 = i64::from(hi);
    let clamped = if v < lo64 {
        lo64
    } else if v > hi64 {
        hi64
    } else {
        v
    };
    saturating_i64_to_i32(clamped)
}

/// Downscale a Q16 product (`v * scale >> 16`) to `i32` without wrapping.
/// Mirrors the servo force downscale: `saturating_mul` matches plain `*` on
/// every in-range tick input (capped forces, Q16 scales), while an absurd
/// product saturates instead of overflowing into a wrapped force.
pub fn q16_downscale_to_i32(v: i64, scale_q16: i64) -> i32 {
    saturating_i64_to_i32(v.saturating_mul(scale_q16) >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    #[test]
    fn clamp_narrow_keeps_integrator_bits() {
        // Oracle: clamped inputs keep the exact bits the old `v as i32`
        // kept after the same clamp. (`as` below is the legacy-bits oracle;
        // production code uses the helper, never a cast.)
        for v in [
            -1_000_001i64,
            -1_000_000,
            -100_000,
            0,
            99,
            100_000,
            1_000_000,
            1_000_001,
        ] {
            let clamped = if v < -1_000_000 {
                -1_000_000
            } else if v > 1_000_000 {
                1_000_000
            } else {
                v
            };
            assert_eq!(clamp_narrow_to_i32(v, -1_000_000, 1_000_000), clamped as i32);
        }
    }

    #[test]
    fn clamp_narrow_stops_absurd_input_at_the_bound() {
        assert_eq!(
            clamp_narrow_to_i32(i64::MAX, -1_000_000, 1_000_000),
            1_000_000
        );
        assert_eq!(
            clamp_narrow_to_i32(i64::MIN, -1_000_000, 1_000_000),
            -1_000_000
        );
        assert_eq!(clamp_narrow_to_i32(5, -1_000_000, 1_000_000), 5);
    }

    #[test]
    fn q16_downscale_matches_servo_bits_in_range() {
        // Oracle: in-range servo inputs keep the exact bits the old
        // `((freq * scale) >> 16) as i32` produced.
        for freq in [-500_000i64, -1, 0, 1, 500_000] {
            for scale in [0i64, 32_768, 65_536] {
                let old = ((freq * scale) >> 16) as i32;
                assert_eq!(q16_downscale_to_i32(freq, scale), old);
            }
        }
    }

    #[test]
    fn q16_downscale_saturates_absurd_product() {
        // A force far past any cap saturates instead of wrapping into a
        // small or negative force that would corrupt the servo hash.
        assert_eq!(q16_downscale_to_i32(i64::MAX, 65_536), i32::MAX);
        assert_eq!(q16_downscale_to_i32(i64::MAX, i64::MAX), i32::MAX);
        assert_eq!(q16_downscale_to_i32(i64::MIN, 65_536), i32::MIN);
        assert_eq!(q16_downscale_to_i32(0, 65_536), 0);
    }

    #[test]
    fn checked_fix_never_allocates() {
        let ((), heap) = measure(|| {
            for v in [-1_000_001i64, 0, 1_000_000, i64::MAX, i64::MIN] {
                std::hint::black_box(clamp_narrow_to_i32(v, -1_000_000, 1_000_000));
                std::hint::black_box(q16_downscale_to_i32(v, 65_536));
            }
        });
        assert_eq!(heap.calls, 0, "tick-path narrowing must not allocate");
    }
}
