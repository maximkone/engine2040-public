//! shadow_pcf: the CPU parity oracle of the forward shader's sunk-shadow
//! compare filter (SW92).
//!
//! Owner: render lane. Presentation only, never hashed. Units: 0..1 texels for
//! `uv` and depth, radians for the kernel angle.
//!
//! Three arms, chosen per tier by [`ShadowFilter`]:
//! - [`pcf3x3`]: nine compares on the texel and its neighbours (the reference).
//! - [`rotated4`]: four corners of a 1x1 texel box, turned by the 4x4 Bayer
//!   angle of the base texel (nine compares become four; no frame noise, so the
//!   pattern does not crawl under TAA).
//! - [`tap2`]: two taps on one fixed diagonal, no rotation (the web arm).
//!
//! The bias is the caller's: every arm compares `frag - bias`, so fewer taps
//! never move the contact line under a fighter's feet.
//!
//! Donor shape (zero bytes pasted): Godot `scene_forward_lights_inc.glsl`
//! `sample_directional_pcf_shadow` (kernel rotated by a per-pixel hash);
//! PORT(godotengine/godot:servers/rendering/renderer_rd/shaders/
//! scene_forward_lights_inc.glsl).

use crate::quality::ShadowFilter;

/// 4x4 Bayer matrix, the table `lod_dither.wgsl` and `forward.wgsl` carry.
pub const SHADOW_BAYER4: [u32; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];

/// Bayer value of one cell: the rotation index of a texel.
#[must_use]
pub fn bayer4_at(x: u32, y: u32) -> u32 {
    let row = (y & 3) * 4;
    let cell = (row + (x & 3)) & 15;
    SHADOW_BAYER4[usize::try_from(cell).unwrap_or(0)]
}

/// Kernel rotation in radians for one texel: `(bayer + 0.5) * 2*PI/16`, bounded
/// to `0..2*PI` so a rotated corner never leaves the 1x1 box.
#[must_use]
pub fn rotation_of(x: u32, y: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a Bayer value is 0..=15, exact in f32"
    )]
    let bayer = bayer4_at(x, y) as f32;
    (bayer + 0.5) * std::f32::consts::FRAC_PI_8
}

/// One depth map the arms sample: `size x size`, row-major, 0..1 depths.
#[derive(Clone, Copy, Debug)]
pub struct DepthMap<'a> {
    /// Depths, row-major, `size * size` entries.
    pub depth: &'a [f32],
    /// Edge in texels.
    pub size: u32,
}

impl DepthMap<'_> {
    /// Stored depth under `uv`, clamped to the edge; a short map reads lit.
    #[must_use]
    pub fn stored(&self, uv: [f32; 2]) -> f32 {
        let edge = self.size;
        if edge == 0 || !uv[0].is_finite() || !uv[1].is_finite() {
            return 1.0;
        }
        let x = texel_index(uv[0], edge);
        let y = texel_index(uv[1], edge);
        let Some(row) = usize::try_from(y)
            .ok()
            .and_then(|r| usize::try_from(edge).ok().and_then(|e| r.checked_mul(e)))
        else {
            return 1.0;
        };
        let at = row.saturating_add(usize::try_from(x).unwrap_or(0));
        self.depth
            .get(at)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(1.0)
    }

    /// One compare at `uv`: 1 lit, 0 shadowed (`frag - bias <= stored`).
    #[must_use]
    pub fn compare(&self, uv: [f32; 2], frag: f32, bias: f32) -> f32 {
        if frag - bias <= self.stored(uv) {
            1.0
        } else {
            0.0
        }
    }
}

/// Texel index of one `0..=1` coordinate: clamped to the edge, no wrap.
fn texel_index(v: f32, edge: u32) -> u32 {
    let span = f32::from(u16::try_from(edge.min(u32::from(u16::MAX))).unwrap_or(1));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0..edge before the cast"
    )]
    let idx = (v.clamp(0.0, 1.0) * (span - 1.0)).round() as u32;
    idx
}

/// Nine compares: the texel and its neighbours.
#[must_use]
pub fn pcf3x3(map: &DepthMap<'_>, uv: [f32; 2], frag: f32, bias: f32) -> f32 {
    let texel = texel_step(map.size);
    let base = snap(uv, texel);
    let mut lit = 0.0;
    for y in -1..=1 {
        for x in -1..=1 {
            let at = [base[0] + to_f32(x) * texel, base[1] + to_f32(y) * texel];
            lit += map.compare(at, frag, bias);
        }
    }
    lit / 9.0
}

/// Four corners of a 1x1 texel box, turned by the base texel's Bayer angle.
#[must_use]
pub fn rotated4(map: &DepthMap<'_>, uv: [f32; 2], frag: f32, bias: f32) -> f32 {
    let texel = texel_step(map.size);
    let base = snap(uv, texel);
    let cell_x = texel_index(base[0], map.size);
    let cell_y = texel_index(base[1], map.size);
    let angle = rotation_of(cell_x, cell_y);
    let (s, c) = angle.sin_cos();
    let corners = [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]];
    let mut lit = 0.0;
    for [cx, cy] in corners {
        let off = [cx * c - cy * s, cx * s + cy * c];
        let at = [base[0] + off[0] * texel, base[1] + off[1] * texel];
        lit += map.compare(at, frag, bias);
    }
    lit * 0.25
}

/// Two taps on the fixed `(1,1)` diagonal: the web arm.
#[must_use]
pub fn tap2(map: &DepthMap<'_>, uv: [f32; 2], frag: f32, bias: f32) -> f32 {
    let texel = texel_step(map.size);
    let base = snap(uv, texel);
    let a = map.compare([base[0] - 0.5 * texel, base[1] - 0.5 * texel], frag, bias);
    let b = map.compare([base[0] + 0.5 * texel, base[1] + 0.5 * texel], frag, bias);
    (a + b) * 0.5
}

/// The arm a tier runs.
#[must_use]
pub fn filter(map: &DepthMap<'_>, uv: [f32; 2], frag: f32, bias: f32, f: ShadowFilter) -> f32 {
    match f {
        ShadowFilter::Pcf3x3 => pcf3x3(map, uv, frag, bias),
        ShadowFilter::Rotated4 => rotated4(map, uv, frag, bias),
        ShadowFilter::Tap2 => tap2(map, uv, frag, bias),
    }
}

/// An integer tap offset as `f32` for the swap.
#[expect(
    clippy::cast_precision_loss,
    reason = "tap offsets are -1..=1, exact in f32"
)]
fn to_f32(v: i32) -> f32 {
    v as f32
}

/// One texel in `uv` space for a `size` map; a bad size reads a whole map.
fn texel_step(size: u32) -> f32 {
    let edge = size.max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "shadow map edges are 128..=4096, exact in f32"
    )]
    let span = edge as f32;
    1.0 / span
}

/// Snap `uv` to the centre of the texel that holds it, as the shader does.
fn snap(uv: [f32; 2], texel: f32) -> [f32; 2] {
    let snap_one = |v: f32| {
        if texel <= 0.0 {
            v
        } else {
            (v / texel).floor().mul_add(texel, 0.5 * texel)
        }
    };
    [snap_one(uv[0]), snap_one(uv[1])]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat wall at `depth`, and the same wall with a straight step edge.
    fn flat(depth: f32, size: u32) -> Vec<f32> {
        vec![depth; usize::try_from(size * size).expect("map fits")]
    }
    fn step(size: u32) -> Vec<f32> {
        let mut out = Vec::with_capacity(usize::try_from(size * size).expect("map fits"));
        for _y in 0..size {
            for x in 0..size {
                out.push(if x < size / 2 { 0.40 } else { 0.90 });
            }
        }
        out
    }
    /// Sample `count` texel centres spread over the map, as `0..1` uv.
    fn centres(size: u32, count: u32) -> Vec<[f32; 2]> {
        let edge = f32::from(u16::try_from(size).expect("edge fits"));
        let n = f32::from(u16::try_from(count).expect("count fits"));
        let mut out = Vec::new();
        for i in 0..count {
            let f = f32::from(u16::try_from(i).expect("index fits")) / n;
            let v = (f * edge).floor().mul_add(1.0 / edge, 0.5 / edge);
            out.push([v, v]);
        }
        out
    }

    #[test]
    fn rotated_four_taps_land_within_two_levels_of_the_nine() {
        let size = 64u32;
        let depths = flat(0.5, size);
        let map = DepthMap {
            depth: &depths,
            size,
        };
        // Wall lit at 40 degrees: the fragment sits just in front of the stored
        // surface, so with the caller's bias every tap reads lit.
        let frag = 0.49;
        let bias = 0.01;
        let mut worst = 0.0f32;
        for uv in centres(size, 32) {
            let nine = pcf3x3(&map, uv, frag, bias);
            let four = rotated4(&map, uv, frag, bias);
            assert_eq!(nine, 1.0, "the wall reads lit");
            worst = worst.max((nine - four).abs());
        }
        assert!(worst * 255.0 < 2.0, "worst {worst}");
        // And a wall the fragment sits behind stays shadowed on both arms.
        let shadowed = flat(0.5, size);
        let map = DepthMap {
            depth: &shadowed,
            size,
        };
        let frag = 0.60;
        for uv in centres(size, 16) {
            assert_eq!(pcf3x3(&map, uv, frag, bias), 0.0);
            assert_eq!(rotated4(&map, uv, frag, bias), 0.0);
        }
    }

    #[test]
    fn rotated_taps_hold_a_straight_edge_within_four_levels() {
        let size = 64u32;
        let depths = step(size);
        let map = DepthMap {
            depth: &depths,
            size,
        };
        // The left half is nearer: a fragment just behind it is lit on the left
        // and shadowed on the right, with the bias kept as the caller's.
        let frag = 0.45;
        let bias = 0.01;
        let texel = texel_step(size);
        let mut worst = 0.0f32;
        let mut compared = 0;
        for uv in centres(size, 48) {
            // Skip the one-texel transition band the edge hides in.
            let from_edge = (uv[0] - 0.5).abs();
            if from_edge < 2.0 * texel {
                continue;
            }
            let nine = pcf3x3(&map, uv, frag, bias);
            let four = rotated4(&map, uv, frag, bias);
            let two = tap2(&map, uv, frag, bias);
            worst = worst.max((nine - four).abs()).max((nine - two).abs());
            compared += 1;
        }
        assert!(compared > 30, "the sweep covered {compared} points");
        assert!(worst * 255.0 < 4.0, "worst {worst}");
        // The tap counts the row trades: 9 -> 4 (56% gone) -> 2 (78% gone).
        assert_eq!(ShadowFilter::Pcf3x3.taps(), 9);
        assert_eq!(ShadowFilter::Rotated4.taps(), 4);
        assert_eq!(ShadowFilter::Tap2.taps(), 2);
        assert_eq!(5 * 100 / 9, 55, "5 of 9 compares gone, 56% rounded");
        assert_eq!(7 * 100 / 9, 77, "7 of 9 compares gone, 78% rounded");
        // Every one of the 16 Bayer cells turns the kernel, and no cell leaves
        // the first turn.
        let mut angles: Vec<u32> = (0..16)
            .map(|i| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the angle is 0..2*PI scaled to 1000 and rounded"
                )]
                let scaled = (rotation_of(i & 3, i / 4) * 1000.0).round() as u32;
                scaled
            })
            .collect();
        angles.sort_unstable();
        angles.dedup();
        assert_eq!(angles.len(), 16, "sixteen distinct rotations");
        assert!(rotation_of(0, 0) > 0.0 && rotation_of(15, 15) < std::f32::consts::TAU);
    }

    #[test]
    fn tier_table_picks_fewer_taps_down_the_rows() {
        use crate::quality::{knobs, shadow_filter, shadow_filter_constants, Quality};
        assert_eq!(
            shadow_filter(Quality::Potato).taps(),
            4,
            "potato rotates four"
        );
        assert_eq!(
            shadow_filter(Quality::Mobile).taps(),
            4,
            "mobile rotates four"
        );
        assert_eq!(shadow_filter(Quality::Web).taps(), 2, "web drops to two");
        assert_eq!(shadow_filter(Quality::Comfort).taps(), 9, "comfort keeps");
        assert_eq!(
            shadow_filter(Quality::High).taps(),
            9,
            "high keeps the grid"
        );
        // The forward pipeline's constant carries the arm's word on every row.
        for q in Quality::ALL {
            let [(name, value)] = shadow_filter_constants(q);
            assert_eq!(name, "SHADOW_FILTER");
            assert_eq!(value, f64::from(shadow_filter(q).as_u32()));
            // And the row still builds (the filter is not a stored field).
            let _ = knobs(q);
        }
    }

    #[test]
    fn tap2_and_the_grid_agree_away_from_the_edge() {
        let size = 32u32;
        let depths = flat(0.30, size);
        let map = DepthMap {
            depth: &depths,
            size,
        };
        for uv in centres(size, 8) {
            // Unoccluded: both read lit.
            assert_eq!(
                filter(&map, uv, 0.31, 0.0, ShadowFilter::Tap2),
                filter(&map, uv, 0.31, 0.0, ShadowFilter::Pcf3x3)
            );
            // Occluded: both read shadowed.
            assert_eq!(
                filter(&map, uv, 0.50, 0.0, ShadowFilter::Tap2),
                filter(&map, uv, 0.50, 0.0, ShadowFilter::Pcf3x3)
            );
        }
        // A zero-sized map reads lit rather than wrapping.
        let empty = DepthMap {
            depth: &[],
            size: 0,
        };
        assert_eq!(tap2(&empty, [0.5, 0.5], 1.0, 0.0), 1.0);
    }
}
