//! lod_fade: LOD dither cross-fade, no pop, no alpha sort.
//!
//! Owner: render lane. Units: meters for distances, 0..=255 keep share,
//! texel coords for dither taps. Determinism: integer threshold compare in
//! fixed order, so the same inputs give the same keep bits twice. Errors:
//! none, non-finite distances fade out and zero band is a hard switch.
//! Budget: O(1) per tap, no allocation. Consumer: `draw::fade_for_lod`
//! fills `draw::Draw::fade` with the share per draw. `keep` and the WGSL
//! mirror are the per-pixel test a forward pass binds against that share,
//! so the switch dissolves instead of popping; no pass binds the WGSL yet.
//! Method follows keijiro CrossFadingLod 4x4 Bayer.

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

    #[test]
    fn wgsl_threshold_and_index_match_rust() {
        // The table alone is not enough: the shader must also round the
        // share up to 16ths and index y-major with a 4-texel wrap.
        assert!(LOD_DITHER_WGSL.contains("(fade + 15u) / 16u"));
        assert!(LOD_DITHER_WGSL.contains("bayer4_table[(y & 3u) * 4u + (x & 3u)]"));
        assert!(LOD_DITHER_WGSL.contains("bayer4_at(x, y) < threshold"));
        let flat: Vec<u8> = BAYER4.iter().flatten().copied().collect();
        for y in 0..8u32 {
            for x in 0..8u32 {
                let at = usize::try_from((y & 3) * 4 + (x & 3)).expect("index fits");
                assert_eq!(flat[at], bayer4(x, y), "cell {x},{y}");
            }
        }
    }

    #[test]
    fn bayer_cells_are_one_wrapping_permutation() {
        let mut seen = [false; 16];
        for y in 0..4 {
            for x in 0..4 {
                seen[usize::from(bayer4(x, y))] = true;
            }
        }
        assert!(seen.iter().all(|s| *s), "all 16 cells appear once");
        assert_eq!(bayer4(5, 6), bayer4(1, 2));
        assert_eq!(bayer4(u32::MAX, u32::MAX), bayer4(3, 3));
    }

    #[test]
    fn kept_masks_are_nested_as_the_share_falls() {
        // A pixel dropped at a higher share never returns at a lower one,
        // so a dissolving LOD never flickers.
        for hi in 0..=255u8 {
            for lo in 0..=hi {
                for y in 0..4 {
                    for x in 0..4 {
                        assert!(
                            !keep(lo, x, y) || keep(hi, x, y),
                            "lo {lo} hi {hi} at {x},{y}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn lod_switch_dissolves_in_small_steps_not_a_pop() {
        use crate::draw::{fade_for_lod, LodTable};
        // Switch at 10 m, hysteresis 0.5 m, so the band is 9.5 m to 10.5 m.
        let table = LodTable {
            id: 1,
            bands_m: vec![10.0, 20.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        };
        let mut seen = [false; 17];
        let mut prev = kept_on_tile(fade_for_lod(9.0, 0, &table));
        assert_eq!(prev, 16, "fully kept before the band");
        seen[16] = true;
        for step in 1..=200u16 {
            let dist_m = 9.0 + f32::from(step) * 0.01;
            let kept = kept_on_tile(fade_for_lod(dist_m, 0, &table));
            assert!(kept <= prev, "share rose at {dist_m} m");
            assert!(prev - kept <= 1, "popped {prev} -> {kept} at {dist_m} m");
            seen[usize::try_from(kept).expect("count fits")] = true;
            prev = kept;
        }
        assert_eq!(prev, 0, "fully gone after the band");
        assert!(seen.iter().all(|s| *s), "every share 0..=16 is visited");
        // Same inputs, same bits.
        assert_eq!(fade_for_lod(10.2, 0, &table), fade_for_lod(10.2, 0, &table));
    }
}
