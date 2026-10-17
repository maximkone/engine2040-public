//! gpu_present: windowed presenter internals (RA1), behind `window`.
//!
//! Owner: render lane, consumer WIN1 window loop and ED1 viewport next.
//! Frame types live in `frame`, the presenter core in `presenter`, the
//! submit/encode path plus free helpers in `submit`; this root only holds
//! the shared target constants and re-exports the presenter API.
// ---------------------------------------------------------------------------
// GPU presenter behind `window`: HDR plus depth plus LDR-twin targets, mesh
// and texture caches, surface/swapchain ownership with last-good recovery.
// ---------------------------------------------------------------------------
/// Studio HDRI baked once into the presenter's IBL state: the same
/// fixture bytes the cook reference bakes, so the per-frame chart samples
/// a fixed bake. Embedded (1.6 MB) to keep the window build
/// self-contained and deterministic; the steady-state path never cooks.
const IBL_STUDIO_HDRI_BYTES: &[u8] =
    include_bytes!("../../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

/// HDR working target: half float, LINEAR radiance. The forward pass writes
/// exposed scene radiance (values above 1.0 survive); the blit does the
/// frame's only tone map, ACES plus sRGB.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// LDR swapchain twin for offscreen readback and PNG receipts.
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Depth target for the forward pass.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Forward pass clear, linear radiance. It tone-maps to the display color
/// (7.65, 8.9, 15.3) of 255 that the display-space clear of the earlier
/// frame had (solved through ACES plus sRGB), so an empty capture keeps its
/// background. The live window frame draws the sky over it.
pub const HDR_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.01607,
    g: 0.01783,
    b: 0.02701,
    a: 1.0,
};
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
const ROW_ALIGN: u32 = 256;

/// Blit shader: `shaders/blit.wgsl` with the display transform
/// (`shaders/tonemap.wgsl`) prepended at compile time. Samples the linear
/// HDR target into the LDR twin or surface view, composites the probe fog
/// and contact AO pair on linear radiance, then does the frame's one tone
/// map. Entry points `fs` (encodes sRGB itself) and `fs_hw_srgb` (for a
/// `*Srgb` surface view, which encodes on write).
pub(super) const BLIT_WGSL: &str = concat!(
    include_str!("../shaders/tonemap.wgsl"),
    include_str!("../shaders/blit.wgsl")
);

mod bind_memo;
mod capture;
mod fog_tranche;
mod forward_bind;
mod frame;
mod gate;
mod gtao_live;
mod live;
mod msaa;
mod presenter;
#[cfg(any(target_arch = "wasm32", test))]
mod scope_ledger;
mod submit;
mod sun_shadow;
mod timing;

pub use capture::{CaptureStats, FrameCapture, DISTINCT_CAP};
pub use frame::{
    last_live_wall_split, FrameOutput, FrameReceipt, LivePixels, LiveWallSplit, LookPassReceipt,
};
pub use msaa::{
    edge_has_intermediate, msaa_enabled, msaa_sample_count, msaa_target_desc, MsaaConfig,
    MsaaTargetDesc, MSAA_SAMPLE_COUNT,
};
pub use presenter::Presenter;
pub use submit::create_surface;

#[cfg(test)]
mod root_wiring_tests {
    //! Crate-root wiring vectors: the window loop reaches every shared
    //! target through this root (`HDR_FORMAT`, `OUTPUT_FORMAT`,
    //! `DEPTH_FORMAT`, `HDR_CLEAR`, the presenter plus frame re-exports).
    //!
    //! Consumer: WIN1 window loop plus ED1 viewport next, via
    //! `crate::present::gpu_present::{HDR_FORMAT, Presenter, ...}`.
    //! Units: GPU formats plus linear radiance clears. Determinism: pure
    //! constants plus string contains, no device, no clock.
    //!
    //! Why this file exists: `crates/render/src/present/gpu_present.rs`
    //! changed 4 times in 30 days yet held zero `#[test]` and no
    //! `tests/it` module named it, so a dropped `pub use` or a format
    //! swap would compile the crate but break every caller at their
    //! `use` line. Each vector below names the root path, so removing
    //! or retargeting one fails here first.

    use super::*;

    /// The window loop's shared targets stay half-float linear HDR plus
    /// 8-bit LDR twin plus 32-bit depth, with a 256-byte row pitch and a
    /// non-empty studio bake behind the IBL chart.
    #[test]
    fn window_targets_keep_hdr_contract_through_root() {
        assert_eq!(
            HDR_FORMAT,
            wgpu::TextureFormat::Rgba16Float,
            "forward HDR stays half float via root"
        );
        assert_eq!(
            OUTPUT_FORMAT,
            wgpu::TextureFormat::Rgba8Unorm,
            "LDR twin stays 8-bit via root"
        );
        assert_eq!(
            DEPTH_FORMAT,
            wgpu::TextureFormat::Depth32Float,
            "depth stays 32-bit float via root"
        );
        assert_ne!(HDR_FORMAT, OUTPUT_FORMAT, "HDR and LDR never alias");
        assert_ne!(HDR_FORMAT, DEPTH_FORMAT, "color and depth never alias");
        assert_eq!(ROW_ALIGN, 256, "copy rows stay 256-byte aligned");
        assert_eq!(HDR_CLEAR.a, 1.0, "clear stays opaque via root");
        for (got, name) in [(HDR_CLEAR.r, "r"), (HDR_CLEAR.g, "g"), (HDR_CLEAR.b, "b")] {
            assert!(
                got > 0.0 && got < 0.1,
                "clear {name} stays dim linear radiance, got {got}"
            );
        }
        assert!(
            !IBL_STUDIO_HDRI_BYTES.is_empty(),
            "studio bake stays embedded via root"
        );
        assert!(
            IBL_STUDIO_HDRI_BYTES.len() > 1_000,
            "studio bake stays a real HDRI, got {} bytes",
            IBL_STUDIO_HDRI_BYTES.len()
        );
    }

    /// The blit carries the frame's one tone map and the root re-exports
    /// keep the window loop's `use` lines compiling.
    #[test]
    fn blit_and_reexports_stay_wired_for_window_loop() {
        assert!(
            BLIT_WGSL.contains("fn aces_fitted("),
            "blit keeps the display transform via root"
        );
        assert!(
            BLIT_WGSL.contains("fn fs("),
            "blit keeps the LDR-twin entry via root"
        );
        assert!(
            BLIT_WGSL.contains("fn fs_hw_srgb("),
            "blit keeps the hardware-sRGB entry via root"
        );
        // Re-exports the window loop names: mentioning each type here
        // fails at the `use` line when the `pub use` above is dropped.
        assert!(
            core::any::type_name::<Presenter>().contains("Presenter"),
            "presenter stays re-exported via root"
        );
        assert!(
            core::any::type_name::<FrameOutput>().contains("FrameOutput"),
            "frame output stays re-exported via root"
        );
        assert!(
            core::any::type_name::<MsaaConfig>().contains("MsaaConfig"),
            "MSAA config stays re-exported via root"
        );
        let _ = super::create_surface;
    }
}
