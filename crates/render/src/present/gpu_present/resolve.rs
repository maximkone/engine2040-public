//! resolve: 4x MSAA box-resolve math plus the multisample plan (EG13 slice 2).
//!
//! Owner: render lane. Caller (this slice): `msaa.rs` includes this file with
//! `#[path]` and re-exports the plan items (plus the resolve math under
//! `cfg(test)` for the edge-test chain), so no `mod` edit outside the
//! owned paths is needed. GPU caller (sibling presenter-hunk slice, which owns
//! the presenter target setup hunk): `submit.rs` (`ensure_targets` builds the
//! 4x HDR plus depth pair, `encode_forward_opaque` sets the view/resolveTarget
//! pair) and `presenter.rs` (4x forward plus sky pipelines); both reach these
//! items as `super::msaa::*`. Stays apart from TAA/SMAA: `post.rs::taa_resolve`
//! is never touched here.
//! Units: sample counts (1 or 4), linear-radiance texels. Determinism: pure
//! CPU math, fixed tap order, f64 accumulation; same finite inputs give same
//! outputs on any host; no GPU here, so headless tests run. Errors: none here.
//! Budget: no allocation on any path.

use super::{MsaaConfig, MSAA_SAMPLE_COUNT};

/// Sample count of the 1x resolve target: the 4x view averages down into the
/// 1x HDR target before the blit (resolve before post and blit).
pub const RESOLVE_SAMPLE_COUNT: u32 = 1;

/// Multisample shape for a config: 4x color plus 4x depth with a resolve when
/// on, 1x with no resolve when off. Read-only plan: the presenter-hunk slice
/// builds the textures from it. Coverage plus depth run per subsample while
/// color shades once (the pipeline state below enables no per-sample shading).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttachmentPlan {
    /// Samples per color pixel: 4 on, 1 off.
    pub color_samples: u32,
    /// Samples per depth texel: 4 on, 1 off (depth grows with color).
    pub depth_samples: u32,
    /// True while the pass must resolve the 4x view into the 1x HDR target.
    pub resolve: bool,
}

/// Plan for a config: `{4, 4, true}` on, `{1, 1, false}` off. Pure: same
/// config gives the same plan twice.
#[must_use]
pub fn attachment_plan(config: &MsaaConfig) -> AttachmentPlan {
    if super::msaa_enabled(config) {
        AttachmentPlan {
            color_samples: MSAA_SAMPLE_COUNT,
            depth_samples: MSAA_SAMPLE_COUNT,
            resolve: true,
        }
    } else {
        AttachmentPlan {
            color_samples: RESOLVE_SAMPLE_COUNT,
            depth_samples: RESOLVE_SAMPLE_COUNT,
            resolve: false,
        }
    }
}

/// True while the pass must resolve: the on path only. Pure: same config
/// gives the same answer twice.
#[must_use]
pub fn needs_resolve(config: &MsaaConfig) -> bool {
    attachment_plan(config).resolve
}

/// Pipeline multisample state for a config: count 4 on with the full sample
/// mask and no alpha-to-coverage, so 4x multisample with one shading run per
/// pixel (cost is bandwidth, not shading); count 1 off. Pure, no GPU work.
#[must_use]
pub fn pipeline_multisample(config: &MsaaConfig) -> wgpu::MultisampleState {
    wgpu::MultisampleState {
        count: super::msaa_sample_count(config),
        mask: !0u64,
        alpha_to_coverage_enabled: false,
    }
}

/// Box-resolve one channel: the mean of the 4 subsamples in fixed tap order
/// with f64 accumulation. Pure: same quad gives the same value twice. A flat
/// quad resolves flat (interiors hold); a mixed quad resolves strictly between
/// 0 and 1 (edges blend). Inputs are finite linear radiance. Widening is exact
/// (`From`, never `as`); the narrowing cast is safe because the f64 sum of 4
/// finite f32 values divided by 4 stays within f32 range, so the result stays
/// finite (float-to-float has no `From` to use). Same shape as the fog-feed
/// mean in `submit.rs`. Test-only: no production path resolves on the CPU
/// (the GPU pass averages through its view/resolveTarget pair).
#[cfg(test)]
#[must_use]
pub fn resolve_average(samples: [f32; 4]) -> f32 {
    let sum = f64::from(samples[0])
        + f64::from(samples[1])
        + f64::from(samples[2])
        + f64::from(samples[3]);
    (sum / 4.0) as f32
}

/// Box-resolve one RGBA texel: per-channel [`resolve_average`]. Pure: same
/// quad gives the same texel twice.
#[cfg(test)]
#[must_use]
pub fn resolve_rgba(samples: [[f32; 4]; 4]) -> [f32; 4] {
    [
        resolve_average([samples[0][0], samples[1][0], samples[2][0], samples[3][0]]),
        resolve_average([samples[0][1], samples[1][1], samples[2][1], samples[3][1]]),
        resolve_average([samples[0][2], samples[1][2], samples[2][2], samples[3][2]]),
        resolve_average([samples[0][3], samples[1][3], samples[2][3], samples[3][3]]),
    ]
}

/// True when the resolved quad value sits strictly between 0 and 1: the edge
/// oracle for the Accept test (on-path quads blend, off-path quads stay
/// stepped). Pure.
#[cfg(test)]
#[must_use]
pub fn resolved_is_intermediate(quad: [f32; 4]) -> bool {
    let value = resolve_average(quad);
    value > 0.0 && value < 1.0
}

#[cfg(test)]
mod tests {
    // Through the `msaa` re-export on purpose: this is the exact import path
    // the sibling GPU edge test uses (`super::msaa::*` from `submit.rs`), so
    // these tests prove the handoff path resolves.
    use super::super::{
        attachment_plan, edge_has_intermediate, needs_resolve, pipeline_multisample,
        resolve_average, resolve_rgba, resolved_is_intermediate, AttachmentPlan, MsaaConfig,
        MSAA_SAMPLE_COUNT, RESOLVE_SAMPLE_COUNT,
    };

    #[test]
    fn flat_quad_resolves_flat() {
        assert_eq!(
            resolve_average([0.0, 0.0, 0.0, 0.0]),
            0.0,
            "all-outside stays 0"
        );
        assert_eq!(
            resolve_average([1.0, 1.0, 1.0, 1.0]),
            1.0,
            "all-inside stays 1"
        );
        assert!(!resolved_is_intermediate([0.0; 4]), "flat 0 blends nothing");
        assert!(!resolved_is_intermediate([1.0; 4]), "flat 1 blends nothing");
        assert_eq!(
            resolve_average([0.0; 4]),
            resolve_average([0.0; 4]),
            "same inputs give same value twice"
        );
    }

    #[test]
    fn mixed_quad_resolves_intermediate() {
        assert_eq!(
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            0.5,
            "half-covered edge resolves to a half"
        );
        assert!(
            resolved_is_intermediate([0.0, 0.0, 1.0, 1.0]),
            "edge quad blends"
        );
        assert!(
            resolved_is_intermediate([0.0, 0.25, 0.5, 1.0]),
            "partial quad blends"
        );
        // The resolved on-value feeds the slice-1 edge oracle: the on path
        // blends while the off path stays stepped.
        let on = [resolve_average([0.0, 0.0, 1.0, 1.0])];
        let off = [resolve_average([1.0, 1.0, 1.0, 1.0])];
        assert!(
            edge_has_intermediate(&on, &off),
            "resolved edge shows an intermediate on value only"
        );
        assert_eq!(
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            "same inputs give same value twice"
        );
    }

    #[test]
    fn plan_off_is_1x_without_resolve_and_on_is_4x_with_resolve() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        let off_plan = attachment_plan(&off);
        assert_eq!(
            off_plan,
            AttachmentPlan {
                color_samples: 1,
                depth_samples: 1,
                resolve: false,
            },
            "off stays the 1x path"
        );
        assert!(!needs_resolve(&off), "off resolves nothing");
        let on_plan = attachment_plan(&on);
        assert_eq!(on_plan.color_samples, MSAA_SAMPLE_COUNT, "on colors 4x");
        assert_eq!(
            on_plan.depth_samples, MSAA_SAMPLE_COUNT,
            "depth grows with color"
        );
        assert!(
            on_plan.resolve && needs_resolve(&on),
            "on resolves before the blit"
        );
        assert_eq!(RESOLVE_SAMPLE_COUNT, 1, "the resolve target stays 1x");
        assert_eq!(
            attachment_plan(&on),
            attachment_plan(&on),
            "same inputs give same plan twice"
        );
        assert_eq!(
            attachment_plan(&off),
            attachment_plan(&off),
            "same inputs give same plan twice"
        );
    }

    #[test]
    fn pipeline_state_is_4x_single_shade_on_and_1x_off() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        let off_state = pipeline_multisample(&off);
        assert_eq!(off_state.count, 1, "off stays the 1x pipeline");
        let on_state = pipeline_multisample(&on);
        assert_eq!(
            on_state.count, MSAA_SAMPLE_COUNT,
            "on picks the 4x pipeline"
        );
        assert!(
            !on_state.alpha_to_coverage_enabled,
            "one shading run per pixel: cost is bandwidth"
        );
        assert_eq!(on_state.mask, !0u64, "every subsample covered");
        assert_eq!(
            pipeline_multisample(&on).count,
            pipeline_multisample(&on).count,
            "same inputs give same state twice"
        );
    }

    #[test]
    fn rgba_resolves_per_channel_stable_x2() {
        let quad = [
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ];
        assert_eq!(
            resolve_rgba(quad),
            [0.5, 0.5, 0.5, 1.0],
            "each channel averages its own taps"
        );
        assert_eq!(
            resolve_rgba(quad),
            resolve_rgba(quad),
            "same inputs give same texel twice"
        );
        assert_eq!(
            resolve_rgba([[1.0; 4]; 4]),
            [1.0; 4],
            "flat interiors hold per channel"
        );
    }
}
