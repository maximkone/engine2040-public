//! lod_fade: LOD dither cross-fade, no pop, no alpha sort.
//!
//! Owner: render lane. Units: meters for distances, 0..=255 keep share,
//! texel coords for dither taps. Determinism: integer threshold compare in
//! fixed order, so the same inputs give the same keep bits twice. Errors:
//! none, non-finite distances fade out and zero band is a hard switch.
//! Budget: O(1) per tap, no allocation. Consumer: `draw::Draw::fade`
//! carries the share per draw and the forward pass thresholds pixels
//! against it, so the switch dissolves instead of popping. Method follows
//! keijiro CrossFadingLod 4x4 Bayer.

/// Bayer cells in y-major rows, one permutation of 0..=15.
pub const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Compiled WGSL mirror of the dither table and keep rule.
pub const LOD_DITHER_WGSL: &str = include_str!("shaders/lod_dither.wgsl");

/// Keep share 0..=255 at `dist_m`: 255 fully near, 0 fully far.
///
/// `switch_m` is the distance of the half-faded middle, `band_m` the width
/// of the transition around it. Non-finite distances fade out; a band at
/// or below zero is a hard switch at `switch_m`.
pub fn fade(dist_m: f32, switch_m: f32, band_m: f32) -> u8 {
    if !dist_m.is_finite() {
        return 0;
    }
    if !switch_m.is_finite() || !band_m.is_finite() || band_m <= 0.0 {
        return if dist_m < switch_m { 255 } else { 0 };
    }
    let lo_m = switch_m - band_m * 0.5;
    let t = ((dist_m - lo_m) / band_m).clamp(0.0, 1.0);
    share_to_u8(((1.0 - t) * 255.0).round())
}

/// Integral float share in 0..=255 as a byte, without a float cast.
///
/// Bit decode keeps every in-range integer exact: non-finite and
/// non-positive read 0, 255 and above saturate, and an integral float
/// below 255 is its significand shifted by its exponent. Callers round
/// first, so a fractional input never arrives.
fn share_to_u8(q: f32) -> u8 {
    if !q.is_finite() || q <= 0.0 {
        return 0;
    }
    if q >= 255.0 {
        return 255;
    }
    let bits = q.to_bits();
    let biased = (bits >> 23) & 0xff;
    let shift = 150u32.saturating_sub(biased).min(31);
    let sig = (bits & 0x7f_ffff) | 0x80_0000;
    u8::try_from((sig >> shift).min(255)).unwrap_or(255)
}

/// Bayer cell 0..=15 at a pixel, wrapping every 4 texels.
pub fn bayer4(x: u32, y: u32) -> u8 {
    let xi = usize::try_from(x & 3).unwrap_or(0);
    let yi = usize::try_from(y & 3).unwrap_or(0);
    BAYER4[yi][xi]
}

/// True when the pixel at (`x`, `y`) survives at this `fade` share.
///
/// Over one 4x4 tile the kept count equals `ceil(fade / 16)`: 0 keeps
/// none, 255 keeps all 16. Touching order is the fixed Bayer order.
pub fn keep(fade: u8, x: u32, y: u32) -> bool {
    bayer4(x, y) < fade.div_ceil(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_maps_band_edges_center_and_failures() {
        assert_eq!(fade(8.0, 10.0, 4.0), 255);
        assert_eq!(fade(12.0, 10.0, 4.0), 0);
        assert_eq!(fade(10.0, 10.0, 4.0), 128);
        assert_eq!(fade(9.0, 10.0, 4.0), 191);
        assert_eq!(fade(11.0, 10.0, 4.0), 64);
        assert_eq!(fade(9.9, 10.0, 0.0), 255);
        assert_eq!(fade(10.0, 10.0, 0.0), 0);
        assert_eq!(fade(10.1, 10.0, -2.0), 0);
        assert_eq!(fade(f32::NAN, 10.0, 4.0), 0);
        assert_eq!(fade(f32::INFINITY, 10.0, 4.0), 0);
        assert_eq!(fade(8.0, f32::NAN, 4.0), 0);
    }

    fn kept_on_tile(fade_share: u8) -> u32 {
        let mut kept: u32 = 0;
        for y in 0..4 {
            for x in 0..4 {
                if keep(fade_share, x, y) {
                    kept += 1;
                }
            }
        }
        kept
    }

    #[test]
    fn kept_share_equals_ceil_over_tile_x2() {
        for _ in 0..2 {
            for fade_share in 0..=255u8 {
                let want = u32::from(fade_share.div_ceil(16));
                assert_eq!(kept_on_tile(fade_share), want, "fade {fade_share}");
            }
        }
        assert_eq!(kept_on_tile(0), 0);
        assert_eq!(kept_on_tile(1), 1);
        assert_eq!(kept_on_tile(16), 1);
        assert_eq!(kept_on_tile(17), 2);
        assert_eq!(kept_on_tile(255), 16);
    }

    #[test]
    fn share_bits_match_every_byte() {
        for b in 0..=255u8 {
            assert_eq!(share_to_u8(f32::from(b)), b, "byte {b}");
        }
        assert_eq!(share_to_u8(0.0), 0);
        assert_eq!(share_to_u8(-1.0), 0);
        assert_eq!(share_to_u8(f32::NAN), 0);
        assert_eq!(share_to_u8(255.0), 255);
        assert_eq!(share_to_u8(10_000.0), 255);
    }

    #[test]
    fn wgsl_holds_same_16_values() {
        let flat: Vec<String> = BAYER4.iter().flatten().map(|v| format!("{v}u")).collect();
        assert_eq!(flat.len(), 16);
        let row = flat.join(", ");
        assert!(
            LOD_DITHER_WGSL.contains(&row),
            "wgsl holds the same 16 cells in order"
        );
        assert!(LOD_DITHER_WGSL.contains("fn lod_keep"));
        assert!(LOD_DITHER_WGSL.contains("fn bayer4_at"));
    }
}
