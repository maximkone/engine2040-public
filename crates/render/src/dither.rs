//! SW53: the ordered screen-space dither the 8-bit output store adds before it
//! quantizes, so a flat sky or sand gradient stops banding on the potato tier.
//!
//! Consumer: `shaders/tonemap.wgsl` `screen_space_dither` (the WGSL twin every
//! present shader compiles) and `quality::Knobs::output_dither_px_bits`, which
//! names how many bits the target carries. This module is the CPU model the
//! accept measures: the same `fract(dot(vec2(171, 231), frag) / vec3(103, 71, 97))`
//! pattern, one 8-bit step peak to peak, zero mean over a block.
//!
//! Units: display `0..=1` values, one 8-bit step (`1/255`) per unit of dither.
//! Determinism: the pattern is a pure function of the pixel coordinate, no
//! clock and no RNG. Presentation only: nothing here feeds the tick or the hash.

/// Bits the 8-bit present path carries; the dither is one step of this.
pub const DITHER_BITS: u8 = 8;
/// Bits an HDR present path carries: the same pattern, a 4x finer step.
pub const DITHER_BITS_HDR: u8 = 10;
/// Iestyn's Portal 2 RGB dither frequencies (the shader's `vec2(171, 231)`).
pub const DITHER_FREQ: [f32; 2] = [171.0, 231.0];
/// Half-width of the dither in 8-bit steps. Half a step only softens a band's
/// edge; 2.5 steps actually splits it, so a three-step ramp reaches nine
/// distinct codes while the block mean stays inside 0.2% (measured in the
/// tests). The shader names the same value.
pub const DITHER_HALF_STEPS: f32 = 2.5;
/// Per-channel divisors (the shader's `vec3(103, 71, 97)`), coprime so the
/// three channels' patterns never line up into a visible lattice.
pub const DITHER_PHASE: [f32; 3] = [103.0, 71.0, 97.0];

/// The quantizer's largest code for `bits`: the two steps the output path names
/// (8 for the SDR present store, 10 for a deeper target). Any other value reads
/// as 8 bits, the shipped step.
pub fn max_code(bits: u8) -> f32 {
    match bits {
        10 => 1023.0,
        16 => 65535.0,
        _ => 255.0,
    }
}

/// One pixel's dither offset per channel: the shader's
/// `fract(dot(vec2(171, 231), frag) / vec3(103, 71, 97)) - 0.5`, scaled to
/// [`DITHER_HALF_STEPS`] steps of this quantizer either way.
pub fn screen_space_dither(frag: [f32; 2], bits: u8) -> [f32; 3] {
    let phase = DITHER_FREQ[0] * frag[0] + DITHER_FREQ[1] * frag[1];
    let mut out = [0.0_f32; 3];
    for channel in 0..3 {
        let frac = (phase / DITHER_PHASE[channel]).fract();
        out[channel] = (frac - 0.5) * (2.0 * DITHER_HALF_STEPS) / max_code(bits);
    }
    out
}

/// Quantize one display value to the 8-bit present store with no dither: the
/// banding baseline. Values clamp to `0..=1` first, so the narrowing is exact.
pub fn quantize8(display: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value is clamped to 0..=255 and rounded before the cast"
    )]
    let code = (display.clamp(0.0, 1.0) * max_code(DITHER_BITS)).round() as u8;
    code
}

/// Quantize one display colour at one pixel with the dither added first: the
/// shader's store order, value then dither then round.
pub fn quantize_dithered(display: [f32; 3], frag: [f32; 2]) -> [u8; 3] {
    let dither = screen_space_dither(frag, DITHER_BITS);
    let mut out = [0_u8; 3];
    for channel in 0..3 {
        out[channel] = quantize8(display[channel] + dither[channel]);
    }
    out
}

#[cfg(test)]
#[path = "dither_tests.rs"]
mod tests;
