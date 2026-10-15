//! Fixed tiny int8 inference step for the tick.
//!
//! Consumer: the fixed-tick brain mapping quantized percepts to actions.
//! Owner: this module owns the fixed dims, the fixed net and the errors.
//! Units: weights are `i8`; inputs are `i8` steps; outputs are `i8`-range
//! values carried as `i32`; accumulation is saturating `i32`.
//! Determinism: integer-only with no heap, no tables and no clock, so the
//! same inputs give the same outputs on every machine.
//! Errors: wrong slice lengths give [`SpikeErr`] and leave output untouched.

/// Input lanes per tick step.
pub const SPIKE_INPUT_DIM: usize = 2;
/// Output lanes per tick step.
pub const SPIKE_OUTPUT_DIM: usize = 2;
/// Fixed weights, row-major `output` by `input`.
pub const SPIKE_WEIGHTS: [[i8; SPIKE_INPUT_DIM]; SPIKE_OUTPUT_DIM] = [[3, -4], [5, 6]];
/// Fixed bias per output.
pub const SPIKE_BIAS: [i32; SPIKE_OUTPUT_DIM] = [1, -2];

/// Fixed-step inference failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpikeErr {
    /// Input slice length differs from [`SPIKE_INPUT_DIM`].
    BadInputLen,
    /// Output slice length differs from [`SPIKE_OUTPUT_DIM`].
    BadOutputLen,
}

impl core::fmt::Display for SpikeErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            SpikeErr::BadInputLen => "input slice length differs from the fixed input dim",
            SpikeErr::BadOutputLen => "output slice length differs from the fixed output dim",
        };
        f.write_str(s)
    }
}

impl std::error::Error for SpikeErr {}

/// Run the fixed 2 by 2 int8 step with explicit weights.
///
/// Each output is `bias + sum(weight * input)` in saturating `i32`,
/// then clamped to `[-128, 127]`. Caller-owned slices, no heap per call,
/// so the tick can reuse its buffers. Same inputs give same outputs.
/// Returns [`SpikeErr`] without touching `out` when a slice has the
/// wrong length.
pub fn spike_infer(
    weights: &[[i8; SPIKE_INPUT_DIM]; SPIKE_OUTPUT_DIM],
    bias: &[i32; SPIKE_OUTPUT_DIM],
    input: &[i8],
    out: &mut [i32],
) -> Result<(), SpikeErr> {
    if input.len() != SPIKE_INPUT_DIM {
        return Err(SpikeErr::BadInputLen);
    }
    if out.len() != SPIKE_OUTPUT_DIM {
        return Err(SpikeErr::BadOutputLen);
    }
    for (o, slot) in out.iter_mut().enumerate() {
        let mut acc = bias[o];
        for (w, v) in weights[o].iter().zip(input.iter()) {
            acc = acc.saturating_add(i32::from(*w).saturating_mul(i32::from(*v)));
        }
        *slot = acc.clamp(-128, 127);
    }
    Ok(())
}

/// Run the fixed net: the in-tick entry.
///
/// Uses [`SPIKE_WEIGHTS`] and [`SPIKE_BIAS`], same math and errors as
/// [`spike_infer`], no heap per call. Short or long slices are errors,
/// never padding or truncation.
pub fn spike_tick(input: &[i8], out: &mut [i32]) -> Result<(), SpikeErr> {
    spike_infer(&SPIKE_WEIGHTS, &SPIKE_BIAS, input, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        // o0 = 1 + 3*7 - 4*8 = -10; o1 = -2 + 5*7 + 6*8 = 81.
        let mut out = [0i32; SPIKE_OUTPUT_DIM];
        spike_tick(&[7, 8], &mut out).expect("fixed slices fit");
        assert_eq!(out, [-10, 81]);
    }

    #[test]
    fn saturation_and_shape_mismatch() {
        // Positive i32 overflow saturates: MAX + 1*1 stays MAX, then 127.
        // Wrapping would give MIN, then -128, so this catches it.
        let mut out = [0i32; SPIKE_OUTPUT_DIM];
        spike_infer(&[[1, 0], [0, 0]], &[i32::MAX, 0], &[1, 0], &mut out)
            .expect("fixed slices fit");
        assert_eq!(out, [127, 0]);
        // Negative i32 overflow saturates: MIN + 1*-1 stays MIN, then -128.
        let mut out = [0i32; SPIKE_OUTPUT_DIM];
        spike_infer(&[[1, 0], [0, 0]], &[i32::MIN, 0], &[-1, 0], &mut out)
            .expect("fixed slices fit");
        assert_eq!(out, [-128, 0]);
        // Output clamp: 127*127 + 127*127 = 32258, then 127.
        let mut out = [0i32; SPIKE_OUTPUT_DIM];
        spike_infer(&[[127, 127], [0, 0]], &[0, 0], &[127, 127], &mut out)
            .expect("fixed slices fit");
        assert_eq!(out, [127, 0]);
        // Shape mismatch leaves output untouched.
        let mut keep = [55i32; SPIKE_OUTPUT_DIM];
        assert_eq!(spike_tick(&[7], &mut keep), Err(SpikeErr::BadInputLen));
        assert_eq!(keep, [55, 55]);
        let mut keep = [55i32; SPIKE_OUTPUT_DIM];
        assert_eq!(
            spike_tick(&[7, 8, 9], &mut keep),
            Err(SpikeErr::BadInputLen)
        );
        assert_eq!(keep, [55, 55]);
        let mut short = [55i32; 1];
        assert_eq!(spike_tick(&[7, 8], &mut short), Err(SpikeErr::BadOutputLen));
        assert_eq!(short, [55]);
        let mut long = [55i32; 3];
        assert_eq!(spike_tick(&[7, 8], &mut long), Err(SpikeErr::BadOutputLen));
        assert_eq!(long, [55, 55, 55]);
    }
}
