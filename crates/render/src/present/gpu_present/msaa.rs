//! msaa: 4x MSAA on/off switch state, the edge-blend helper (EG13 slice 1)
//! plus the box-resolve math and multisample plan in `resolve.rs` (slice 2).
//!
//! Owner: render lane, consumer the windowed [`super::presenter::Presenter`]
//! (off stays today's 1x path; the sibling presenter-hunk slice builds the
//! on-path 4x MSAA targets from the plan below and resolves before the blit).
//! Units: sample counts (1 or 4). Determinism: pure CPU state, same
//! inputs give same outputs on any host; no GPU here, so headless tests run.
//! Errors: none here. Budget: no allocation on any path.

/// MSAA sample count when the switch is on: 4x multisample with one shading
/// run per pixel (donor pipeline multisample count 4).
pub const MSAA_SAMPLE_COUNT: u32 = 4;

/// EG13 slice 2: the box-resolve math plus the multisample plan live in the
/// owned sibling `resolve.rs`, included here so no `mod` edit outside the
/// owned paths is needed. This module is its caller: the plan items below are
/// re-exported for the GPU caller (the sibling presenter-hunk slice reaches
/// them as `super::msaa::*`; `msaa_target_desc` already consumes them, so
/// they are production-used, not test-only).
#[path = "resolve.rs"]
mod resolve;
pub use resolve::{
    attachment_plan, needs_resolve, pipeline_multisample, AttachmentPlan, RESOLVE_SAMPLE_COUNT,
};
/// Edge-oracle API for the Accept test chain: the CPU resolve math the on/off
/// edge tests compare against (this slice's `resolve` tests plus the
/// sibling's GPU edge test). Gated on `test` because no production path
/// resolves on the CPU: the GPU pass averages through its view/resolveTarget
/// pair instead, so these names must never grow a non-test caller in this
/// subtree.
#[cfg(test)]
pub use resolve::{resolve_average, resolve_rgba, resolved_is_intermediate};

/// MSAA on/off switch. Default off keeps today's 1x path; the window leg
/// flips it through [`super::presenter::Presenter::set_msaa_enabled`].
/// Copyable state, no allocation, never read on the fixed tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MsaaConfig {
    /// True while 4x MSAA is on. Default false (the 1x path).
    pub enabled: bool,
}

/// MSAA target shape for a config: the HDR plus depth formats with the
/// chosen sample count. Read-only description, no textures built here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MsaaTargetDesc {
    /// Working color format (the `HDR_FORMAT` target, 1x resolve target).
    pub hdr_format: wgpu::TextureFormat,
    /// Depth format (the `DEPTH_FORMAT` target, multisampled when on).
    pub depth_format: wgpu::TextureFormat,
    /// Samples per pixel: 1 off, [`MSAA_SAMPLE_COUNT`] on.
    pub sample_count: u32,
}

/// True while the switch is on. Pure: same config gives the same answer.
#[must_use]
pub fn msaa_enabled(config: &MsaaConfig) -> bool {
    config.enabled
}

/// Sample count for a config: 4 on, 1 off. Pure: same inputs give the same
/// count twice.
#[must_use]
pub fn msaa_sample_count(config: &MsaaConfig) -> u32 {
    if config.enabled {
        MSAA_SAMPLE_COUNT
    } else {
        // Off is the single-sample path: the same 1x shape as the resolve
        // target, so the off count names it instead of repeating the literal.
        RESOLVE_SAMPLE_COUNT
    }
}

/// Forward attachment decision for a config plus the built flag: on plus
/// built gives `(4, true)` (4x views, resolve into 1x HDR), anything else
/// gives `(1, false)` (today's 1x path).
///
/// Owner: render lane; Caller: `crates/render/src/present/gpu_present/submit.rs`
/// (`Presenter::encode_forward` resolve branch). Units: samples plus flag out.
/// Determinism: pure integer/bool logic; same inputs give same pair twice;
/// no allocation, no clock, no device, so headless tests run. Errors: none.
/// Budget: no allocation on any path.
#[must_use]
pub fn msaa_forward_choice(config: &MsaaConfig, targets_built: bool) -> (u32, bool) {
    if msaa_enabled(config) && targets_built {
        (MSAA_SAMPLE_COUNT, true)
    } else {
        (RESOLVE_SAMPLE_COUNT, false)
    }
}

/// Target shape for a config: `HDR_FORMAT` plus `DEPTH_FORMAT` with the
/// chosen sample count. Read-only: it names the formats, never copies the
/// target-build body. Donor shape is pipeline multisample count 4 plus
/// target sampleCount 4 plus the view/resolveTarget pair (the 4x view
/// resolves into the 1x HDR target before the blit); the sibling
/// presenter-hunk slice builds those textures from [`attachment_plan`].
#[must_use]
pub fn msaa_target_desc(config: &MsaaConfig) -> MsaaTargetDesc {
    use super::{DEPTH_FORMAT, HDR_FORMAT};
    // Single source of truth: the plan names the sample counts, so the desc
    // can never disagree with the 4x textures the sibling slice builds from
    // it. The asserts tie the pipeline state and the resolve flag to the same
    // config on every build of this desc.
    let plan: AttachmentPlan = attachment_plan(config);
    debug_assert_eq!(plan.color_samples, msaa_sample_count(config));
    debug_assert_eq!(plan.resolve, needs_resolve(config));
    debug_assert_eq!(pipeline_multisample(config).count, plan.color_samples);
    debug_assert!(
        msaa_transient_within_cap(1920, 1080, config),
        "1080p transient stays under the cap"
    );
    MsaaTargetDesc {
        hdr_format: HDR_FORMAT,
        depth_format: DEPTH_FORMAT,
        sample_count: plan.color_samples,
    }
}

/// True when the `on` edge texel set carries MSAA blending and the `off`
/// set stays flat: `on` holds a value strictly between 0 and 1 while `off`
/// holds none. Pure CPU helper for the edge test only: same slices give the
/// same answer twice.
#[must_use]
pub fn edge_has_intermediate(on_texels: &[f32], off_texels: &[f32]) -> bool {
    fn has_mid(texels: &[f32]) -> bool {
        texels.iter().any(|v| *v > 0.0 && *v < 1.0)
    }
    has_mid(on_texels) && !has_mid(off_texels)
}

/// Transient bytes for the HDR plus depth targets at `width` x `height`
/// under `config`: color samples times 8 bytes per `Rgba16Float` texel plus
/// depth samples times 4 bytes per `Depth32Float` texel, both from the
/// [`attachment_plan`].
///
/// Owner: render lane; Caller: `crates/render/src/present/gpu_present/presenter.rs`
/// (`Presenter::resize` budget debug_assert). Units: pixels in, bytes out.
/// Determinism: pure integer math with saturation; same inputs give the same
/// bytes on every run; no allocation, no clock, no device, so headless tests
/// run. Errors: none; zero area sizes 0 (suspended path keeps last good).
/// Budget: 1x 1080p is 24,883,200 bytes, 4x is 99,532,800, both under
/// [`MSAA_TRANSIENT_CAP_BYTES`].
#[must_use]
pub fn msaa_transient_bytes(width: u32, height: u32, config: &MsaaConfig) -> u64 {
    if width == 0 || height == 0 {
        return 0;
    }
    let plan = attachment_plan(config);
    let px = u64::from(width).saturating_mul(u64::from(height));
    let color = px
        .saturating_mul(u64::from(plan.color_samples))
        .saturating_mul(8);
    let depth = px
        .saturating_mul(u64::from(plan.depth_samples))
        .saturating_mul(4);
    color.saturating_add(depth)
}

/// Cap for the MSAA transient while on: 128 MiB covers 4x 1080p
/// (99,532,800 bytes) with headroom; the presenter asserts it in debug.
pub const MSAA_TRANSIENT_CAP_BYTES: u64 = 128 * 1024 * 1024;

/// True while the HDR plus depth transient for `width` x `height` under
/// `config` stays within [`MSAA_TRANSIENT_CAP_BYTES`]. Pure wrapper over
/// [`msaa_transient_bytes`] for the live resize budget check.
/// Owner: render lane; Caller: `msaa_target_desc` (same file, 1080p cap
/// debug_assert) plus `crates/render/src/present/gpu_present/presenter.rs`
/// (`Presenter::resize` budget debug_assert, sibling slice wires it next).
/// Units: pixels in, boolean out.
/// Determinism: same inputs give the same verdict; no allocation, no clock,
/// no device, so headless tests run. Presentation only, never hashed.
#[must_use]
pub fn msaa_transient_within_cap(width: u32, height: u32, config: &MsaaConfig) -> bool {
    msaa_transient_bytes(width, height, config) <= MSAA_TRANSIENT_CAP_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msaa_off_is_1x_and_on_is_4x_with_edge() {
        let off = MsaaConfig::default();
        assert!(!msaa_enabled(&off), "switch defaults off");
        assert_eq!(msaa_sample_count(&off), 1, "off stays the 1x path");
        let on = MsaaConfig { enabled: true };
        assert!(msaa_enabled(&on), "setter state reads back on");
        assert_eq!(msaa_sample_count(&on), 4, "on picks 4x");
        assert_eq!(msaa_sample_count(&on), MSAA_SAMPLE_COUNT);
        // Spike fixture: two small CPU edge texel sets, one mixed with a
        // strict 0-1 interior value and one flat with only 0/1.
        let mixed = [0.0f32, 0.25, 0.5, 1.0];
        let flat = [0.0f32, 1.0, 1.0, 0.0];
        assert!(
            edge_has_intermediate(&mixed, &flat),
            "mixed on-set blends, flat off-set stays stepped"
        );
        assert!(
            !edge_has_intermediate(&flat, &flat),
            "flat on-set blends nothing"
        );
        // Repeat run is equal: same inputs give same outputs twice.
        assert_eq!(msaa_sample_count(&on), msaa_sample_count(&on));
        assert_eq!(
            edge_has_intermediate(&mixed, &flat),
            edge_has_intermediate(&mixed, &flat)
        );
        // The desc names the HDR plus depth formats with the chosen count.
        let desc = msaa_target_desc(&on);
        assert_eq!(desc.sample_count, MSAA_SAMPLE_COUNT);
        assert_eq!(msaa_target_desc(&off).sample_count, 1);
    }

    #[test]
    fn msaa_desc_names_formats_stable_x2() {
        use super::super::{DEPTH_FORMAT, HDR_FORMAT};
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        let off_desc = msaa_target_desc(&off);
        let on_desc = msaa_target_desc(&on);
        assert_eq!(off_desc.hdr_format, HDR_FORMAT, "off names the HDR target");
        assert_eq!(
            off_desc.depth_format, DEPTH_FORMAT,
            "off names the depth target"
        );
        assert_eq!(on_desc.hdr_format, HDR_FORMAT, "on keeps the HDR target");
        assert_eq!(
            on_desc.depth_format, DEPTH_FORMAT,
            "on keeps the depth target"
        );
        assert_eq!(off_desc.sample_count, 1, "off stays the 1x path");
        assert_eq!(on_desc.sample_count, MSAA_SAMPLE_COUNT, "on picks 4x");
        assert_eq!(
            off_desc,
            msaa_target_desc(&off),
            "same inputs give same desc twice"
        );
        assert_eq!(
            on_desc,
            msaa_target_desc(&on),
            "same inputs give same desc twice"
        );
        assert!(!edge_has_intermediate(&[], &[]), "empty sets blend nothing");
    }

    #[test]
    fn msaa_transient_bytes_1080p_is_24mb_off_and_99mb_on() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        assert_eq!(
            msaa_transient_bytes(1920, 1080, &off),
            24_883_200,
            "1x 1080p transient is HDR 16.6MB plus depth 8.3MB"
        );
        assert_eq!(
            msaa_transient_bytes(1920, 1080, &on),
            99_532_800,
            "4x 1080p transient quadruples both targets"
        );
        assert_eq!(
            msaa_transient_bytes(1920, 1080, &on),
            msaa_transient_bytes(1920, 1080, &on),
            "same inputs give same bytes twice"
        );
        assert_eq!(
            msaa_transient_bytes(0, 1080, &on),
            0,
            "zero area sizes nothing"
        );
        assert_eq!(
            msaa_transient_bytes(0, 0, &off),
            0,
            "suspended path sizes nothing"
        );
    }

    #[test]
    fn msaa_transient_within_cap_1080p() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        assert!(
            msaa_transient_within_cap(1920, 1080, &off),
            "1x 1080p stays under the cap"
        );
        assert!(
            msaa_transient_within_cap(1920, 1080, &on),
            "4x 1080p stays under the cap"
        );
        assert!(
            msaa_transient_within_cap(0, 0, &off),
            "suspended path stays under the cap"
        );
        assert!(
            !msaa_transient_within_cap(16_384, 16_384, &on),
            "huge 4x exceeds the cap"
        );
        assert_eq!(
            msaa_transient_within_cap(1920, 1080, &on),
            msaa_transient_within_cap(1920, 1080, &on),
            "same inputs give same verdict twice"
        );
    }

    #[test]
    fn msaa_edge_intermediate_on_only() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        assert_eq!(
            msaa_target_desc(&off).sample_count,
            1,
            "off stays the 1x path"
        );
        assert_eq!(
            msaa_target_desc(&on).sample_count,
            MSAA_SAMPLE_COUNT,
            "on picks the 4x pair"
        );
        assert!(!needs_resolve(&off), "off resolves nothing");
        assert!(needs_resolve(&on), "on resolves before the blit");
        assert_eq!(
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            0.5,
            "half-covered edge resolves to a half"
        );
        assert!(
            resolved_is_intermediate([0.0, 0.0, 1.0, 1.0]),
            "edge quad blends"
        );
        assert!(!resolved_is_intermediate([0.0; 4]), "flat 0 holds");
        assert!(!resolved_is_intermediate([1.0; 4]), "flat 1 holds");
        let on_tex = [resolve_average([0.0, 0.0, 1.0, 1.0])];
        let off_tex = [resolve_average([1.0; 4])];
        assert!(
            edge_has_intermediate(&on_tex, &off_tex),
            "resolved edge shows an intermediate on value only"
        );
        assert!(
            !edge_has_intermediate(&off_tex, &off_tex),
            "flat off-set stays stepped"
        );
        assert_eq!(
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            "same inputs give same value twice"
        );
        assert_eq!(
            msaa_target_desc(&on),
            msaa_target_desc(&on),
            "same inputs give same desc twice"
        );
    }

    #[test]
    fn msaa_forward_choice_resolves_only_when_on() {
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        assert_eq!(
            msaa_forward_choice(&on, true),
            (4, true),
            "on plus built resolves 4x into 1x HDR"
        );
        assert_eq!(
            msaa_forward_choice(&on, false),
            (1, false),
            "on plus unbuilt stays 1x"
        );
        assert_eq!(
            msaa_forward_choice(&off, true),
            (1, false),
            "off plus built stays 1x"
        );
        assert_eq!(
            msaa_forward_choice(&off, false),
            (1, false),
            "off plus unbuilt stays 1x"
        );
        assert_eq!(
            msaa_forward_choice(&on, true),
            msaa_forward_choice(&on, true),
            "same inputs give same choice twice"
        );
        assert_eq!(
            msaa_forward_choice(&off, false),
            msaa_forward_choice(&off, false),
            "same inputs give same choice twice"
        );
    }
}
