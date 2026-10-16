//! The one quantization edge: `f32` percept lanes to the `i8` net inputs.
//!
//! Consumer: every caller that feeds [`infer_into`](crate::infer_into) from
//! float observations (the distill teacher recorder and student controller
//! today; the arena percept once the net binds). Owner: this module owns the
//! rounding rule, the saturation range and the NaN value, so no caller
//! re-derives them. Units: a lane's scale is `i8` steps per input unit
//! (for example `127 / 16` for a lane whose full scale is 16 m).
//! Determinism: one IEEE `f32` multiply then `round_ties_even`, no fused
//! multiply-add, no libm, so the same `(value, scale)` gives the same byte on
//! every machine. Errors: none; every input maps to a defined byte.
//!
//! Rules, fixed here once:
//! - `q = round_half_even(value * steps_per_unit)`;
//! - saturate to `[-127, 127]` (symmetric: `-128` is never produced, so
//!   negating a lane never overflows);
//! - `NaN` value maps to [`QUANT_NAN`] (0); `+inf`/`-inf` saturate;
//! - a scale that is `NaN`, infinite, zero or negative maps every value to
//!   [`QUANT_NAN`].

/// Byte a `NaN` lane (or an unusable scale) maps to.
pub const QUANT_NAN: i8 = 0;
/// Largest quantized lane value.
pub const QUANT_MAX: i8 = 127;
/// Smallest quantized lane value (symmetric with [`QUANT_MAX`]).
pub const QUANT_MIN: i8 = -127;

/// Quantize one lane: `round_half_even(value * steps_per_unit)` saturated to
/// `[QUANT_MIN, QUANT_MAX]`, `NaN` to [`QUANT_NAN`]. Pure and total.
pub fn quantize_lane(value: f32, steps_per_unit: f32) -> i8 {
    if value.is_nan() || !steps_per_unit.is_finite() || steps_per_unit <= 0.0 {
        return QUANT_NAN;
    }
    let rounded = (value * steps_per_unit).round_ties_even();
    if rounded >= f32::from(QUANT_MAX) {
        QUANT_MAX
    } else if rounded <= f32::from(QUANT_MIN) {
        QUANT_MIN
    } else {
        // Strictly inside (-127, 127) and integral: the cast is exact.
        rounded as i8
    }
}

/// Quantize `values` lane by lane with the matching fixed scale table into
/// `out`. Writes `min(values.len(), steps_per_unit.len(), out.len())` lanes
/// and returns that count; never allocates, never panics.
pub fn quantize_lanes(values: &[f32], steps_per_unit: &[f32], out: &mut [i8]) -> usize {
    let mut written = 0usize;
    for ((slot, value), scale) in out.iter_mut().zip(values).zip(steps_per_unit) {
        *slot = quantize_lane(*value, *scale);
        written += 1;
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_half_to_even() {
        // 0.5, 1.5, 2.5, -0.5, -1.5 steps: ties go to the even neighbour.
        assert_eq!(quantize_lane(0.5, 1.0), 0);
        assert_eq!(quantize_lane(1.5, 1.0), 2);
        assert_eq!(quantize_lane(2.5, 1.0), 2);
        assert_eq!(quantize_lane(-0.5, 1.0), 0);
        assert_eq!(quantize_lane(-1.5, 1.0), -2);
        assert_eq!(quantize_lane(-2.5, 1.0), -2);
        // Off-tie values round to nearest.
        assert_eq!(quantize_lane(0.51, 1.0), 1);
        assert_eq!(quantize_lane(-0.51, 1.0), -1);
        // A 16 m full-scale lane: 8 m is 63.5 steps, a tie, so 64 (even).
        assert_eq!(quantize_lane(8.0, 127.0 / 16.0), 64);
    }

    #[test]
    fn nan_inf_and_saturation_are_defined() {
        assert_eq!(quantize_lane(f32::NAN, 16.0), QUANT_NAN);
        assert_eq!(quantize_lane(-f32::NAN, 16.0), QUANT_NAN);
        assert_eq!(quantize_lane(f32::INFINITY, 16.0), QUANT_MAX);
        assert_eq!(quantize_lane(f32::NEG_INFINITY, 16.0), QUANT_MIN);
        assert_eq!(quantize_lane(f32::MAX, 16.0), QUANT_MAX);
        assert_eq!(quantize_lane(f32::MIN, 16.0), QUANT_MIN);
        assert_eq!(quantize_lane(1e30, 1e30), QUANT_MAX);
        // Exactly at and just past the edges.
        assert_eq!(quantize_lane(127.0, 1.0), 127);
        assert_eq!(quantize_lane(126.5, 1.0), 126);
        assert_eq!(quantize_lane(127.4, 1.0), 127);
        assert_eq!(quantize_lane(-127.0, 1.0), -127);
        assert_eq!(quantize_lane(-128.0, 1.0), -127);
        assert_eq!(quantize_lane(-1000.0, 1.0), -127);
        // Unusable scales map everything to the NaN byte.
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(quantize_lane(3.0, scale), QUANT_NAN, "scale {scale}");
        }
        // Signed zero and subnormals land on zero.
        assert_eq!(quantize_lane(-0.0, 16.0), 0);
        assert_eq!(quantize_lane(f32::MIN_POSITIVE, 16.0), 0);
    }

    #[test]
    fn lanes_use_their_own_scale_and_never_overrun() {
        let values = [1.0f32, 1.0, f32::NAN, -9.0];
        let scales = [16.0f32, 127.0 / 16.0, 16.0, 16.0];
        let mut out = [99i8; 4];
        assert_eq!(quantize_lanes(&values, &scales, &mut out), 4);
        assert_eq!(out, [16, 8, 0, -127]);
        // Short output: only the first two lanes are written.
        let mut short = [99i8; 2];
        assert_eq!(quantize_lanes(&values, &scales, &mut short), 2);
        assert_eq!(short, [16, 8]);
        // Short scale table: the extra values are left alone.
        let mut wide = [99i8; 4];
        assert_eq!(quantize_lanes(&values, &scales[..1], &mut wide), 1);
        assert_eq!(wide, [16, 99, 99, 99]);
    }
}
