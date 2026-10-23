//! Shared helpers for the windowed KAM2 proof harnesses (iso and sprite).
//!
//! Both proofs read the same cooked pixels back and compare them the same way,
//! so the sRGB conversion and the near-colour count live here once. Native
//! only, like the proofs that call them.

/// sRGB byte triple to three linear `f32` channels.
pub(crate) fn linear(rgb: [u8; 3]) -> [f32; 3] {
    [
        engine_assets::bc::srgb_byte_to_linear(rgb[0]),
        engine_assets::bc::srgb_byte_to_linear(rgb[1]),
        engine_assets::bc::srgb_byte_to_linear(rgb[2]),
    ]
}

/// Pixels in `rgba` whose first three bytes are within 3 of `want`.
pub(crate) fn count_near(rgba: &[u8], want: [u8; 3]) -> usize {
    rgba.chunks_exact(4)
        .filter(|pixel| {
            pixel[0].abs_diff(want[0]) <= 3
                && pixel[1].abs_diff(want[1]) <= 3
                && pixel[2].abs_diff(want[2]) <= 3
        })
        .count()
}
