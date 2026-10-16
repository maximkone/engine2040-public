//! frame: presented-frame types plus look-error mappers (window build).
//!
//! `FrameReceipt`/`FrameOutput` quote cache budgets and look passes per
//! frame; `GpuMesh`/`GpuTexture`/`Targets` back the presenter caches.
use super::super::*;
use crate::v1::Manifest;
use crate::{decals, gtao, ibl, material_layer, particles_gpu, sky};

/// One presented (or skipped) frame. The cache-budget rows quote the
/// mesh/texture budgets at frame end: resident counts, high-water marks,
/// cumulative evictions and the most recently evicted asset id, so an
/// overflow fixture proves eviction without error and names its assets.
/// The look rows quote the four Tier A look passes (sky, material
/// layers, particles, decals) the forward path recorded before the PBR
/// draws: `look_passes` is 4 on every capture frame and on probe frames
/// (`Presenter::set_look_probes`), 0 on live window frames and while
/// suspended. The IBL rows quote the material-chart pass sampling the
/// init-time bake after the forward draws: `ibl_passes` is 1 on those same
/// frames, else 0. The GTAO rows quote the occlusion passes (half-res
/// plus bilateral upsample) the post-forward tranche ran on this
/// frame's depth feed; live window frames run none (their fog is a
/// per-fragment function of the forward shader).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameReceipt {
    /// Frame width in pixels (last-good size while suspended).
    pub width: u32,
    /// Frame height in pixels (last-good size while suspended).
    pub height: u32,
    /// Draws submitted this frame.
    pub draws: u32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Successful reconfigures so far.
    pub generation: u64,
    /// True when pixels reached a target or the surface.
    pub presented: bool,
    /// Adapter name behind this frame.
    pub adapter: String,
    /// Driver string behind this frame.
    pub driver: String,
    /// Resident meshes at frame end.
    pub mesh_count: u32,
    /// Resident textures at frame end.
    pub texture_count: u32,
    /// Most meshes ever resident at once.
    pub mesh_high_water: u32,
    /// Most textures ever resident at once.
    pub texture_high_water: u32,
    /// Cumulative mesh evictions (capacity plus manual drops).
    pub mesh_evicted: u64,
    /// Cumulative texture evictions (capacity plus manual drops).
    pub texture_evicted: u64,
    /// Most recently evicted mesh asset id, if any.
    pub evicted_mesh_id: Option<u64>,
    /// Most recently evicted texture asset id, if any.
    pub evicted_texture_id: Option<u64>,
    /// Look passes recorded before the forward draws (4 live, 0 held).
    pub look_passes: u32,
    /// Look validation errors summed over the four passes; green is 0.
    pub look_validation_errors: u32,
    /// Chart passes recorded after the forward draws (1 live, 0 held).
    pub ibl_passes: u32,
    /// Chart validation errors; green is 0.
    pub ibl_validation_errors: u32,
    /// RL9 fog passes recorded after the forward pass from this frame's
    /// depth feed (2 live: height fog plus contact; 0 held, off or
    /// over-budget).
    pub fog_passes: u32,
    /// RL9 fog validation errors over both passes; green is 0.
    pub fog_validation_errors: u32,
    /// RL9 fog submit wall time in us (evidence vs FOG+CONTACT bars).
    pub fog_submit_us: u64,
    /// GTAO passes recorded after the forward pass from this
    /// frame's depth feed (2 live: half-res plus bilateral upsample;
    /// 0 held, off or over-budget).
    pub gtao_passes: u32,
    /// GTAO validation errors over both passes; green is 0.
    pub gtao_validation_errors: u32,
    /// GTAO submit wall time in us (evidence vs the 0.8 ms bar).
    pub gtao_submit_us: u64,
    /// Reporting seam: per-pass wall-clock evidence in us.
    /// The look tranche (sky, layers, particles, decals) plus the chart
    /// report here so the window binary can post per-pass ms
    /// without new passes or budget changes. Evidence only, never hashed.
    /// Sky submit wall time in us.
    pub sky_submit_us: u64,
    /// Layer chart submit wall time in us.
    pub layer_submit_us: u64,
    /// Particle bind submit wall time in us.
    pub particle_submit_us: u64,
    /// Decal bind submit wall time in us.
    pub decal_submit_us: u64,
    /// Chart submit wall time in us.
    pub ibl_submit_us: u64,
}

impl FrameReceipt {
    /// True when the frame observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }

    /// One-line budget telemetry naming the evicted assets, for the
    /// observe span the frame loop posts. Cold path only (receipts),
    /// never per-draw.
    pub fn cache_summary(&self) -> String {
        format!(
            "meshes {}/hw{} ev{} last{:?} textures {}/hw{} ev{} last{:?}",
            self.mesh_count,
            self.mesh_high_water,
            self.mesh_evicted,
            self.evicted_mesh_id,
            self.texture_count,
            self.texture_high_water,
            self.texture_evicted,
            self.evicted_texture_id,
        )
    }

    /// Total validation errors across the frame plus every pass tranche;
    /// green means 0. The window binary posts this beside per-pass ms.
    #[must_use]
    pub fn total_validation_errors(&self) -> u32 {
        self.validation_errors
            .saturating_add(self.look_validation_errors)
            .saturating_add(self.ibl_validation_errors)
            .saturating_add(self.fog_validation_errors)
            .saturating_add(self.gtao_validation_errors)
    }

    /// Per-pass wall-clock in report order: sky, layers,
    /// particles, decals, chart, fog, occlusion. Evidence only, never
    /// hashed; micros with no allocation.
    #[must_use]
    pub fn per_pass_us(&self) -> [(&'static str, u64); 7] {
        [
            ("sky", self.sky_submit_us),
            ("layers", self.layer_submit_us),
            ("particles", self.particle_submit_us),
            ("decals", self.decal_submit_us),
            ("ibl", self.ibl_submit_us),
            ("fog", self.fog_submit_us),
            ("gtao", self.gtao_submit_us),
        ]
    }

    /// Format micros as millis with three decimals using integer math,
    /// so the receipt path needs no float cast.
    fn format_ms_us(us: u64) -> String {
        format!("{}.{:03}", us / 1000, us % 1000)
    }

    /// One-line per-pass table for capture logs: adapter plus driver
    /// plus resolution plus each pass ms plus total validation errors.
    /// Fixed field order, cold receipt path only.
    #[must_use]
    pub fn per_pass_summary(&self) -> String {
        format!(
            "adapter={} driver={} resolution={}x{} sky={}ms layers={}ms particles={}ms decals={}ms ibl={}ms fog={}ms gtao={}ms validation_errors={}",
            self.adapter,
            self.driver,
            self.width,
            self.height,
            Self::format_ms_us(self.sky_submit_us),
            Self::format_ms_us(self.layer_submit_us),
            Self::format_ms_us(self.particle_submit_us),
            Self::format_ms_us(self.decal_submit_us),
            Self::format_ms_us(self.ibl_submit_us),
            Self::format_ms_us(self.fog_submit_us),
            Self::format_ms_us(self.gtao_submit_us),
            self.total_validation_errors(),
        )
    }
}

/// Offscreen frame output: PNG bytes (absent while suspended) plus the
/// adapter/driver manifest and the frame receipt.
#[derive(Debug)]
pub struct FrameOutput {
    /// PNG bytes; `None` while suspended (nothing drawn).
    pub png: Option<Vec<u8>>,
    /// Adapter, driver, size and validation-error count.
    pub manifest: Manifest,
    /// Draw count, generation and presented flag.
    pub receipt: FrameReceipt,
}

pub(crate) struct GpuMesh {
    /// Interleaved vertex buffer (`COPY_DST`: a dynamic mesh rewrites it
    /// in place every frame instead of making a new one).
    pub(crate) vertex: wgpu::Buffer,
    pub(crate) index: wgpu::Buffer,
    /// Index count.
    pub(crate) count: u32,
    /// Vertex count the buffer holds (fixed for the buffer's life).
    pub(crate) vertices: u32,
}

pub(crate) struct GpuTexture {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

pub(crate) struct Targets {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) hdr_view: wgpu::TextureView,
    /// Linear HDR texture behind `hdr_view` (Rgba16Float); tests read the
    /// linear values back.
    #[cfg(test)]
    pub(crate) hdr_texture: wgpu::Texture,
    pub(crate) out_view: wgpu::TextureView,
    pub(crate) out_texture: wgpu::Texture,
    pub(crate) depth_view: wgpu::TextureView,
    /// Depth texture behind `depth_view`; RL9 copies it per frame.
    pub(crate) depth_texture: wgpu::Texture,
    /// RL9 fog-factor texture the blit composites (R32 float).
    pub(crate) fog_texture: wgpu::Texture,
    /// RL9 contact-AO texture the blit multiplies (R32 float).
    pub(crate) ao_texture: wgpu::Texture,
    /// RL9 fog composite color (sRGB) plus frame size for texel lookup.
    pub(crate) fog_col_buf: wgpu::Buffer,
    /// Blit bind group over `hdr_view` plus the RL9 pair; rebuilt with
    /// the targets.
    pub(crate) blit_bind: wgpu::BindGroup,
}

impl From<&wgpu::CurrentSurfaceTexture> for SurfaceEvent {
    fn from(current: &wgpu::CurrentSurfaceTexture) -> Self {
        match current {
            wgpu::CurrentSurfaceTexture::Success(_) => Self::AcquiredClean,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) => Self::AcquiredSuboptimal,
            wgpu::CurrentSurfaceTexture::Outdated => Self::Outdated,
            wgpu::CurrentSurfaceTexture::Lost => Self::Lost,
            wgpu::CurrentSurfaceTexture::Timeout => Self::Timeout,
            wgpu::CurrentSurfaceTexture::Occluded => Self::Occluded,
            wgpu::CurrentSurfaceTexture::Validation => Self::ValidationError,
        }
    }
}

/// One forward-path look tranche: the four Tier A look passes (RL4 sky,
/// RL6 layers, RP2 particles, RP3 decals) recorded under one shared
/// [`gtao::LookFrame`] bind before the PBR draws, plus the RL9 fog
/// tranche (height fog plus contact from this frame's depth feed) after
/// the forward pass whose pair the blit composites. Units: counts plus
/// per-pass wall-clock ms (evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookPassReceipt {
    /// Look passes recorded (4 on every live frame).
    pub passes: u32,
    /// Validation errors summed over the four passes; green means 0.
    pub validation_errors: u32,
    /// RL4 sky submit ms.
    pub sky_ms: f64,
    /// RL6 layer chart submit ms.
    pub layer_ms: f64,
    /// RP2 particle bind submit ms.
    pub particle_ms: f64,
    /// RP3 decal bind submit ms.
    pub decal_ms: f64,
}

impl LookPassReceipt {
    /// A frame that skipped the look probes: zero passes, zero errors.
    #[must_use]
    pub const fn skipped() -> Self {
        Self {
            passes: 0,
            validation_errors: 0,
            sky_ms: 0.0,
            layer_ms: 0.0,
            particle_ms: 0.0,
            decal_ms: 0.0,
        }
    }
}

/// Map a sky failure into the presenter error (window build).
pub(super) fn sky_err(e: sky::SkyError) -> PresentError {
    match e {
        sky::SkyError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("sky look: {reason}"),
        },
        sky::SkyError::NoDevice(reason) => PresentError::Device(format!("sky look: {reason}")),
        sky::SkyError::Device(reason) => PresentError::Device(format!("sky look: {reason}")),
        sky::SkyError::Validation(reason) => {
            PresentError::Validation(format!("sky look: {reason}"))
        }
    }
}

/// Map a material-layer failure into the presenter error (window build).
pub(super) fn layer_err(e: material_layer::MaterialError) -> PresentError {
    match e {
        material_layer::MaterialError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("layer look: {reason}"),
        },
        material_layer::MaterialError::NoDevice(reason) => {
            PresentError::Device(format!("layer look: {reason}"))
        }
        material_layer::MaterialError::Device(reason) => {
            PresentError::Device(format!("layer look: {reason}"))
        }
        material_layer::MaterialError::Validation(reason) => {
            PresentError::Validation(format!("layer look: {reason}"))
        }
    }
}

/// Map a particle failure into the presenter error (window build).
pub(super) fn particle_err(e: particles_gpu::ParticleError) -> PresentError {
    match e {
        particles_gpu::ParticleError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("particle look: {reason}"),
        },
        particles_gpu::ParticleError::BudgetExceeded { requested, allowed } => {
            PresentError::InvalidDraw {
                index: 0,
                reason: format!("particle look budget: {requested} over {allowed}"),
            }
        }
        particles_gpu::ParticleError::NoDevice(reason) => {
            PresentError::Device(format!("particle look: {reason}"))
        }
        particles_gpu::ParticleError::Device(reason) => {
            PresentError::Device(format!("particle look: {reason}"))
        }
        particles_gpu::ParticleError::Validation(reason) => {
            PresentError::Validation(format!("particle look: {reason}"))
        }
    }
}

/// Map a decal failure into the presenter error (window build).
pub(super) fn decal_err(e: decals::DecalError) -> PresentError {
    match e {
        decals::DecalError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("decal look: {reason}"),
        },
        decals::DecalError::BudgetExceeded { requested, allowed } => PresentError::InvalidDraw {
            index: 0,
            reason: format!("decal look budget: {requested} over {allowed}"),
        },
        decals::DecalError::NoDevice(reason) => {
            PresentError::Device(format!("decal look: {reason}"))
        }
        decals::DecalError::Device(reason) => PresentError::Device(format!("decal look: {reason}")),
        decals::DecalError::Validation(reason) => {
            PresentError::Validation(format!("decal look: {reason}"))
        }
    }
}

/// Map a GTAO failure into the presenter error (window build).
pub(super) fn gtao_err(e: gtao::LookError) -> PresentError {
    match e {
        gtao::LookError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("gtao look: {reason}"),
        },
        gtao::LookError::NoDevice(reason) => PresentError::Device(format!("gtao look: {reason}")),
        gtao::LookError::Device(reason) => PresentError::Device(format!("gtao look: {reason}")),
    }
}

/// Map an IBL failure into the presenter error (window build).
pub(super) fn ibl_err(e: ibl::IblError) -> PresentError {
    match e {
        ibl::IblError::Invalid(reason) => PresentError::InvalidDraw {
            index: 0,
            reason: format!("ibl look: {reason}"),
        },
        ibl::IblError::MissingHdri => {
            PresentError::Device("ibl look: no bake bound at submit".into())
        }
        ibl::IblError::Hdri(reason) => PresentError::Device(format!("ibl look hdri: {reason}")),
        ibl::IblError::Sample(reason) => PresentError::Device(format!("ibl look sample: {reason}")),
        ibl::IblError::Device(reason) => PresentError::Device(format!("ibl look: {reason}")),
        ibl::IblError::Validation(reason) => {
            PresentError::Validation(format!("ibl look: {reason}"))
        }
    }
}
