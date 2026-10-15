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
    include_bytes!("../../../../content/incoming/studio_small_09_1k.hdr");

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

mod frame;
mod live;
mod presenter;
mod submit;
mod timing;

pub use frame::{FrameOutput, FrameReceipt, LookPassReceipt};
pub use presenter::Presenter;
pub use submit::create_surface;
