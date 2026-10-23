//! frame: presented-frame types plus look-error mappers (window build).
//!
//! `FrameReceipt`/`FrameOutput` quote cache budgets and look passes per
//! frame; `GpuMesh`/`GpuTexture`/`Targets` back the presenter caches.
use super::super::*;
use crate::v1::Manifest;
use crate::{decals, gtao, ibl, material_layer, particles_gpu, sky};

/// Second forward attachment for draw ids: one `u32` per pixel, sized
/// with the frame. The shader slice writes it; this slice only proves
/// the target exists and stays clear.
pub(crate) const ID_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// Clear value for the id target: no draw claims this texel.
pub(crate) const ID_CLEAR: u32 = crate::pass_truth::live_gate::NO_DRAW_ID;

/// Clear bytes for the id target: `width * height` copies of
/// [`ID_CLEAR`] in little-endian row-major order. Returns empty on
/// zero size. Pure: same inputs give same bytes twice. No GPU here.
pub(crate) fn id_clear_bytes(width: u32, height: u32) -> Vec<u8> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let w = usize::try_from(width).unwrap_or(0);
    let h = usize::try_from(height).unwrap_or(0);
    let len = w.checked_mul(h).unwrap_or(0).checked_mul(4).unwrap_or(0);
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![0u8; len];
    let word = ID_CLEAR.to_le_bytes();
    for chunk in out.chunks_exact_mut(4) {
        chunk.copy_from_slice(&word);
    }
    out
}

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
    /// 0 held, off or over-budget). The blocking probe tranche and the
    /// stale reuse count here; the live frame's device pass (LF3) reports
    /// its time in `gtao_submit_us` alone, so a live frame reads 0 passes.
    pub gtao_passes: u32,
    /// GTAO validation errors over both passes; green is 0.
    pub gtao_validation_errors: u32,
    /// GTAO submit wall time in us (evidence vs the 0.8 ms bar). On a live
    /// frame: the device time of the newest resolved GTAO pass.
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
    /// Sun depth submit wall time in us.
    pub shadow_submit_us: u64,
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
    /// particles, decals, chart, fog, occlusion, shadow. Evidence only, never
    /// hashed; micros with no allocation.
    #[must_use]
    pub fn per_pass_us(&self) -> [(&'static str, u64); 8] {
        [
            ("sky", self.sky_submit_us),
            ("layers", self.layer_submit_us),
            ("particles", self.particle_submit_us),
            ("decals", self.decal_submit_us),
            ("ibl", self.ibl_submit_us),
            ("fog", self.fog_submit_us),
            ("gtao", self.gtao_submit_us),
            ("shadow", self.shadow_submit_us),
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
            "adapter={} driver={} resolution={}x{} sky={}ms layers={}ms particles={}ms decals={}ms ibl={}ms fog={}ms gtao={}ms shadow={}ms validation_errors={}",
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
            Self::format_ms_us(self.shadow_submit_us),
            self.total_validation_errors(),
        )
    }
}

/// Wall-clock split of one live window frame in microseconds. It names where
/// the CPU time of a present goes: the staging before any command exists, the
/// encode through `queue.submit`, and the `queue.present` call. Reported
/// beside [`FrameReceipt`], never inside it. Evidence only, never hashed.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct LiveWallSplit {
    /// Camera and draw pack, targets, uniforms, shadow bind, fog uniform and
    /// draw binds: everything before the encoder opens.
    pub stage_us: u64,
    /// Encoder create through `queue.submit` (forward, blit and overlay).
    pub submit_us: u64,
    /// The `queue.present` call; 0 on the offscreen twin, which has no
    /// swapchain image to present.
    pub present_call_us: u64,
}

impl LiveWallSplit {
    /// Sum of the three spans; never above the wall of the whole call.
    #[must_use]
    pub fn total_us(&self) -> u64 {
        self.stage_us
            .saturating_add(self.submit_us)
            .saturating_add(self.present_call_us)
    }

    /// `stage=..ms submit=..ms present=..ms`, the millisecond format of the
    /// per-pass line. Cold receipt path only.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "stage={}ms submit={}ms present={}ms",
            FrameReceipt::format_ms_us(self.stage_us),
            FrameReceipt::format_ms_us(self.submit_us),
            FrameReceipt::format_ms_us(self.present_call_us),
        )
    }
}

thread_local! {
    /// The last live window frame's split on the presenting thread. A frame
    /// is written and read on the thread that presents it, so two presenters
    /// on two threads never see each other's numbers.
    static LAST_LIVE_WALL: core::cell::Cell<LiveWallSplit> = const {
        core::cell::Cell::new(LiveWallSplit {
            stage_us: 0,
            submit_us: 0,
            present_call_us: 0,
        })
    };
}

/// Store the split of the live frame that just submitted (present call 0
/// until the surface path patches it).
pub(super) fn record_live_wall_split(split: LiveWallSplit) {
    LAST_LIVE_WALL.with(|cell| cell.set(split));
}

/// Patch the `queue.present` span into the split the same frame recorded.
pub(super) fn record_present_call_us(us: u64) {
    LAST_LIVE_WALL.with(|cell| {
        let mut split = cell.get();
        split.present_call_us = us;
        cell.set(split);
    });
}

/// The last live window frame's wall split on this thread; all zero before
/// the first live frame. Evidence only, never hashed.
#[must_use]
pub fn last_live_wall_split() -> LiveWallSplit {
    LAST_LIVE_WALL.with(core::cell::Cell::get)
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

/// Pixels of one window-path frame read back for the pass-truth gate: the
/// LDR twin as the blit wrote it plus the draw id of every pixel. A blocking
/// readback by design (a capture frame in the wait tally), never on the live
/// path. Evidence only, never hashed.
#[derive(Debug)]
pub struct LivePixels {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// RGBA8 row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Draw id per pixel, row-major; `NO_DRAW_ID` where no draw covers.
    pub ids: Vec<u32>,
    /// The receipt of the frame these pixels came from.
    pub receipt: FrameReceipt,
}

pub(crate) struct GpuMesh {
    /// Interleaved vertex buffer (`COPY_DST`: a dynamic mesh rewrites it
    /// in place every frame instead of making a new one).
    pub(crate) vertex: wgpu::Buffer,
    /// LF5: previous-frame positions, compact `vec3` rows (stride 12), the
    /// motion target's second vertex stream. `None` for a skinned mesh,
    /// whose previous pose comes from the previous joint palette instead.
    pub(crate) prev_vertex: Option<wgpu::Buffer>,
    /// CPU copy of the positions `vertex` last held: the next refresh
    /// publishes them into `prev_vertex`. Empty for a skinned mesh.
    pub(crate) last_positions: Vec<[f32; 3]>,
    pub(crate) index: wgpu::Buffer,
    /// Index count.
    pub(crate) count: u32,
    /// Vertex count the buffer holds (fixed for the buffer's life).
    pub(crate) vertices: u32,
    /// World-space bounding sphere `[x, y, z, radius]` of the positions the
    /// vertex buffer holds, refreshed with them. The forward stage draws
    /// positions verbatim, so this is the draw's world bound for the EG38
    /// submit cull.
    pub(crate) bounds: [f32; 4],
    /// CPU copy of the triangle list the index buffer holds: an in-place
    /// vertex refresh recomputes the tangents from it. Empty for a skinned
    /// mesh, which never refreshes in place.
    pub(crate) indices: Vec<u32>,
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
    /// LF3 landing 2: group-1 bind over `ao_texture`, the occlusion field
    /// the forward pass scales its ambient by. Rebuilt with the targets.
    pub(crate) ao_bind: wgpu::BindGroup,
    /// Live GTAO resources over this frame size (`None` when the kernel
    /// cannot size the frame): the half-res buffer, the uniform and the two
    /// bind groups over this target's depth and `ao_texture`.
    pub(crate) gtao: Option<super::gtao_live::GtaoSized>,
    /// Draw-id attachment (`R32Uint`): cleared to [`ID_CLEAR`] at target
    /// build; the forward pass loads it and writes each covered pixel's draw
    /// id. The pass-truth gate reads it back for its sky and geometry
    /// regions.
    pub(crate) id_texture: wgpu::Texture,
    /// View over `id_texture` for the forward and sky attachments.
    pub(crate) id_view: wgpu::TextureView,
    /// CPU image of the id clear (`ID_CLEAR` bytes), written back into the
    /// target before a pass-truth capture so its ids describe one frame.
    pub(crate) id_clear: Vec<u8>,
    /// LF5 uv-space motion (`Rg16Float`): the forward pass writes it beside
    /// HDR and id on the 1x path; the MSAA path clears it (no 4x twin).
    pub(crate) motion_view: wgpu::TextureView,
    /// Texture behind `motion_view` (COPY_SRC: tests copy it back). Kept
    /// beside the view so the pair dies together.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "owns the texture behind `motion_view`")
    )]
    pub(crate) motion_texture: wgpu::Texture,
    /// RL9 fog composite color (sRGB) plus frame size for texel lookup.
    pub(crate) fog_col_buf: wgpu::Buffer,
    /// Blit bind group over `hdr_view` plus the RL9 pair; rebuilt with
    /// the targets.
    pub(crate) blit_bind: wgpu::BindGroup,
    /// 4x MSAA color plus depth pair when on; `None` keeps the 1x path.
    pub(crate) msaa: Option<MsaaTargets>,
}

/// 4x MSAA color plus depth pair when the switch is on; `None` is the 1x
/// path. `samples` comes from the plan (4 on), color is HDR, depth is DEPTH.
pub(crate) struct MsaaTargets {
    /// Samples per pixel from the plan (4 when on).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the plan's sample count; only tests read it")
    )]
    pub(crate) samples: u32,
    /// Multisampled HDR color (RENDER_ATTACHMENT only).
    #[expect(dead_code, reason = "owns the texture behind `color_view`")]
    pub(crate) color: wgpu::Texture,
    /// View over `color` for the resolve slice.
    pub(crate) color_view: wgpu::TextureView,
    /// Multisampled depth (RENDER_ATTACHMENT only).
    #[expect(dead_code, reason = "owns the texture behind `depth_view`")]
    pub(crate) depth: wgpu::Texture,
    /// View over `depth` for the resolve slice.
    pub(crate) depth_view: wgpu::TextureView,
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

/// Map a weapon-trail stage failure into the presenter error (window build).
pub(super) fn trail_err(e: crate::trail_draw_gpu::TrailDrawError) -> PresentError {
    PresentError::InvalidDraw {
        index: 0,
        reason: format!("weapon trail: {e}"),
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

#[cfg(test)]
#[path = "frame_id_tests.rs"]
mod frame_id_tests;

#[cfg(test)]
mod id_tests {
    use super::frame_id_tests::read_id_back;
    use super::*;

    #[test]
    fn id_target_cleared_to_max() {
        assert_eq!(ID_CLEAR, u32::MAX);
        assert_eq!(ID_TARGET_FORMAT, wgpu::TextureFormat::R32Uint);
        let bytes = id_clear_bytes(2, 1);
        let word = ID_CLEAR.to_le_bytes();
        assert_eq!(&bytes[0..4], &word);
        assert_eq!(&bytes[4..8], &word);
        assert!(id_clear_bytes(0, 1).is_empty() && id_clear_bytes(1, 0).is_empty());
        assert_eq!(id_clear_bytes(2, 1), id_clear_bytes(2, 1));
    }

    #[test]
    fn id_target_stays_clear_on_offscreen() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(e) => panic!("presenter init failed: {e}"),
        };
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("id_target_stays_clear_on_offscreen", probe)
        else {
            return;
        };
        let key = crate::present::MeshKey::new(0x1D1D, [0x1D; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        // Resize drops targets (lazy rebuild on the next render), so
        // materialize the cold set before the clear readback.
        presenter.ensure_targets(64, 64);
        // Cold target starts clear before any forward write.
        let targets = presenter.targets.as_ref().expect("targets exist");
        assert_eq!(targets.id_texture.format(), ID_TARGET_FORMAT);
        let cold = read_id_back(&presenter, &targets.id_texture);
        assert!(cold.iter().all(|v| *v == ID_CLEAR));
        let scene = crate::v1::default_scene(64, 64).expect("default scene is valid");
        let list = crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("box frame renders");
        assert!(out.receipt.is_clean() && out.manifest.is_clean());
    }

    #[test]
    fn id_target_reports_box_id() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(e) => panic!("presenter init failed: {e}"),
        };
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("id_target_reports_box_id", probe)
        else {
            return;
        };
        let key = crate::present::MeshKey::new(0x1D1D, [0x1D; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let scene = crate::v1::default_scene(64, 64).expect("default scene is valid");
        let list = crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![crate::present::DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("box frame renders");
        assert!(out.receipt.is_clean() && out.manifest.is_clean());
        let targets = presenter.targets.as_ref().expect("targets exist");
        assert_eq!(targets.id_texture.format(), ID_TARGET_FORMAT);
        let first = read_id_back(&presenter, &targets.id_texture);
        assert_eq!(first, read_id_back(&presenter, &targets.id_texture));
        let covered = first.iter().filter(|v| **v == 0).count();
        let clear = first.iter().filter(|v| **v == ID_CLEAR).count();
        assert!(
            covered > 0,
            "box must claim pixels with id 0, got {covered} covered"
        );
        assert!(
            clear > 0,
            "background must stay clear {ID_CLEAR}, got {clear} clear"
        );
        assert_eq!(
            covered + clear,
            first.len(),
            "single box leaves only id 0 or clear"
        );
    }

    #[test]
    fn id_target_box_readback_holds_box_id_and_clear() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(e) => panic!("presenter init failed: {e}"),
        };
        let Some(mut presenter) = crate::test_gpu::require_or_skip(
            "id_target_box_readback_holds_box_id_and_clear",
            probe,
        ) else {
            return;
        };
        let box_key = crate::present::MeshKey::new(0xB0E1, [0xB0; 32]);
        let plane_key = crate::present::MeshKey::new(0x51A0, [0x51; 32]);
        let plane = crate::v1::MeshData {
            // Ground quad at y=-0.5, +Y normal. The winding is CCW seen
            // from above (same convention as `v1::cube`), so the top face
            // is front-facing for the down-looking default camera and the
            // back-face cull keeps it.
            positions: vec![
                [-16.0, -0.5, -16.0],
                [16.0, -0.5, -16.0],
                [16.0, -0.5, 16.0],
                [-16.0, -0.5, 16.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 2, 1, 0, 3, 2],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        };
        presenter
            .upload_mesh(box_key, &crate::v1::cube())
            .expect("box uploads");
        presenter
            .upload_mesh(plane_key, &plane)
            .expect("plane uploads");
        presenter.resize(64, 64).expect("size applies");
        let scene = crate::v1::default_scene(64, 64).expect("scene builds");
        let list = crate::present::DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![
                crate::present::DrawItem {
                    mesh: box_key,
                    material: scene.material.clone(),
                    base_color: None,
                    normal_map: None,
                    orm: None,
                    emissive: None,
                },
                crate::present::DrawItem {
                    mesh: plane_key,
                    material: scene.material,
                    base_color: None,
                    normal_map: None,
                    orm: None,
                    emissive: None,
                },
            ],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("box on plane renders");
        assert!(out.receipt.is_clean(), "receipt clean");
        assert!(out.manifest.is_clean(), "manifest clean");
        assert_eq!(out.receipt.validation_errors, 0, "0 validation errors");
        let targets = presenter.targets.as_ref().expect("targets exist");
        assert_eq!(targets.id_texture.format(), ID_TARGET_FORMAT);
        assert_eq!(targets.id_clear, id_clear_bytes(64, 64));
        let first = read_id_back(&presenter, &targets.id_texture);
        let second = read_id_back(&presenter, &targets.id_texture);
        assert_eq!(first, second, "id bytes x2 equal");
        let mut counts = vec![0u64; 2];
        let (tallied, skipped, box_hits, plane_hits, partitioned) =
            crate::draw_id::verify_box_plane_partition(&first, &mut counts);
        assert!(partitioned, "box plus plane partition through tally");
        assert!(box_hits > 0, "box claims pixels with id 0");
        assert!(plane_hits > 0, "plane keeps pixels with id 1");
        let box_direct = first.iter().filter(|v| **v == 0).count();
        let plane_direct = first.iter().filter(|v| **v == 1).count();
        let clear = first.iter().filter(|v| **v == ID_CLEAR).count();
        assert!(clear > 0, "background stays clear");
        assert_eq!(
            u64::try_from(box_direct).expect("box count fits"),
            box_hits,
            "oracle box matches direct id 0"
        );
        assert_eq!(
            u64::try_from(plane_direct).expect("plane count fits"),
            plane_hits,
            "oracle plane matches direct id 1"
        );
        assert_eq!(
            u64::try_from(clear).expect("clear count fits"),
            skipped,
            "clear texels are the skipped ones"
        );
        assert_eq!(
            box_hits.saturating_add(plane_hits),
            tallied,
            "tally holds box plus plane"
        );
        assert!(
            first.iter().all(|v| *v == 0 || *v == 1 || *v == ID_CLEAR),
            "only box, plane or clear appear"
        );
    }
}

#[cfg(test)]
mod msaa_targets_tests {
    use super::super::msaa::{
        attachment_plan, edge_has_intermediate, msaa_forward_choice, msaa_target_desc,
        needs_resolve, resolve_average, resolved_is_intermediate, MsaaConfig, MSAA_SAMPLE_COUNT,
    };

    #[test]
    fn msaa_targets_resolve_only_when_on() {
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
            attachment_plan(&on).depth_samples,
            MSAA_SAMPLE_COUNT,
            "depth grows with color"
        );
        assert_eq!(
            msaa_forward_choice(&on, true),
            (MSAA_SAMPLE_COUNT, true),
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
            resolve_average([0.0, 0.0, 1.0, 1.0]),
            0.5,
            "half-covered edge resolves to a half"
        );
        assert!(
            resolved_is_intermediate([0.0, 0.0, 1.0, 1.0]),
            "edge quad blends"
        );
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
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(crate::present::PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(error) => panic!("presenter init failed: {error}"),
        };
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("msaa_targets_resolve_only_when_on", probe)
        else {
            return;
        };
        presenter.resize(64, 64).expect("size applies");
        presenter.ensure_targets(64, 64);
        assert!(
            presenter
                .targets
                .as_ref()
                .expect("targets exist")
                .msaa
                .is_none(),
            "off keeps the 1x path"
        );
        presenter.set_msaa_enabled(true);
        assert!(presenter.targets.is_none(), "flip drops targets");
        presenter.ensure_targets(64, 64);
        let pair = presenter
            .targets
            .as_ref()
            .expect("targets exist")
            .msaa
            .as_ref()
            .expect("on builds the 4x pair");
        assert_eq!(pair.samples, MSAA_SAMPLE_COUNT, "pair names 4 samples");
        assert!(
            msaa_forward_choice(&on, true).1,
            "built on path resolves before the blit"
        );
        presenter.set_msaa_enabled(false);
        assert!(presenter.targets.is_none(), "flip back drops targets");
        presenter.ensure_targets(64, 64);
        assert!(
            presenter
                .targets
                .as_ref()
                .expect("targets exist")
                .msaa
                .is_none(),
            "off back stays the 1x path bit-identical"
        );
    }
}
