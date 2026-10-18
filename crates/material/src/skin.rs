//! skin: the terminator LUT and the thin transmittance of the skin lobe (RL50).
//!
//! Skin is not plastic because light does not stop at the surface: red scatters far, green less,
//! blue barely. Two numbers per pixel carry that, and both come from the same six-Gaussian
//! diffusion profile (d'Eon and Luebke, GPU Gems 3 chapter 14, table of variances and RGB
//! weights; the sum-of-Gaussians idea is the one Filament, O3DE and the separable-SSS family use):
//!
//! * The terminator LUT (the pre-integrated skin of Penner, SIGGRAPH 2011): for a surface of
//!   curvature `c` and a light at `n.l`, the cosine blurred by the profile along the ring the
//!   curvature describes. It replaces the old one-number wrap: red keeps light past the
//!   terminator, blue stops at it, and the dark side of a cheek goes warm instead of grey.
//! * The thin transmittance: the share of each channel that passes a slab of thickness `d`, the
//!   same Gaussians evaluated at `d`: ears and fingers glow red when lit from behind.
//!
//! The LUT is `LUT_N` curvatures by `LUT_N` light cosines of RGBA32F texels, [`LUT_BYTES`] bytes,
//! built here at first use (fixed sample order, f64 accumulation: the same bytes on every host).
//! Every curvature column is renormalized so that the LUT integrates over `n.l` exactly like the
//! unblurred cosine, which makes the white furnace over the whole sphere 1 by construction.
//! `wgsl` emits the table and the lookups for the generated shader; [`terminator`] is the CPU
//! twin of that lookup. Units: curvature `0..=1` where 1 is a 2 mm radius, millimeters for
//! thickness. Presentation only; nothing here enters a hash.

use std::f64::consts::PI;
use std::sync::OnceLock;

use engine_core::checked::{f32_from_f64, f32_from_u64, saturating_f64_to_u64};

/// Texels per LUT axis.
pub const LUT_N: usize = 32;
/// Floats per texel: red, green, blue and an alpha of 1.
pub const LUT_CHANNELS: usize = 4;
/// Size of the LUT image in bytes: 32 by 32 RGBA32F.
pub const LUT_BYTES: usize = LUT_N * LUT_N * LUT_CHANNELS * 4;
/// Radius in millimeters of curvature 1.
pub const MIN_RADIUS_MM: f32 = 2.0;
/// Curvature units per unit of `|dN| / |dP|` (1/meter): a radius of `r` meters is curvature
/// `MIN_RADIUS_MM / (1000 r)`.
pub const CURVATURE_PER_INVERSE_METER: f32 = MIN_RADIUS_MM / 1000.0;

/// `LUT_N - 1` and `LUT_N - 2` as floats: the last texel coordinate and the last cell.
const LAST: f64 = 31.0;
const PREV: f64 = 30.0;
const LAST_F32: f32 = 31.0;
const _: () = assert!(LUT_N == 32, "LAST and PREV are written for 32 texels");

/// The position of texel `i` along an axis, `0..=1`.
fn node(i: usize) -> f64 {
    f64::from(u16::try_from(i).unwrap_or(u16::MAX)) / LAST
}

/// The skin profile: variance in mm squared and RGB weight of each Gaussian (GPU Gems 3,
/// chapter 14). Each channel's weights sum to 1.
pub const GAUSSIANS: [(f64, [f64; 3]); 6] = [
    (0.0064, [0.233, 0.455, 0.649]),
    (0.0484, [0.100, 0.336, 0.344]),
    (0.187, [0.118, 0.198, 0.0]),
    (0.567, [0.113, 0.007, 0.007]),
    (1.99, [0.358, 0.004, 0.0]),
    (7.41, [0.078, 0.0, 0.0]),
];

/// Samples per LUT texel and channel along the ring.
const RING_SAMPLES: u32 = 2000;

/// The profile of one channel at distance `d` mm: radial 2D Gaussians, weights summing to 1.
fn profile(d: f64, channel: usize) -> f64 {
    GAUSSIANS
        .iter()
        .map(|(variance, weight)| {
            weight[channel] * (-d * d / (2.0 * variance)).exp() / (2.0 * PI * variance)
        })
        .sum()
}

/// The cosine at angle `theta` blurred by the profile along a ring of radius `radius_mm`.
fn blurred_cosine(theta: f64, radius_mm: f64, channel: usize) -> f64 {
    // Past five sigma of the widest Gaussian the profile is gone.
    let widest = GAUSSIANS[GAUSSIANS.len() - 1].0.sqrt();
    let reach = (5.0 * widest / radius_mm).min(PI);
    let (mut num, mut den) = (0.0, 0.0);
    for j in 0..RING_SAMPLES {
        let x = -reach + (f64::from(j) + 0.5) / f64::from(RING_SAMPLES) * 2.0 * reach;
        let w = profile(2.0 * radius_mm * (x.abs() / 2.0).sin(), channel);
        num += w * (theta + x).cos().max(0.0);
        den += w;
    }
    num / den
}

fn build_lut() -> Vec<f32> {
    let step = 2.0 / LAST;
    let mut lut = vec![1.0f32; LUT_N * LUT_N * LUT_CHANNELS];
    for ic in 0..LUT_N {
        let curvature = node(ic);
        for channel in 0..3 {
            let column: Vec<f64> = (0..LUT_N)
                .map(|i| {
                    let nl = -1.0 + 2.0 * node(i);
                    if ic == 0 {
                        nl.max(0.0)
                    } else {
                        blurred_cosine(nl.acos(), f64::from(MIN_RADIUS_MM) / curvature, channel)
                    }
                })
                .collect();
            // The trapezoid over n.l of the unblurred cosine is 1/2; the interpolated LUT
            // integrates to exactly this.
            let area = step * (column.iter().sum::<f64>() - 0.5 * (column[0] + column[LUT_N - 1]));
            for (i, v) in column.iter().enumerate() {
                lut[(ic * LUT_N + i) * LUT_CHANNELS + channel] = f32_from_f64(v * 0.5 / area);
            }
        }
    }
    lut
}

fn lut() -> &'static [f32] {
    static LUT: OnceLock<Vec<f32>> = OnceLock::new();
    LUT.get_or_init(build_lut)
}

/// The LUT image: RGBA32F little-endian, curvature rows (0 to 1) of `LUT_N` light cosines
/// (-1 to 1). [`LUT_BYTES`] long; the same bytes every call.
#[must_use]
pub fn lut_bytes() -> Vec<u8> {
    lut().iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The cell a texel coordinate falls in and the fraction across it.
fn cell(s: f32) -> (usize, f32) {
    let floor = saturating_f64_to_u64(f64::from(s.floor()));
    let i = usize::try_from(floor).unwrap_or(0).min(LUT_N - 2);
    (i, s - f32_from_u64(u64::try_from(i).unwrap_or(0)))
}

/// Bilinear lookup of the LUT: the blurred cosine per channel for a light at `nl` on a surface
/// of `curvature`. The CPU twin of `skin_terminator` in the generated shader.
#[must_use]
pub fn terminator(nl: f32, curvature: f32) -> [f32; 3] {
    let (x0, fx) = cell((nl * 0.5 + 0.5).clamp(0.0, 1.0) * LAST_F32);
    let (y0, fy) = cell(curvature.clamp(0.0, 1.0) * LAST_F32);
    let texel = |x: usize, y: usize, c: usize| lut()[(y * LUT_N + x) * LUT_CHANNELS + c];
    std::array::from_fn(|c| {
        let a = texel(x0, y0, c) + (texel(x0 + 1, y0, c) - texel(x0, y0, c)) * fx;
        let b = texel(x0, y0 + 1, c) + (texel(x0 + 1, y0 + 1, c) - texel(x0, y0 + 1, c)) * fx;
        a + (b - a) * fy
    })
}

/// The share of each channel that passes a slab of `thickness_mm`: the profile's Gaussians
/// evaluated at the thickness. 1 at zero thickness, red last to fade.
#[must_use]
pub fn transmittance(thickness_mm: f32) -> [f32; 3] {
    let d = f64::from(thickness_mm);
    std::array::from_fn(|c| {
        f32_from_f64(
            GAUSSIANS
                .iter()
                .map(|(variance, weight)| weight[c] * (-d * d / variance).exp())
                .sum::<f64>(),
        )
    })
}

/// The WGSL of the skin lobe: the LUT as a private array, `skin_terminator` (the lookup, as
/// [`terminator`]) and `skin_transmit` (as [`transmittance`]).
#[must_use]
pub fn wgsl() -> String {
    let mut out = String::with_capacity(48 * 1024);
    out.push_str(&format!(
        "var<private> SKIN_LUT: array<vec3<f32>, {n}> = array<vec3<f32>, {n}>(\n",
        n = LUT_N * LUT_N
    ));
    for texel in lut().chunks_exact(LUT_CHANNELS) {
        out.push_str(&format!(
            "    vec3<f32>({:.5}, {:.5}, {:.5}),\n",
            texel[0], texel[1], texel[2]
        ));
    }
    out.push_str(");\n");
    out.push_str(&format!(
        "fn skin_terminator(nl: f32, curvature: f32) -> vec3<f32> {{\n\
         \x20   let sx = clamp(nl * 0.5 + 0.5, 0.0, 1.0) * {last:.1};\n\
         \x20   let sy = clamp(curvature, 0.0, 1.0) * {last:.1};\n\
         \x20   let x0 = u32(min(floor(sx), {prev:.1}));\n\
         \x20   let y0 = u32(min(floor(sy), {prev:.1}));\n\
         \x20   let fx = sx - f32(x0);\n\
         \x20   let fy = sy - f32(y0);\n\
         \x20   let a = mix(SKIN_LUT[y0 * {n}u + x0], SKIN_LUT[y0 * {n}u + x0 + 1u], fx);\n\
         \x20   let b = mix(SKIN_LUT[(y0 + 1u) * {n}u + x0], SKIN_LUT[(y0 + 1u) * {n}u + x0 + 1u], fx);\n\
         \x20   return mix(a, b, fy);\n\
         }}\n",
        last = LAST,
        prev = PREV,
        n = LUT_N
    ));
    out.push_str("fn skin_transmit(thickness_mm: f32) -> vec3<f32> {\n    let d2 = thickness_mm * thickness_mm;\n    return ");
    for (i, (variance, weight)) in GAUSSIANS.iter().enumerate() {
        if i > 0 {
            out.push_str("\n        + ");
        }
        out.push_str(&format!(
            "vec3<f32>({:.3}, {:.3}, {:.3}) * exp(-d2 / {variance:?})",
            weight[0], weight[1], weight[2]
        ));
    }
    out.push_str(";\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_lut_is_16384_bytes_and_the_same_twice() {
        let first = lut_bytes();
        assert_eq!(first.len(), 16_384);
        assert_eq!(first.len(), LUT_BYTES);
        // A second build from nothing, not the cached one.
        let rebuilt: Vec<u8> = build_lut().iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(first, rebuilt);
        assert_eq!(lut_bytes(), first);
    }

    #[test]
    fn skin_lut_red_terminator_is_at_least_one_and_a_half_times_blue_at_curvature_half() {
        let [r, g, b] = terminator(0.0, 0.5);
        assert!(r >= 1.5 * b, "terminator r {r:.4} g {g:.4} b {b:.4}");
        assert!(
            r > g && g > b,
            "the channels fall in order: {r:.4} {g:.4} {b:.4}"
        );
    }

    #[test]
    fn skin_lut_flat_skin_is_lambert_and_curved_skin_leaks_past_the_terminator() {
        let flat = terminator(0.5, 0.0);
        for v in flat {
            assert!((v - 0.5).abs() < 0.02, "flat n.l 0.5 gives {v}");
        }
        assert!(
            terminator(-0.5, 0.0)[0] < 0.02,
            "flat skin is dark behind the terminator"
        );
        let curved = terminator(-0.1, 0.6);
        assert!(
            curved[0] > 0.005,
            "red reaches the dark side of a curved cheek: {curved:?}"
        );
        assert!(curved[0] > curved[2]);
    }

    #[test]
    fn skin_lut_every_column_keeps_the_cosine_integral() {
        let step = 2.0 / (LUT_N - 1) as f64;
        for ic in 0..LUT_N {
            for channel in 0..3 {
                let at = |i: usize| f64::from(lut()[(ic * LUT_N + i) * LUT_CHANNELS + channel]);
                let sum: f64 = (0..LUT_N).map(at).sum::<f64>() - 0.5 * (at(0) + at(LUT_N - 1));
                assert!(
                    (sum * step - 0.5).abs() < 1e-5,
                    "column {ic} channel {channel}: {}",
                    sum * step
                );
            }
        }
    }

    #[test]
    fn skin_lut_values_are_finite_and_in_range() {
        assert!(lut()
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.2).contains(v)));
        assert!(lut()
            .chunks_exact(LUT_CHANNELS)
            .all(|texel| texel[3] == 1.0));
    }

    #[test]
    fn skin_lut_wgsl_holds_every_texel_and_both_lookups() {
        let text = wgsl();
        assert_eq!(
            text.matches("vec3<f32>(").count(),
            LUT_N * LUT_N + GAUSSIANS.len()
        );
        assert!(text.contains("fn skin_terminator"));
        assert!(text.contains("fn skin_transmit"));
        assert_eq!(wgsl(), text);
    }

    #[test]
    fn skin_transmittance_is_one_when_thin_and_goes_red_when_thick() {
        for v in transmittance(0.0) {
            assert!((v - 1.0).abs() < 1e-5);
        }
        let [r, g, b] = transmittance(1.5);
        assert!(r > 0.1 && r > 20.0 * g && g >= b, "r {r} g {g} b {b}");
        let thicker = transmittance(3.0);
        assert!(thicker[0] < r, "thicker passes less");
    }
}
