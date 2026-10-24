//! gtao_half: GTAO at half resolution on the web and potato tiers (QA5).
//!
//! Owner: render lane. The tier table picks the AO resolution per quality
//! tier: half on `Potato` and `Web` (the browser tab and the iGPU floor),
//! full on `Mobile`, `Comfort` and `High`. The half target is
//! `ceil(w/2) x ceil(h/2)`, the same size `gtao::GtaoConfig::half_width` and
//! the device kernel use. The depth-aware upsample back to full res is
//! `gtao::upsample_bilateral` (the CPU twin); the tests here pin that a depth
//! step stays sharp across the upsample. Units: pixels for sizes, typed
//! `Quality` tiers. Pure, no allocation, presentation only, never hashed.

use crate::gtao::GTAO_HALF_DIV;
use crate::quality::Quality;

/// AO resolution a quality tier renders at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GtaoRes {
    /// The AO kernel runs at `ceil(w/2) x ceil(h/2)` and the result is
    /// upsampled to the frame with the depth-aware bilateral filter.
    Half,
    /// The AO kernel runs at the frame size.
    Full,
}

/// The AO resolution for a tier: half on `Potato` and `Web`, full on the rest.
/// Pure table, no clock, no allocation.
#[must_use]
pub const fn gtao_res_for_tier(tier: Quality) -> GtaoRes {
    match tier {
        Quality::Potato | Quality::Web => GtaoRes::Half,
        Quality::Mobile | Quality::Comfort | Quality::High => GtaoRes::Full,
    }
}

/// AO target size for a frame at `res` in pixels: `ceil(w/2) x ceil(h/2)` at
/// half, the frame itself at full. A zero side passes through unchanged (the
/// suspend path runs no pass). Pure, no allocation.
#[must_use]
pub fn gtao_target_size(res: GtaoRes, width_px: u32, height_px: u32) -> [u32; 2] {
    if width_px == 0 || height_px == 0 {
        return [width_px, height_px];
    }
    match res {
        GtaoRes::Half => [
            width_px.div_ceil(GTAO_HALF_DIV),
            height_px.div_ceil(GTAO_HALF_DIV),
        ],
        GtaoRes::Full => [width_px, height_px],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtao::{upsample_bilateral, GtaoAo, GtaoConfig};

    fn config(width_px: u32, height_px: u32) -> GtaoConfig {
        GtaoConfig {
            width_px,
            height_px,
            radius_px: 8.0,
            falloff_m: 2.0,
            intensity: 1.0,
        }
    }

    #[test]
    fn gtao_half_target_is_ceil_half_of_the_frame() {
        assert_eq!(gtao_target_size(GtaoRes::Half, 1920, 1080), [960, 540]);
        assert_eq!(gtao_target_size(GtaoRes::Half, 1921, 1081), [961, 541]);
        assert_eq!(gtao_target_size(GtaoRes::Half, 1, 1), [1, 1]);
        assert_eq!(gtao_target_size(GtaoRes::Half, 3, 5), [2, 3]);
    }

    #[test]
    fn gtao_half_target_matches_the_config_half_size() {
        for (w, h) in [(1920u32, 1080u32), (1921, 1081), (7, 3), (1, 9)] {
            let cfg = config(w, h);
            assert_eq!(
                gtao_target_size(GtaoRes::Half, w, h),
                [cfg.half_width() as u32, cfg.half_height() as u32],
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn gtao_full_target_is_the_frame_and_zero_passes_through() {
        assert_eq!(gtao_target_size(GtaoRes::Full, 1921, 1081), [1921, 1081]);
        assert_eq!(gtao_target_size(GtaoRes::Half, 0, 720), [0, 720]);
        assert_eq!(gtao_target_size(GtaoRes::Full, 0, 0), [0, 0]);
    }

    #[test]
    fn gtao_tier_table_is_half_on_web_and_potato_full_elsewhere() {
        assert_eq!(gtao_res_for_tier(Quality::Potato), GtaoRes::Half);
        assert_eq!(gtao_res_for_tier(Quality::Web), GtaoRes::Half);
        assert_eq!(gtao_res_for_tier(Quality::Mobile), GtaoRes::Full);
        assert_eq!(gtao_res_for_tier(Quality::Comfort), GtaoRes::Full);
        assert_eq!(gtao_res_for_tier(Quality::High), GtaoRes::Full);
        let half: Vec<&str> = Quality::ALL
            .into_iter()
            .filter(|t| gtao_res_for_tier(*t) == GtaoRes::Half)
            .map(|t| t.name())
            .collect();
        assert_eq!(half, ["potato", "web"], "exactly the two half tiers");
    }

    /// A 8x4 frame with a depth step at x=4: near side 2 m with AO 0.2 (the
    /// half texels at hx 0..2), far side 40 m with AO 1.0 (hx 2..4). The
    /// bilateral upsample must keep the near occlusion on the near side of
    /// the step and keep the far side open; a flat-depth control shows the
    /// same pixel blends to 0.6, so the depth weight is what holds the edge.
    #[test]
    fn gtao_bilateral_upsample_keeps_a_depth_step_sharp() {
        let cfg = config(8, 4);
        let half_row = [0.2f32, 0.2, 1.0, 1.0];
        let half = GtaoAo {
            width_half: 4,
            height_half: 2,
            values: half_row.iter().chain(half_row.iter()).copied().collect(),
        };
        let stepped: Vec<f32> = (0..32)
            .map(|i| if i % 8 < 4 { 2.0 } else { 40.0 })
            .collect();
        let out = upsample_bilateral(&half, &stepped, &cfg).expect("step upsamples");
        // Near side (x=0..=3) keeps its occlusion; plain bilinear would give 0.6 at x=3.
        for x in 0..4 {
            assert!(out[x] < 0.25, "near pixel x={x} bled to {}", out[x]);
        }
        // Far side (x=4..=7) is never darkened by the near side's AO.
        for x in 4..8 {
            assert!((out[x] - 1.0).abs() < 1e-6, "far pixel x={x} is {}", out[x]);
        }
        let flat = vec![2.0f32; 32];
        let control = upsample_bilateral(&half, &flat, &cfg).expect("flat upsamples");
        assert!(
            (control[3] - 0.6).abs() < 1e-5,
            "flat depth blends bilinear, got {}",
            control[3]
        );
    }
}
