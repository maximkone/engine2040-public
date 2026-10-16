//! presenter: `Presenter` assembly, lifecycle and cache accessors.
//!
//! Units: pixels for sizes, GPU handles for cached entries. Methods that
//! encode or submit frames live in `submit`; free helpers there too. The
//! per-frame fog and GTAO tranche lives in `fog_tranche`.
use super::super::fog_fwd::{FogFwd, FOG_FWD_UNIFORM_BYTES};
use super::super::*;
use super::frame::{gtao_err, sky_err, GpuMesh, GpuTexture, Targets};
use super::msaa::{msaa_sample_count, msaa_target_desc, msaa_transient_bytes, MsaaConfig};
use super::submit::{
    blit_pipeline, forward_pipeline, material_entry, padded_depth_row, sky_pipeline,
    uniform_buffer, uniform_entry, MATERIAL_RING_BYTES, MATERIAL_RING_LEN,
};
use super::timing::GpuTimer;
use super::{BLIT_WGSL, HDR_FORMAT, IBL_STUDIO_HDRI_BYTES, OUTPUT_FORMAT};
use crate::v1::{self};
use crate::v1_gpu::FORWARD_PBR_WGSL;
use crate::{decals, fog, gtao, ibl, material_layer, overlay, particles_gpu, sky};
use std::sync::Arc;

/// LF4 slice 1 bloom state: enable flag plus CPU-mirrored thresholds.
/// Owner presenter, consumer `submit.rs` live encode. Units linear HDR
/// threshold plus fraction knee plus flag. Determinism pure Copy, no
/// allocation, no device. Reuses the HDR target, sizes nothing per frame.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` live encode.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct BloomState {
    /// Live halo switch: false keeps today's frame bit-identical.
    pub enabled: bool,
    /// Bloom thresholds mirroring [`crate::post::BloomConfig`].
    pub config: crate::post::BloomConfig,
}

impl BloomState {
    /// Default on with the CPU oracle thresholds.
    pub fn on() -> Self {
        Self {
            enabled: true,
            config: crate::post::BloomConfig::default(),
        }
    }

    /// Flag for `max_luma`: `1.0` when enabled and the oracle gate runs,
    /// else `0.0` (bit-identical off). Fail-closed by delegation.
    #[must_use]
    pub fn flag(&self, max_luma: f32) -> f32 {
        if self.enabled
            && crate::post::bloom_should_run(max_luma, self.config.threshold, self.config.knee)
        {
            1.0
        } else {
            0.0
        }
    }
}

/// Windowed presenter: surface/swapchain plus HDR, depth and LDR-twin
/// targets, GPU mesh/texture caches, last-good recovery. Units: pixels
/// for sizes, GPU handles for cached entries. Pixels never enter any
/// simulation hash.
pub struct Presenter {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    /// Owns the instance that created `surface`. Dropping the instance
    /// destroys its wgpu-core storage, so a later acquire panics with
    /// `Cannot get non-existent resource SurfaceId`; keeping one clone
    /// here keeps the surface alive. `None` on the offscreen twin.
    pub(crate) instance: Option<wgpu::Instance>,
    pub(crate) adapter_name: String,
    pub(crate) adapter_driver: String,
    pub(crate) forward_layout: wgpu::BindGroupLayout,
    pub(crate) forward_hdr: wgpu::RenderPipeline,
    /// 4x MSAA forward pipeline for the on path: same shader plus layout
    /// as `forward_hdr` with count 4; bound when `targets.msaa` is `Some`.
    pub(crate) forward_hdr_msaa: wgpu::RenderPipeline,
    pub(crate) blit_layout: wgpu::BindGroupLayout,
    pub(crate) blit_shader: wgpu::ShaderModule,
    pub(crate) blit_sampler: wgpu::Sampler,
    /// Filtering sampler for per-draw base-color textures (shared by
    /// every draw bind; created once, never per frame).
    pub(crate) base_sampler: wgpu::Sampler,
    /// Blit into the LDR twin; built once (no per-frame pipeline alloc).
    pub(crate) twin_pipeline: wgpu::RenderPipeline,
    pub(crate) surface_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    pub(crate) surface: Option<wgpu::Surface<'static>>,
    pub(crate) surface_config: Option<wgpu::SurfaceConfiguration>,
    pub(crate) swap: v1::SwapState,
    pub(crate) targets: Option<Targets>,
    pub(crate) cam_buf: wgpu::Buffer,
    pub(crate) light_buf: wgpu::Buffer,
    /// One 256-byte-strided uniform ring for every draw's material: one
    /// `write_buffer` per frame, bound with a dynamic offset per draw.
    pub(crate) material_ring: wgpu::Buffer,
    /// CPU image of the material ring, reused every frame.
    pub(crate) material_scratch: Vec<u8>,
    /// Packed vertex bytes of a dynamic mesh refresh, reused every frame.
    pub(crate) vertex_scratch: Vec<u8>,
    /// Forward bind group per resident base-color texture, built when the
    /// texture uploads and dropped when it leaves, never inside a frame.
    pub(crate) texture_binds: Vec<(TextureKey, wgpu::BindGroup)>,
    /// Forward bind group over the 1x1 white fallback (`None` draws).
    pub(crate) fallback_bind: wgpu::BindGroup,
    /// Neutral 1x1 normal-map fallback (linear 128,128,255) bound at
    /// forward 17 for draws with no normal map; the TBN decode keeps
    /// the geometric normal. Cold init only, never per frame.
    /// Caller: `Presenter::build` creates it, `forward_bind` samples it.
    pub(crate) normal_fallback: GpuTexture,
    /// Bind group chosen for each draw of the frame: an index into
    /// `texture_binds`, `None` for the fallback. Reused scratch.
    pub(crate) draw_binds: Vec<Option<usize>>,
    /// Buffers and bind groups created, per frame and in total.
    pub(crate) creates: CreateTally,
    pub(crate) meshes: CpuCache<MeshKey, GpuMesh>,
    pub(crate) textures: CpuCache<TextureKey, GpuTexture>,
    /// Arena crowd mesh and palette, packed by `ensure_crowd`. They sit
    /// beside the caches, not in them, so the receipt's cache rows count
    /// only user uploads. The palette's bind group lives in `texture_binds`.
    pub(crate) crowd_mesh: Option<GpuMesh>,
    pub(crate) crowd_palette: Option<GpuTexture>,
    pub(crate) device_lost: Option<String>,
    pub(crate) frames_presented: u64,
    /// Surface frames between look-probe rounds (sky, layer, particle and
    /// decal bind probes plus the IBL chart); 1 probes every frame.
    pub(crate) probe_every: u64,
    /// Surface frames between fog-tranche refreshes; 1 refreshes every
    /// frame (the offscreen twin always refreshes).
    pub(crate) fog_every: u64,
    /// Day capture bound by the RL4 sky pass (baked once at init; WIN1
    /// will hand time-of-day bakes per frame next, same shape).
    pub(crate) sky_capture: sky::SkyCapture,
    /// RL8 look bake over `sky_capture` (baked once on the plan worker
    /// thread at init, joined before bind): the RA1 skybox samples these
    /// texels while RL2 ambient and RL7 fog read the precomputed pair
    /// from the same backing every frame. Row order follows the day
    /// capture (row 0 at the zenith); the RA1 bottom-up blit finding
    /// (BLOCKERS RA1 row) is a blit-UV decision owned by RA1 and is not
    /// reoriented here.
    pub(crate) sky_look: sky::SkyLookBake,
    /// LF1 live sky: the one init-time sky-view texture (64x32 Rgba32Float,
    /// row 0 at the zenith) sampled by `fs_sky`. Uploaded once, never per
    /// frame, same backing the fog horizon reads.
    pub(crate) sky_view: wgpu::Texture,
    /// View over `sky_view` bound at forward 15 for the sky pass.
    pub(crate) sky_view_view: wgpu::TextureView,
    /// LF1 live exposure: EV100 scale from the one `sky_forward_feed`
    /// call, staged into frame materials. Presentation only.
    pub(crate) sky_exposure: f32,
    /// RL4 sky bind state (steady-state submits size nothing).
    pub(crate) sky_sub: sky::SkySubmitter,
    /// RL6 layer chart bind state (steady-state submits size nothing).
    pub(crate) layer_sub: material_layer::LayerSubmitter,
    /// RP2 live particle pool stepped once per presented frame.
    pub(crate) particles: particles_gpu::ParticlePool,
    /// RP2 particle bind state (sizes only when the word shape moves).
    pub(crate) particle_sub: particles_gpu::ParticleSubmitter,
    /// RP3 resident decal pool (empty until WIN1 maps CB2 gore events).
    pub(crate) decal_pool: decals::DecalPool,
    /// RP3 decal bind state (sizes only when the word shape moves).
    pub(crate) decal_sub: decals::DecalSubmitter,
    /// RL9 fog plus contact submitter (steady-state submits on a shape
    /// match size nothing; see the reuse test in `fog.rs`).
    pub(crate) fog_sub: fog::FogSubmitter,
    /// GTAO bind state (steady-state submits size nothing; the
    /// reuse test in `gtao.rs` proves it).
    pub(crate) gtao_sub: gtao::GtaoSubmitter,
    /// IBL chart bind state (steady-state submits size nothing;
    /// the reuse test in `ibl.rs` proves it).
    pub(crate) ibl_sub: ibl::gpu::IblSubmitter,
    /// Lighting bake the chart samples every frame (baked once at
    /// init from the studio fixture; same bytes as the cook reference).
    pub(crate) ibl_bake: ibl::IblBake,
    /// Live forward prefilter array (30 layers of 64x64 float texels).
    pub(crate) ibl_prefilter: wgpu::Texture,
    /// Live forward BRDF LUT (64x64 scale/bias pairs).
    pub(crate) ibl_brdf: wgpu::Texture,
    /// Live forward SH uniform (144 bytes, 9 vec4s).
    pub(crate) ibl_sh_buf: wgpu::Buffer,
    /// RL9 compiled fog -> forward wiring, refreshed only on shape
    /// change so steady-state frames allocate nothing for the graph.
    pub(crate) fog_plan: Option<(u32, u32, crate::graph::CompiledGraph)>,
    /// RL9 depth readback staging, shape-keyed like the fog buffers.
    pub(crate) depth_rb: Option<(u32, u32, wgpu::Buffer)>,
    /// RL9 scratch for fog/ao byte staging (resized on shape change
    /// only, reused across both uploads and the neutral clears).
    pub(crate) fog_scratch: Vec<u8>,
    /// RL9 kill switch (A/B evidence and operator fallback); default on.
    pub(crate) fog_enabled: bool,
    /// RL9 evidence: mean fog factor over the top frame half.
    pub(crate) fog_top_mean: f32,
    /// RL9 evidence: mean fog factor over the bottom frame half.
    pub(crate) fog_bottom_mean: f32,
    /// RL9 evidence: minimum contact AO (below 1 proves shadowing ran).
    pub(crate) fog_contact_min: f32,
    /// RL9 evidence: mean contact AO over the feed halves (normalizes
    /// the ramp proof: fogged bands read ramp times AO).
    pub(crate) fog_ao_top_mean: f32,
    pub(crate) fog_ao_bottom_mean: f32,
    /// RL9 evidence: fog submit wall time in us (FOG+CONTACT bars).
    pub(crate) fog_submit_us: u64,
    /// GTAO evidence: submit wall time in us (0.8 ms bar).
    pub(crate) gtao_submit_us: u64,
    /// Twin sun pass over `twin_sun_config`: the per-frame bind
    /// resolves the frame focus depth into this pass's band, so the
    /// window twin samples the landed cascaded sun.
    pub(crate) twin_sun: crate::shadow_cluster::SunShadowPass,
    /// Twin shadow uniform (32 bytes under `twin_forward_wgsl`):
    /// the per-frame bind refreshes the active cascade here.
    pub(crate) twin_shadow_buf: wgpu::Buffer,
    /// Warmed pipeline variants for the scene key set (pipeline-cache
    /// residency; load allocates, per-frame borrows).
    pub(crate) pipelines: crate::cache::PipelineCache,
    /// Scene variant keys warmed at load; the per-frame walk
    /// borrows this slice so steady-state frames allocate nothing.
    pub(crate) pipeline_keys: Vec<crate::cache::PipelineKey>,
    /// Opt-in look probes: the sky/layer/particle/decal bind probes, the
    /// IBL chart, the depth readback plus CPU decode and the fog and GTAO
    /// compute tranche, each a blocking GPU readback. Default off: the
    /// live window frame never waits on the GPU.
    pub(crate) look_probes: bool,
    /// MSAA on/off switch (EG13 slice 1): default off keeps today's 1x
    /// path. The window leg flips it; slice 2 builds the 4x targets plus
    /// resolve. Copyable state, no per-frame allocation.
    pub(crate) msaa: MsaaConfig,
    /// Height fog and sky colors the forward shader evaluates per fragment.
    pub(crate) fog_fwd: FogFwd,
    /// Per-frame fog uniform (group 0 binding 6, read by the forward
    /// fragment and by the sky entry points).
    pub(crate) fog_fwd_buf: wgpu::Buffer,
    /// Per-frame torch block (group 0 binding 16, EG12 slice 2): two warm
    /// torches when draws exist, else zeroed. Presentation only.
    pub(crate) torch_buf: wgpu::Buffer,
    /// Fullscreen far-plane sky draw (`vs_sky`/`fs_sky` of the forward
    /// shader over the forward bind layout; it reads only the fog uniform).
    pub(crate) sky_pipeline: wgpu::RenderPipeline,
    /// Blocking-call tally per frame kind (live, probe, capture).
    pub(crate) waits: WaitStats,
    /// Blocking calls recorded so far in the frame being encoded.
    pub(crate) frame_waits: FrameWaits,
    /// Pass timestamps resolved late; `None` when the adapter lacks
    /// `TIMESTAMP_QUERY`.
    pub(crate) gpu_timer: Option<GpuTimer>,
    /// Surface acquire wall time summed over acquired frames (us).
    pub(crate) acquire_us: u64,
    /// Surface frames acquired.
    pub(crate) acquire_frames: u64,
    /// The fog pair holds stale probe output and must be reset to neutral
    /// before the next live frame blits it.
    pub(crate) neutral_fog_pending: bool,
    /// CPU HUD quad ring in 0..1 space; drained by the next slice.
    pub(crate) overlay: overlay::OverlayRing,
    /// LF4 slice 1 bloom state: enable flag plus pass thresholds, default
    /// on. Init only, no per-frame allocation; reuses the HDR target.
    pub(crate) bloom: BloomState,
}

/// Staged frame uniforms: camera bytes and light bytes (the per-draw
/// materials go straight into `Presenter::material_scratch`).
pub(crate) type StagedUniforms = (Vec<u8>, Vec<u8>);

/// Neutral normal-map texel in 0..=255: 128,128,255 decodes to (0,0,1)
/// via `eg12_decode_normal`, so the TBN keeps the geometric normal.
/// Owner presenter, consumer the forward 17 fallback bind.
/// Caller: `Presenter::build` uploads it once, `forward_bind` samples it.
fn normal_fallback_rgba() -> [u8; 4] {
    [128, 128, 255, 255]
}

/// Layout entries for EG12 slice c normal sampling at group 0 bindings
/// 17 (filterable 2D texture) plus 18 (filtering sampler). Cold layout
/// only, never per frame. Mirrors `shaders/forward.wgsl` `normal_map`
/// plus `normal_smp` sampled by `fs` and `fs_skinned`.
/// Owner presenter, consumer `Presenter::build` forward layout.
/// Caller: `Presenter::build` extends the forward entries with these.
fn forward_normal_layout_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [
        wgpu::BindGroupLayoutEntry {
            binding: 17,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 18,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ]
}

/// LF1 live sky bind: the forward group plus the init-time sky-view
/// texture at 15. Same buffers the IBL live bind carries, plus the sky
/// view; cold bind creation only, never per frame. Keeps the binary
/// uniform layout, adds no pass.
#[cfg(feature = "window")]
fn forward_bind_with_sky(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    cam: &wgpu::Buffer,
    light: &wgpu::Buffer,
    material: &wgpu::Buffer,
    material_bytes: u64,
    shadow: &wgpu::Buffer,
    base_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    fog: &wgpu::Buffer,
    torch: &wgpu::Buffer,
    pre_view: &wgpu::TextureView,
    brdf_view: &wgpu::TextureView,
    sh_buf: &wgpu::Buffer,
    sky_view: &wgpu::TextureView,
    normal_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ra1-forward-bind-sky"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: cam.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: light.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: material,
                    offset: 0,
                    size: wgpu::BufferSize::new(material_bytes),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: shadow.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(base_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: fog.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(pre_view),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(brdf_view),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: sh_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 15,
                resource: wgpu::BindingResource::TextureView(sky_view),
            },
            wgpu::BindGroupEntry {
                binding: 16,
                resource: torch.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 17,
                resource: wgpu::BindingResource::TextureView(normal_view),
            },
            wgpu::BindGroupEntry {
                binding: 18,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

impl Presenter {
    /// 1x1 white sRGB texture (samples to linear 1.0): the identity the
    /// `None` draws bind, so untextured pixels never move. Cold init path
    /// only; fixed bytes, never caller input.
    fn fallback_white(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTexture {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-base-fallback"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        GpuTexture {
            texture,
            view,
            width: 1,
            height: 1,
        }
    }

    /// 1x1 neutral normal texture (linear 128,128,255,255): the identity
    /// the `None` normal draws bind, so unmapped pixels keep the geometric
    /// normal via the TBN decode. Cold init path only; fixed bytes, never
    /// caller input. Data maps stay linear, never sRGB.
    /// Caller: `Presenter::build` creates it once, `forward_bind` samples it.
    fn normal_fallback(device: &wgpu::Device, queue: &wgpu::Queue) -> GpuTexture {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-normal-fallback"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &normal_fallback_rgba(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        GpuTexture {
            texture,
            view,
            width: 1,
            height: 1,
        }
    }

    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: wgpu::AdapterInfo,
    ) -> Result<Self, PresentError> {
        // The one forward shader file: static and sky vertex entries, the
        // linear HDR fragment entry, the sky fragment entry. No patching.
        let forward_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ra1-forward"),
            source: wgpu::ShaderSource::Wgsl(FORWARD_PBR_WGSL.into()),
        });
        let mut forward_entries = vec![
            uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
            uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            material_entry(2),
            uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            uniform_entry(6, wgpu::ShaderStages::FRAGMENT),
            // LF1 live sky: the init-time sky-view texture (Rgba32Float,
            // unfilterable, manual bilinear in `fs_sky`). Free binding:
            // 9-12 owned by IBL, 13-14 dead sun shadow, 7-8 skin.
            wgpu::BindGroupLayoutEntry {
                binding: 15,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // EG12 slice 2: live torch block, first of 16-19.
            uniform_entry(16, wgpu::ShaderStages::FRAGMENT),
        ];
        forward_entries.extend_from_slice(&forward_normal_layout_entries());
        forward_entries.extend_from_slice(&ibl::gpu::forward_ibl_layout_entries());
        let forward_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ra1-uniforms"),
            entries: &forward_entries,
        });
        let forward_hdr = forward_pipeline(&device, &forward_shader, &forward_layout, HDR_FORMAT);
        // EG13 slice 1 keeps the 1x path: the MSAA pipeline reuses the 1x
        // build until slice 2 wires the 4x targets plus resolve.
        let forward_hdr_msaa =
            forward_pipeline(&device, &forward_shader, &forward_layout, HDR_FORMAT);
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ra1-blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ra1-blit-bind"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // RL9 fog-factor pair: exact texel loads, never filtered.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ra1-blit-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let twin_pipeline = blit_pipeline(&device, &blit_shader, &blit_layout, OUTPUT_FORMAT);
        // Base-color sampling: one shared filtering sampler plus the
        // 1x1 white fallback every `None` draw binds (identity multiply).
        // Cold init path only, never per frame.
        let base_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ra1-base-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        // The 1x1 white texture `None` draws sample (identity multiply): the
        // fallback bind group below keeps it alive for the presenter's life.
        let base_fallback = Self::fallback_white(&device, &queue);
        // EG12 slice c neutral normal (linear 128,128,255): the identity
        // `None` normal draws sample, so unmapped pixels keep geometry.
        // Cold init only; the bind groups below keep it alive.
        let normal_fallback = Self::normal_fallback(&device, &queue);
        let cam_buf = uniform_buffer(&device, "ra1-u-camera", 80);
        let light_buf = uniform_buffer(&device, "ra1-u-light", 32);
        // The twin sun pass is std-only (no device work); a bad
        // shape fails init loudly, never a silent unlit twin.
        let twin_sun = crate::twin_shadow::twin_sun_pass()
            .map_err(|e| PresentError::Device(format!("rl19 twin sun: {e}")))?;
        let twin_shadow_buf = uniform_buffer(&device, "rl19-twin-shadow", 32);
        // The day capture is the arena day rig (one sun preset feeding
        // the look bake and the capture bundle from one bake);
        // per-frame time-of-day bakes ride the same submit shape, so this
        // never grows the steady-state path. A failed bake fails init
        // loudly: the preset params are fixed valid inputs, so this is
        // unreachable unless the sky contract itself changed.
        let sky_capture = crate::arena_day::bake_arena_day()
            .map(|rig| rig.capture)
            .map_err(|e| PresentError::Device(format!("look day rig bake: {e}")))?;
        // The look bake over the day capture runs on the plan worker
        // thread and joins before bind, so the first frame never hitches
        // on a same-thread bake. A failed bake fails init loudly: the
        // day-rig params are fixed valid inputs, so this is unreachable
        // unless the sky contract itself changed.
        let (sky_look, _) = super::super::bake_look_on_worker(sky_capture.clone())?;
        // LF1 live sky: one feed gives the init-time sky-view texels plus
        // the horizon, zenith and EV100 exposure from the same bake, so
        // the upload, fog and exposure can never drift. Row-major 64x32
        // Rgba32Float (alpha 1.0), row 0 at the zenith; `fs_sky` samples
        // it by ray direction. Once at init, never per frame; the live
        // layout carries it at forward 15. SkyError fails init loudly.
        let (sky_upload_texels, sky_horizon, sky_zenith, sky_exposure) =
            sky::sky_forward_feed(&sky_look, crate::arena_day::ARENA_DAY_EXPOSURE_EV100)
                .map_err(sky_err)?;
        debug_assert_eq!(sky_upload_texels.len(), sky_look.texels.len());
        if !sky_exposure.is_finite() || sky_exposure <= 0.0 {
            return Err(sky_err(sky::SkyError::Invalid(format!(
                "sky forward feed exposure {sky_exposure} is not positive finite"
            ))));
        }
        debug_assert_eq!(
            (sky_look.width_px, sky_look.height_px),
            (
                crate::arena_day::ARENA_DAY_CAPTURE_W,
                crate::arena_day::ARENA_DAY_CAPTURE_H
            )
        );
        let sky_view = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-sky-view"),
            size: wgpu::Extent3d {
                width: sky_look.width_px,
                height: sky_look.height_px,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let mut sky_bytes = Vec::with_capacity(sky_upload_texels.len() * 16);
            let one = 1.0f32.to_le_bytes();
            for t in &sky_upload_texels {
                sky_bytes.extend_from_slice(&t[0].to_le_bytes());
                sky_bytes.extend_from_slice(&t[1].to_le_bytes());
                sky_bytes.extend_from_slice(&t[2].to_le_bytes());
                sky_bytes.extend_from_slice(&one);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &sky_view,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &sky_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(sky_look.width_px * 16),
                    rows_per_image: Some(sky_look.height_px),
                },
                wgpu::Extent3d {
                    width: sky_look.width_px,
                    height: sky_look.height_px,
                    depth_or_array_layers: 1,
                },
            );
        }
        let sky_view_view = sky_view.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ra1-sky-view-view"),
            ..Default::default()
        });
        // The studio HDRI bakes once here (same fixture bytes the
        // cook reference bakes); per-frame charts sample this bake, so the
        // steady-state path never cooks. A failed bake fails init loudly:
        // the fixture bytes are fixed valid inputs, so this is
        // unreachable unless the IBL contract itself changed.
        let (ibl_bake, _) = ibl::bake(IBL_STUDIO_HDRI_BYTES)
            .map_err(|e| PresentError::Device(format!("look ibl bake: {e}")))?;
        // Live forward bake upload once through the bake owner.
        let (ibl_prefilter, ibl_brdf, ibl_sh_buf) =
            ibl::gpu::upload_live_bake(&device, &queue, &ibl_bake)
                .map_err(|e| PresentError::Device(format!("live ibl bake: {e}")))?;
        // Warm every scene variant before the first frame over
        // the real forward shader bytes, so the camera walk never
        // compiles mid-frame. Blobs are cache-residency markers keyed
        // by (shader, defines, layout); device variant pipelines ride
        // their own pass rows. Load-path only: allocates the warm set.
        let pipeline_keys = super::super::scene_variant_keys_for(
            crate::v1_gpu::FORWARD_PBR_WGSL.as_bytes(),
            super::super::OPT19_PIPELINE_LAYOUT,
        );
        let mut pipelines = crate::cache::PipelineCache::new();
        let _warm = pipelines.warm_keys(&pipeline_keys, |key| {
            Ok(vec![key.shader_hash.to_le_bytes()[0]; 8])
        });
        // Fog colors come from the same feed the sky-view upload used:
        // the horizon ring is the fog color, straight up is the zenith.
        let fog_fwd = FogFwd::day(sky_horizon, sky_zenith);
        let fog_fwd_buf =
            uniform_buffer(&device, "ra1-u-fog-forward", FOG_FWD_UNIFORM_BYTES as u64);
        // EG12 slice 2: staged torch block, 272 bytes, zeroed for clears.
        let torch_buf = uniform_buffer(&device, "eg12-u-torch", 272);
        let sky_pipeline = sky_pipeline(&device, &forward_shader, &forward_layout, HDR_FORMAT);
        let material_ring = uniform_buffer(&device, "ra1-u-material-ring", MATERIAL_RING_BYTES);
        let ibl_prefilter_view = ibl_prefilter.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ra1-ibl-prefilter-view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let ibl_brdf_view = ibl_brdf.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ra1-ibl-brdf-view"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });
        let fallback_bind = forward_bind_with_sky(
            &device,
            &forward_layout,
            &cam_buf,
            &light_buf,
            &material_ring,
            super::submit::MATERIAL_BYTES,
            &twin_shadow_buf,
            &base_fallback.view,
            &base_sampler,
            &fog_fwd_buf,
            &torch_buf,
            &ibl_prefilter_view,
            &ibl_brdf_view,
            &ibl_sh_buf,
            &sky_view_view,
            &normal_fallback.view,
        );
        let gpu_timer = GpuTimer::new(&device, &queue);
        // EG13 slice 1: the MSAA on/off switch. Off (default) keeps
        // today's 1x HDR plus depth targets and the 1x submit path; on
        // notes the 4x shape (donor pipeline multisample count 4 with a
        // view/resolveTarget pair resolved before the blit) while the 1x
        // path keeps working. Full MSAA textures plus resolve land in
        // slice 2, so this stays EG2-green. Never waits on the GPU.
        let msaa = MsaaConfig::default();
        debug_assert_eq!(
            msaa_sample_count(&msaa),
            1,
            "MSAA defaults off: the 1x path"
        );
        // The desc names the HDR plus depth formats with the chosen count;
        // slice 2 builds the 4x textures from it (resolve before the blit).
        let msaa_desc = msaa_target_desc(&msaa);
        debug_assert_eq!(msaa_desc.sample_count, 1);
        Ok(Self {
            material_ring,
            material_scratch: Vec::with_capacity(MATERIAL_RING_LEN),
            vertex_scratch: Vec::new(),
            texture_binds: Vec::new(),
            fallback_bind,
            normal_fallback,
            draw_binds: Vec::with_capacity(MAX_DRAWS_PER_FRAME),
            creates: CreateTally::default(),
            look_probes: false,
            msaa,
            fog_fwd,
            fog_fwd_buf,
            torch_buf,
            sky_pipeline,
            waits: WaitStats::default(),
            frame_waits: FrameWaits::default(),
            gpu_timer,
            acquire_us: 0,
            acquire_frames: 0,
            neutral_fog_pending: false,
            overlay: overlay::OverlayRing::new(),
            bloom: BloomState::on(),
            sky_sub: sky::SkySubmitter::new(&device),
            layer_sub: material_layer::LayerSubmitter::new(&device),
            particles: particles_gpu::ParticlePool::new(2040),
            particle_sub: particles_gpu::ParticleSubmitter::new(&device),
            decal_pool: decals::DecalPool::new(),
            decal_sub: decals::DecalSubmitter::new(&device),
            fog_sub: fog::FogSubmitter::new(&device),
            gtao_sub: gtao::GtaoSubmitter::new(&device),
            ibl_sub: ibl::gpu::IblSubmitter::new(&device),
            ibl_bake,
            ibl_prefilter,
            ibl_brdf,
            ibl_sh_buf,
            fog_plan: None,
            depth_rb: None,
            fog_scratch: Vec::new(),
            fog_enabled: true,
            fog_top_mean: 0.0,
            fog_bottom_mean: 0.0,
            fog_contact_min: 1.0,
            fog_ao_top_mean: 1.0,
            fog_ao_bottom_mean: 1.0,
            fog_submit_us: 0,
            gtao_submit_us: 0,
            twin_sun,
            twin_shadow_buf,
            pipelines,
            pipeline_keys,
            device,
            queue,
            instance: None,
            adapter_name: info.name.clone(),
            adapter_driver: format!("{} {}", info.driver, info.driver_info),
            forward_layout,
            forward_hdr,
            forward_hdr_msaa,
            blit_layout,
            blit_shader,
            blit_sampler,
            base_sampler,
            twin_pipeline,
            surface_pipeline: None,
            surface: None,
            surface_config: None,
            swap: v1::SwapState::new(0, 0),
            targets: None,
            cam_buf,
            light_buf,
            meshes: CpuCache::new(MAX_CACHED_MESHES),
            textures: CpuCache::new(MAX_CACHED_TEXTURES),
            crowd_mesh: None,
            crowd_palette: None,
            device_lost: None,
            frames_presented: 0,
            probe_every: 1,
            fog_every: 1,
            sky_capture,
            sky_look,
            sky_view,
            sky_view_view,
            sky_exposure,
        })
    }

    /// Bring up a high-performance adapter with no surface and compile the
    /// forward plus blit pipelines. `NoAdapter` on headless machines is
    /// skip-not-green for callers, never a fake pass.
    pub fn init_offscreen() -> Result<Self, PresentError> {
        // Backend pick is resolved here for the wgpu instance; the binary
        // logs `pick.startup_line()`. No stdout in lib.
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: crate::backend::wgpu_backends(pick.primary),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        // GS6: enumerate and rank-pick the env choice instead of trusting
        // the driver's power-preference hint, so an Optimus laptop never
        // lands on the Intel adapter and the CPU rasterizer fails loudly
        // (NoAdapter) unless E2040_ALLOW_SOFTWARE=1.
        let want = crate::adapter::choice_from_env();
        let adapter = pollster::block_on(crate::presenter_pick::pick_adapter(
            &instance,
            crate::backend::wgpu_backends(pick.primary),
            &want,
        ))
        .map_err(|_| PresentError::NoAdapter)?;
        let info = adapter.get_info();
        let (device, queue) = Self::request_device(&adapter)?;
        Self::build(device, queue, info)
    }

    /// Open the device and queue on `adapter`, asking for pass timestamps
    /// when the adapter supports them (the GPU timing rows stay
    /// `unsupported` otherwise and nothing else changes).
    fn request_device(
        adapter: &wgpu::Adapter,
    ) -> Result<(wgpu::Device, wgpu::Queue), PresentError> {
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("engine2040-ra1-present"),
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }))
        .map_err(|e| PresentError::Device(format!("request_device: {e}")))
    }

    /// Bring up an adapter compatible with `surface`, configure the
    /// swapchain at `width` x `height` and compile the surface blit
    /// pipeline. Zero area is refused (configuring would panic); the
    /// caller suspends instead. The surface keeps the window alive.
    pub fn init_windowed(
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        // Windowed path uses the caller-built instance; backend selection
        // is reported by the binary via `backend::startup_line`. No
        // stdout in lib. GS6: rank-pick over the surface-compatible
        // adapters so the env choice wins over the driver's hint.
        let (bchoice, _) = crate::backend::resolve_from_env(None);
        let bp = crate::backend::pick_backend(bchoice, None);
        let adapter = Self::pick_surface_adapter(
            instance,
            &surface,
            crate::backend::wgpu_backends(bp.primary),
        )?;
        let info = adapter.get_info();
        let (device, queue) = Self::request_device(&adapter)?;
        let mut presenter = Self::build(device, queue, info)?;
        presenter.attach_surface(instance, surface, width, height)?;
        Ok(presenter)
    }

    /// Bring up the visible presenter for an owned winit window: build
    /// the backend-picked instance, attach the surface (which keeps the
    /// window alive) and configure the swapchain at `width` x `height`.
    /// The instance is retained through the attach so the surface stays
    /// valid past this call. This is the one call the window loop makes;
    /// the offscreen twin (`init_offscreen` plus `render_offscreen`)
    /// stays the explicit capture path. Zero area fails loudly (the
    /// caller suspends instead); `NoAdapter` is never a fake present.
    pub fn init_for_window(
        window: Arc<winit::window::Window>,
        width: u32,
        height: u32,
    ) -> Result<Self, PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: crate::backend::wgpu_backends(pick.primary),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = create_surface(&instance, window)?;
        Self::init_windowed(&instance, surface, width, height)
    }

    /// Pick the swapchain present mode: the shortest acquire wait the
    /// driver offers, else the first listed mode. Units: none.
    /// Caller: `Presenter::attach_surface`.
    pub fn pick_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
        if modes.contains(&wgpu::PresentMode::Mailbox) {
            return wgpu::PresentMode::Mailbox;
        }
        if modes.contains(&wgpu::PresentMode::Immediate) {
            return wgpu::PresentMode::Immediate;
        }
        modes.first().copied().unwrap_or(wgpu::PresentMode::Fifo)
    }

    /// Attach (or re-attach) a surface, reconfiguring the swapchain and
    /// rebuilding the surface blit pipeline. The old pipeline stays on
    /// failure, so last-good survives a bad attach. `instance` must be
    /// the same [`wgpu::Instance`] that created `surface`: surface
    /// handles are per-instance, so a second instance panics inside
    /// wgpu-core instead of returning `NoAdapter`. On success the
    /// instance is cloned into the presenter so it outlives the surface.
    pub fn attach_surface(
        &mut self,
        instance: &wgpu::Instance,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<(), PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let adapter = Self::compatible_adapter(instance, &surface)?;
        let caps = surface.get_capabilities(&adapter);
        let Some(first_format) = caps.formats.first().copied() else {
            return Err(PresentError::Device("surface reports no formats".into()));
        };
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(first_format);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width,
            height,
            present_mode: Self::pick_present_mode(&caps.present_modes),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.device, &config);
        let pipeline = blit_pipeline(&self.device, &self.blit_shader, &self.blit_layout, format);
        self.surface_pipeline = Some((format, pipeline));
        self.surface = Some(surface);
        self.surface_config = Some(config);
        self.instance = Some(instance.clone());
        let _ = self.resize(width, height);
        Ok(())
    }

    /// Adapter compatible with `surface` for capability listing. The
    /// device adapter stays put so live pipelines are never disturbed.
    /// `instance` is the same instance that created `surface`; a fresh
    /// instance would panic inside wgpu-core on lookup.
    /// `NoAdapter` (not a panic) when no adapter supports the surface.
    fn compatible_adapter(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<wgpu::Adapter, PresentError> {
        let (bchoice, _) = crate::backend::resolve_from_env(None);
        let bp = crate::backend::pick_backend(bchoice, None);
        Self::pick_surface_adapter(instance, surface, crate::backend::wgpu_backends(bp.primary))
    }

    /// Enumerate `instance`, keep the surface-compatible adapters and
    /// rank-pick the `E2040_ADAPTER` choice through
    /// [`crate::adapter::choose_compatible`]. Cold path only; the pick
    /// never falls back silently: an empty enumerate, a software-only
    /// list without opt-in, or no compatible adapter is `NoAdapter`.
    /// Caller: `init_windowed` plus `compatible_adapter` (the GS6 surface
    /// sites); `init_offscreen` uses `pick_adapter` (no surface to test).
    fn pick_surface_adapter(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        backends: wgpu::Backends,
    ) -> Result<wgpu::Adapter, PresentError> {
        let want = crate::adapter::choice_from_env();
        let adapters = pollster::block_on(instance.enumerate_adapters(backends));
        let list: Vec<crate::adapter::AdapterSummary> = adapters
            .iter()
            .map(|found| crate::presenter_pick::summary_of(&found.get_info()))
            .collect();
        let compat: Vec<bool> = adapters
            .iter()
            .map(|found| found.is_surface_supported(surface))
            .collect();
        let index = crate::adapter::choose_compatible(
            &list,
            &compat,
            &want,
            crate::adapter::allow_software(),
        )
        .map_err(|_| PresentError::NoAdapter)?;
        adapters
            .into_iter()
            .nth(index)
            .ok_or(PresentError::NoAdapter)
    }

    /// Resize the backing targets. Zero area suspends (minimize path) and
    /// keeps last-good; over-bound sizes are refused without touching the
    /// GPU. Never panics.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<v1::RecreateOutcome, PresentError> {
        match SizeClass::classify(width, height) {
            SizeClass::Suspend => Ok(self.swap.recreate(0, 0)),
            SizeClass::Reject => Err(PresentError::InvalidSize { width, height }),
            SizeClass::Sizes => {
                let outcome = self.swap.recreate(width, height);
                if matches!(outcome, v1::RecreateOutcome::Reconfigured { .. }) {
                    self.targets = None;
                    if let (Some(surface), Some(config)) =
                        (self.surface.as_ref(), self.surface_config.as_mut())
                    {
                        config.width = width;
                        config.height = height;
                        surface.configure(&self.device, config);
                    }
                }
                // EG13 transient budget: 4x HDR plus 4x depth while on stays
                // under the cap; debug-only so release frames never branch.
                let (w, h) = self.swap.size();
                debug_assert!(
                    msaa_transient_bytes(w, h, &self.msaa) <= super::msaa::MSAA_TRANSIENT_CAP_BYTES,
                    "MSAA transient over budget"
                );
                Ok(outcome)
            }
        }
    }

    /// Record a device loss; later frames fail with `DeviceLost` until the
    /// consumer builds a new presenter. Seam for the AH2 supervisor.
    pub fn mark_device_lost(&mut self, reason: String) {
        self.device_lost = Some(reason);
    }

    /// Upload a mesh under `key`. The mesh is revalidated on upload (its
    /// fields are public, so hand-built data cannot sneak past), then
    /// packed into GPU buffers. A changed digest replaces the resident
    /// buffers; eviction drops the oldest entry first.
    pub fn upload_mesh(&mut self, key: MeshKey, mesh: &v1::MeshData) -> Result<(), PresentError> {
        let gpu = self.pack_mesh(mesh)?;
        if let Some(evicted) = self.meshes.insert(key, gpu) {
            debug_assert_ne!(evicted, key, "reinsert never evicts itself");
        }
        Ok(())
    }

    /// Pack a mesh into GPU buffers without caching it: `upload_mesh`
    /// inserts the result, and the arena crowd keeps its own copy.
    pub(super) fn pack_mesh(&self, mesh: &v1::MeshData) -> Result<GpuMesh, PresentError> {
        use wgpu::util::DeviceExt;
        // Revalidate: `MeshData` fields are public, so only the validator
        // decides what reaches the GPU. Uploads are rare (cached), so the
        // re-pack cost is off the frame path.
        let mesh = v1::upload_mesh(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices)
            .map_err(|e| PresentError::InvalidMesh(format!("{e}")))?;
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-mesh-vb"),
                contents: &mesh.vertex_bytes(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-mesh-ib"),
                contents: &mesh.index_bytes(),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.creates.buffer();
        self.creates.buffer();
        let count = u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX);
        let vertices = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
        Ok(GpuMesh {
            vertex: vb,
            index: ib,
            count,
            vertices,
        })
    }

    /// Upload validated pixels under `key`. Resident until evicted; held
    /// for the RL2 material bind group (`texture_keys` reports residency).
    pub fn upload_texture(
        &mut self,
        key: TextureKey,
        upload: &TextureUpload,
    ) -> Result<(), PresentError> {
        let (gpu, bind) = self.pack_texture(upload)?;
        self.texture_binds.retain(|(known, _)| *known != key);
        self.texture_binds.push((key, bind));
        if let Some(evicted) = self.textures.insert(key, gpu) {
            debug_assert_ne!(evicted, key, "reinsert never evicts itself");
            self.texture_binds.retain(|(known, _)| *known != evicted);
        }
        Ok(())
    }

    /// Pack validated pixels into a GPU texture and its forward bind group
    /// without caching either: `upload_texture` caches the pair, and the
    /// arena crowd keeps its own.
    pub(super) fn pack_texture(
        &self,
        upload: &TextureUpload,
    ) -> Result<(GpuTexture, wgpu::BindGroup), PresentError> {
        upload.validate()?;
        let size = wgpu::Extent3d {
            width: upload.width,
            height: upload.height,
            depth_or_array_layers: 1,
        };
        let mips = upload.mip_chain();
        let mip_level_count = u32::try_from(mips.len() + 1).unwrap_or(1);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ra1-texture"),
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &upload.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(upload.width * 4),
                rows_per_image: Some(upload.height),
            },
            size,
        );
        for (level, (width, height, rgba)) in (1u32..).zip(mips.iter()) {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(*height),
                },
                wgpu::Extent3d {
                    width: *width,
                    height: *height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // The forward bind group for this texture is built here, once, so
        // no frame ever creates one; a re-upload replaces the old group.
        let bind = self.forward_bind(&view);
        self.creates.bind_group();
        let gpu = GpuTexture {
            texture,
            view,
            width: upload.width,
            height: upload.height,
        };
        Ok((gpu, bind))
    }

    /// Borrow a resident texture view plus its size; `None` when absent.
    /// The RL2 material bind group consumes these views. This is the
    /// explicitly-marked preview path: the caller may substitute its own
    /// placeholder when `None`. Ship paths must call
    /// `require_texture_view` instead so a missing texture fails loudly
    /// with `MissingTexture` (never silent magenta).
    pub fn texture_view(&self, key: &TextureKey) -> Option<(&wgpu::TextureView, u32, u32)> {
        self.textures.get(key).map(|t| (&t.view, t.width, t.height))
    }

    /// Borrow a resident texture view plus its size, refusing loudly
    /// with `MissingTexture` (asset id named, `ASSETS_MISSING` fault)
    /// when absent. Ship path: fails the content gate, never falls back
    /// to magenta.
    pub fn require_texture_view(
        &self,
        key: &TextureKey,
    ) -> Result<(&wgpu::TextureView, u32, u32), PresentError> {
        self.textures
            .get(key)
            .map(|t| (&t.view, t.width, t.height))
            .ok_or_else(|| missing_texture_error(key.id))
    }

    /// Borrow a resident texture handle; `None` when absent. The handle
    /// keeps the allocation alive behind the views above.
    pub fn texture_handle(&self, key: &TextureKey) -> Option<&wgpu::Texture> {
        self.textures.get(key).map(|t| &t.texture)
    }

    /// Drop one cached texture; true when it was resident.
    pub fn evict_texture(&mut self, key: &TextureKey) -> bool {
        let evicted = self.textures.evict(key);
        if evicted {
            self.texture_binds.retain(|(known, _)| known != key);
        }
        evicted
    }

    /// Forward bind group over `view` and the shared frame buffers plus
    /// the live baked lighting in the forward group, with the LF1 sky-view
    /// texture at 15 for the sky pass (opaque `fs` ignores it). The neutral
    /// normal fallback rides at 17 until per-draw normal maps land.
    pub(crate) fn forward_bind(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        let pre_view = self
            .ibl_prefilter
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("ra1-ibl-prefilter-view"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let brdf_view = self.ibl_brdf.create_view(&wgpu::TextureViewDescriptor {
            label: Some("ra1-ibl-brdf-view"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            ..Default::default()
        });
        forward_bind_with_sky(
            &self.device,
            &self.forward_layout,
            &self.cam_buf,
            &self.light_buf,
            &self.material_ring,
            super::submit::MATERIAL_BYTES,
            &self.twin_shadow_buf,
            view,
            &self.base_sampler,
            &self.fog_fwd_buf,
            &self.torch_buf,
            &pre_view,
            &brdf_view,
            &self.ibl_sh_buf,
            &self.sky_view_view,
            &self.normal_fallback.view,
        )
    }

    /// Rewrite the vertices of the resident mesh `key` in place: pack
    /// position, normal and uv into the mesh's own vertex buffer with one
    /// `queue.write_buffer`, no new buffer, no revalidation of the index
    /// list. This is the per-frame path of a CPU-skinned body: create the
    /// mesh once with [`Presenter::upload_mesh`], refresh it every frame.
    /// Returns `Ok(false)` when `key` is not resident (the caller makes
    /// the one-time upload), `Ok(true)` once written. The vertex count is
    /// fixed for the buffer's life; a different count, a non-finite
    /// component or a degenerate normal is refused loudly and writes
    /// nothing.
    pub fn refresh_mesh_vertices(
        &mut self,
        key: MeshKey,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        uvs: &[[f32; 2]],
    ) -> Result<bool, PresentError> {
        let Some(resident) = self.meshes.get(&key) else {
            return Ok(false);
        };
        let expected = usize::try_from(resident.vertices).unwrap_or(usize::MAX);
        if positions.len() != expected || normals.len() != expected || uvs.len() != expected {
            return Err(PresentError::InvalidMesh(format!(
                "refresh mesh {}: {expected} vertices resident, got {}/{}/{}",
                key.id,
                positions.len(),
                normals.len(),
                uvs.len()
            )));
        }
        self.vertex_scratch.clear();
        for (index, ((p, n), uv)) in positions.iter().zip(normals).zip(uvs).enumerate() {
            let finite = p
                .iter()
                .chain(n.iter())
                .chain(uv.iter())
                .all(|c| c.is_finite());
            let normal_len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
            if !finite || normal_len2 < 1e-12 {
                return Err(PresentError::InvalidMesh(format!(
                    "refresh mesh {}: vertex {index} is not finite or has a degenerate normal",
                    key.id
                )));
            }
            for c in p.iter().chain(n.iter()).chain(uv.iter()) {
                self.vertex_scratch.extend_from_slice(&c.to_le_bytes());
            }
        }
        let Some(resident) = self.meshes.get(&key) else {
            return Ok(false);
        };
        self.queue
            .write_buffer(&resident.vertex, 0, &self.vertex_scratch);
        Ok(true)
    }

    /// Buffers and bind groups created since the presenter was built
    /// (cumulative): a steady frame leaves both unchanged.
    pub fn gpu_create_totals(&self) -> (u64, u64) {
        self.creates.totals()
    }

    /// Resident mesh count.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Resident texture count.
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Mesh budget capacity (`MAX_CACHED_MESHES`).
    pub fn mesh_capacity(&self) -> usize {
        self.meshes.capacity()
    }

    /// Texture budget capacity (`MAX_CACHED_TEXTURES`).
    pub fn texture_capacity(&self) -> usize {
        self.textures.capacity()
    }

    /// Mesh high-water mark: most meshes ever resident at once.
    pub fn mesh_high_water(&self) -> usize {
        self.meshes.high_water()
    }

    /// Texture high-water mark: most textures ever resident at once.
    pub fn texture_high_water(&self) -> usize {
        self.textures.high_water()
    }

    /// Cumulative mesh evictions (capacity plus manual drops).
    pub fn mesh_evicted(&self) -> u64 {
        self.meshes.evicted()
    }

    /// Cumulative texture evictions (capacity plus manual drops).
    pub fn texture_evicted(&self) -> u64 {
        self.textures.evicted()
    }

    /// Most recently evicted texture asset id, if any.
    pub fn last_evicted_texture_id(&self) -> Option<u64> {
        self.textures.last_evicted().map(|k| k.id)
    }

    /// Cache-budget rows for the frame receipt: counts, high-waters,
    /// cumulative evictions and last evicted ids, in receipt field order.
    pub(crate) fn cache_receipt_fields(
        &self,
    ) -> (u32, u32, u32, u32, u64, u64, Option<u64>, Option<u64>) {
        (
            self.meshes.len() as u32,
            self.textures.len() as u32,
            self.meshes.high_water() as u32,
            self.textures.high_water() as u32,
            self.meshes.evicted(),
            self.textures.evicted(),
            self.meshes.last_evicted().map(|k| k.id),
            self.textures.last_evicted().map(|k| k.id),
        )
    }

    /// Resident texture keys in sorted order (deterministic receipts).
    pub fn texture_keys(&self) -> Vec<TextureKey> {
        self.textures.sorted_keys()
    }

    /// Resident mesh keys in sorted order (deterministic receipts).
    pub fn mesh_keys(&self) -> Vec<MeshKey> {
        self.meshes.sorted_keys()
    }

    /// True while minimized/zero-area (last-good kept, nothing drawn).
    pub fn suspended(&self) -> bool {
        !self.swap.is_live()
    }

    /// Backing size (last-good while suspended) in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.swap.size()
    }

    /// Successful reconfigures so far.
    pub fn generation(&self) -> u64 {
        self.swap.generation()
    }

    /// Opt in to the look probes on the window path (default off): the
    /// sky, layer, particle and decal bind probes, the IBL chart, the depth
    /// readback with its CPU decode, and the fog and GTAO tranche. Each is
    /// a blocking GPU readback, so with this on, probe frames wait on the
    /// GPU (they are tallied apart from live frames) and the fog comes
    /// from the blit pair instead of the forward shader. Turning it back
    /// off resets the pair to neutral before the next live frame.
    pub fn set_look_probes(&mut self, on: bool) {
        if self.look_probes && !on {
            self.neutral_fog_pending = true;
        }
        self.look_probes = on;
    }

    /// True while the opt-in look probes are on.
    pub fn look_probes(&self) -> bool {
        self.look_probes
    }

    /// Capture-bool for the look probes: `--look-probes` or an armed
    /// capture enables the blocking probes (the capture sidecar needs
    /// measured sky_ms); live without capture stays non-blocking. Pure
    /// over the same inputs, so the binary pins it without a GPU.
    /// Caller: `crates/game/src/bin/arena.rs`.
    pub fn look_probes_for_capture(flag: bool, capture: bool) -> bool {
        flag || capture
    }

    /// Turn 4x MSAA on or off on the window leg (default off). Off keeps
    /// today's 1x path; on notes the 4x shape until slice 2 builds the
    /// MSAA targets plus resolve. Copyable state, no per-frame allocation,
    /// never waits on the GPU.
    pub fn set_msaa_enabled(&mut self, on: bool) {
        if self.msaa.enabled != on {
            self.msaa.enabled = on;
            self.targets = None;
        }
    }

    /// True while 4x MSAA is on.
    pub fn msaa_enabled(&self) -> bool {
        super::msaa::msaa_enabled(&self.msaa)
    }

    /// Blocking-call tally per frame kind since the presenter was built.
    /// A healthy window run reads zero maps and zero waits on live frames;
    /// capture frames (the offscreen twin) block by design.
    pub fn wait_stats(&self) -> WaitStats {
        self.waits
    }

    /// GPU pass timing (forward plus blit) as p50 and p99 over the frames
    /// resolved so far; `None` when the adapter lacks timestamp queries
    /// or before the first result lands. Results arrive a few frames late.
    pub fn gpu_timing_report(&self) -> Option<GpuTimingReport> {
        self.gpu_timer.as_ref().and_then(GpuTimer::report)
    }

    /// True when pass timestamps were requested and the device grants them.
    pub fn gpu_timing_supported(&self) -> bool {
        self.gpu_timer.is_some()
    }

    /// Mean wall time of the swapchain acquire in us over acquired frames:
    /// how long the CPU parked waiting for the display (vsync-bound
    /// present modes park here).
    pub fn acquire_mean_us(&self) -> u64 {
        self.acquire_us / self.acquire_frames.max(1)
    }

    /// Present mode the surface was configured with (`None` without a
    /// surface).
    pub fn present_mode_name(&self) -> Option<String> {
        self.surface_config
            .as_ref()
            .map(|config| format!("{:?}", config.present_mode))
    }

    /// Run the look probes (bind probes plus IBL chart, each a blocking
    /// GPU readback that feeds no pixel) on one surface frame in `every`
    /// instead of every frame; `0` and `1` probe every frame. The first
    /// surface frame always probes, so a broken bind still fails loudly
    /// at startup. The offscreen twin (captures) always probes. Skipped
    /// frames report zero look and IBL passes. Only matters under
    /// [`Presenter::set_look_probes`]; the default live frame has no probes.
    pub fn set_probe_every(&mut self, every: u32) {
        self.probe_every = u64::from(every.max(1));
    }

    /// Refresh the RL9 fog plus contact pair on one surface frame in
    /// `every` (`0`/`1`: every frame); frames in between blit the last
    /// pair and report zero fog and GTAO passes. Fresh targets (first
    /// frame, resize) always refresh. The offscreen twin always refreshes.
    /// Only matters under [`Presenter::set_look_probes`].
    pub fn set_fog_every(&mut self, every: u32) {
        self.fog_every = u64::from(every.max(1));
    }

    /// Frames actually presented.
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    /// Stage one HP bar (back plus fill) in 0..1 space; false when full.
    /// Window leg only; no GPU work this slice.
    pub fn push_hp_bar(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, fraction: f32) -> bool {
        self.overlay.push_hp_bar(x0, y0, x1, y1, fraction)
    }

    /// Drop staged HUD quads; the fixed backing stays for the next frame.
    /// Window leg only; the smoke stages per presented tick so a long run
    /// never fills the 128-quad ring. Caller: `crates/game/src/windowed.rs`.
    pub fn clear_overlay(&mut self) {
        self.overlay.clear();
    }

    /// Staged HUD quads in push order in 0..1 space. Window leg only.
    pub fn overlay_rects(&self) -> &[overlay::Quad] {
        self.overlay.quads()
    }

    /// Adapter name behind this presenter.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Driver string behind this presenter.
    pub fn adapter_driver(&self) -> &str {
        &self.adapter_driver
    }

    /// Per-frame walk probe over the load-warmed scene keys.
    /// Calls only `get` + `poll_ready` (through `walk_ready`), never
    /// compiles, and borrows the load-time key set so steady-state
    /// frames allocate nothing. The frame loop asserts this in debug;
    /// release keeps the zero-cost read.
    pub fn pipeline_walk_ready(&self) -> bool {
        self.pipelines.walk_ready(&self.pipeline_keys)
    }

    /// RP6 selection for the WIN1 loop: TAA at full controller rate,
    /// the SMAA fallback once the RP5 hysteresis steps the scale down.
    /// Pure read of the controller; the window build calls
    /// [`compile_taa_off`] on the fallback arm.
    pub fn aa_choice(&self, controller: &crate::aa_scale::ScaleController) -> AaChoice {
        super::super::select_aa(controller)
    }

    /// RP6 TAA-off compile at the controller scale over the live swap
    /// size: the RP5 seam sizes the render target from the controller
    /// hysteresis and re-checks the SMAA-first order. Returns the
    /// compiled graph plus the render size it was built at, so the
    /// window build renders the TAA-off path at the controller scale.
    /// A suspended (zero-area) swap fails loudly; last-good is kept.
    pub fn compile_taa_off(
        &self,
        controller: &crate::aa_scale::ScaleController,
        chain: &crate::post::PostChain,
    ) -> Result<(crate::graph::CompiledGraph, u32, u32), PresentError> {
        let (width, height) = self.swap.size();
        super::super::compile_taa_off_at_controller_scale(controller, width, height, chain)
    }

    pub(crate) fn check_device(&self) -> Result<(), PresentError> {
        if let Some(reason) = &self.device_lost {
            return Err(PresentError::DeviceLost(reason.clone()));
        }
        Ok(())
    }

    /// Shared [`gtao::LookFrame`] for the look tranche: the swap size as
    /// viewport, the presented count as frame index, the EV100 noon
    /// anchor and the arena sun. Valid by construction on live sizes.
    pub(crate) fn look_frame(&self) -> gtao::LookFrame {
        let (width, height) = self.swap.size();
        gtao::LookFrame {
            frame_index: self.frames_presented,
            viewport_px: [width.max(1), height.max(1)],
            exposure_ev100: 12.0,
            sun_dir: [0.3, 1.0, 0.2],
        }
    }

    /// Pinhole camera matching the swap size for the RP3 decal pass:
    /// focal length equals the frame height (a ~53-degree vertical
    /// field), arena view range. Valid by construction on live sizes.
    pub(crate) fn look_cam(&self) -> decals::Pinhole {
        let (width, height) = self.swap.size();
        decals::Pinhole {
            width_px: width.max(1),
            height_px: height.max(1),
            focal_px: height.max(1) as f32,
            near_m: 0.5,
            far_m: 60.0,
        }
    }

    /// GTAO kernel for one live frame: the arena kernel (8 px
    /// radius, 2 m falloff, full strength) over the frame size.
    /// Zero-area and over-bound frames have no feed and fail loudly;
    /// the caller renders those without GTAO. Plain data, no allocation.
    pub fn gtao_config_for_frame(
        width_px: u32,
        height_px: u32,
    ) -> Result<gtao::GtaoConfig, PresentError> {
        if SizeClass::classify(width_px, height_px) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize {
                width: width_px,
                height: height_px,
            });
        }
        let config = gtao::GtaoConfig {
            width_px,
            height_px,
            radius_px: 8.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        config.validate().map_err(gtao_err)?;
        Ok(config)
    }
    /// Fog kill switch for A/B evidence: off renders the unfogged frame (a
    /// zero density slope in the forward and sky shaders on the window
    /// path, a neutral fog pair on the capture twin and the probe
    /// composite). Default on.
    pub fn set_fog_enabled(&mut self, on: bool) {
        self.fog_enabled = on;
    }

    /// RL9 kill switch state.
    pub fn fog_enabled(&self) -> bool {
        self.fog_enabled
    }

    /// LF4 bloom kill switch for A/B evidence: off keeps today's frame
    /// bit-identical (flag `0.0`), on lets the gate decide per frame.
    /// Default on. Presentation only, never hashed.
    /// Caller: `crates/render/src/present/gpu_present/submit.rs` live encode.
    pub fn set_bloom_enabled(&mut self, on: bool) {
        self.bloom.enabled = on;
    }

    /// LF4 bloom switch state.
    pub fn bloom_enabled(&self) -> bool {
        self.bloom.enabled
    }

    /// RL9 cached fog -> forward wiring (`None` until the first live fog
    /// frame). The order is fixed to fog-height, contact-shadow,
    /// forward by the compile seam; the window submit executes the same
    /// shape (both fog passes after forward, composited in the blit).
    pub fn fog_plan(&self) -> Option<&crate::graph::CompiledGraph> {
        self.fog_plan.as_ref().map(|(_, _, graph)| graph)
    }

    /// RL9 evidence: mean fog factor over the top/bottom frame halves
    /// (valid after the first live fog frame; 0/0 while held or off).
    pub fn fog_means(&self) -> (f32, f32) {
        (self.fog_top_mean, self.fog_bottom_mean)
    }

    /// RL9 evidence: minimum contact AO over the frame (below 1 proves
    /// shadowing ran; 1.0 while held or off).
    pub fn fog_contact_min(&self) -> f32 {
        self.fog_contact_min
    }

    /// RL9 evidence: mean contact AO over the feed top/bottom halves
    /// (1.0/1.0 while held or off). The ramp proof normalizes by these:
    /// fully fogged bands read ramp times AO.
    pub fn fog_ao_means(&self) -> (f32, f32) {
        (self.fog_ao_top_mean, self.fog_ao_bottom_mean)
    }

    /// RL9 evidence: fog submit wall time in us (FOG+CONTACT bars).
    pub fn fog_submit_us(&self) -> u64 {
        self.fog_submit_us
    }

    /// GTAO evidence: submit wall time in us (0.8 ms bar).
    pub fn gtao_submit_us(&self) -> u64 {
        self.gtao_submit_us
    }

    /// RL9 fog params ramped from the bound RL8 look bake (one bake, three
    /// readers: skybox, IBL ambient and this ramp can never drift). No
    /// allocation per frame: reads the precomputed fog triple off the
    /// cached bake.
    pub(crate) fn look_fog_params(&self) -> Result<fog::FogParams, PresentError> {
        super::super::fog_params_for_look_bake(&self.sky_look)
    }

    /// RA1 skybox bind (RL8 reader 1 of 3): bilinear sample of the bound
    /// look bake along `dir`. CPU twin of the forward skybox chunk; the
    /// device bind samples the same bake. Row order is the bake's (row 0
    /// at the zenith); the RA1 bottom-up blit finding is untouched.
    pub fn skybox_sample(&self, dir: [f32; 3]) -> Result<[f32; 3], PresentError> {
        sky::sample_look_skybox(&self.sky_look, dir).map_err(sky_err)
    }

    /// RL2 IBL ambient (RL8 reader 2 of 3) from the same bound bake the
    /// RA1 skybox samples. No allocation per call.
    pub fn ibl_ambient(&self) -> [f32; 3] {
        sky::look_ibl_ambient(&self.sky_look)
    }

    /// Size the RL9 depth readback staging for one live frame; reused
    /// across matching-shape frames (steady-state sizes nothing).
    pub(crate) fn ensure_depth_rb(&mut self, width: u32, height: u32) {
        let fresh = match &self.depth_rb {
            Some((w, h, _)) => *w != width || *h != height,
            None => true,
        };
        if !fresh {
            return;
        }
        let padded = padded_depth_row(width);
        self.creates.buffer();
        self.depth_rb = Some((
            width,
            height,
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ra1-depth-readback"),
                size: (padded as u64) * (height as u64),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        ));
    }

    /// Compile the RL9 fog -> forward wiring once per shape; matching
    /// frames reuse the cached plan (steady-state allocates nothing).
    pub(crate) fn ensure_fog_plan(
        &mut self,
        params: &fog::FogParams,
        contact: &fog::ContactConfig,
        width: u32,
        height: u32,
    ) -> Result<(), PresentError> {
        let fresh = match &self.fog_plan {
            Some((w, h, _)) => *w != width || *h != height,
            None => true,
        };
        if fresh {
            let graph = super::super::compile_fog_for_presenter(params, contact)?;
            self.fog_plan = Some((width, height, graph));
        }
        Ok(())
    }

    /// Evidence ms to whole microseconds for the receipt rows.
    /// Non-finite or non-positive input reads 0; overflow saturates.
    /// Named conversion so the submit path never casts floats inline.
    pub(crate) fn ms_to_us(ms: f64) -> u64 {
        if !ms.is_finite() || ms <= 0.0 {
            return 0;
        }
        if ms >= 1.0e15 {
            return u64::MAX;
        }
        u64::try_from(std::time::Duration::from_secs_f64(ms / 1000.0).as_micros())
            .unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod presenter_adapter_tests {
    #[test]
    fn msaa_toggle_drops_targets() {
        let mut presenter = match crate::present::Presenter::init_offscreen() {
            Ok(p) => p,
            Err(crate::present::PresentError::NoAdapter) => {
                eprintln!("SKIPPED msaa toggle: no adapter");
                return;
            }
            Err(e) => panic!("presenter init failed: {e}"),
        };
        assert!(!presenter.msaa_enabled(), "MSAA defaults off");
        presenter.resize(64, 64).expect("size applies");
        presenter.ensure_targets(64, 64);
        assert!(presenter.targets.is_some(), "targets exist before toggle");
        presenter.set_msaa_enabled(true);
        assert!(presenter.msaa_enabled(), "switch reads back on");
        assert!(presenter.targets.is_none(), "toggle on drops targets");
        presenter.ensure_targets(64, 64);
        assert!(presenter.targets.is_some(), "targets rebuild after toggle");
        presenter.set_msaa_enabled(true);
        assert!(presenter.targets.is_some(), "unchanged flag keeps targets");
        presenter.set_msaa_enabled(false);
        assert!(!presenter.msaa_enabled(), "switch reads back off");
        assert!(presenter.targets.is_none(), "toggle off drops targets");
    }

    #[test]
    fn present_mode_prefers_short_wait_when_ready() {
        let picked = crate::present::Presenter::pick_present_mode(&[
            wgpu::PresentMode::Fifo,
            wgpu::PresentMode::Mailbox,
        ]);
        assert_eq!(picked, wgpu::PresentMode::Mailbox);
    }

    #[test]
    fn present_mode_keeps_old_path_when_not_ready() {
        let picked = crate::present::Presenter::pick_present_mode(&[wgpu::PresentMode::Fifo]);
        assert_eq!(picked, wgpu::PresentMode::Fifo);
        let picked = crate::present::Presenter::pick_present_mode(&[]);
        assert_eq!(picked, wgpu::PresentMode::Fifo);
    }
}

#[cfg(test)]
mod forward_normal_tests {
    use super::{forward_normal_layout_entries, normal_fallback_rgba};

    #[test]
    fn normal_layout_covers_shader_bindings() {
        assert!(
            crate::v1_gpu::FORWARD_PBR_WGSL
                .contains("@group(0) @binding(17) var normal_map"),
            "forward shader declares normal_map at 17"
        );
        assert!(
            crate::v1_gpu::FORWARD_PBR_WGSL
                .contains("@group(0) @binding(18) var normal_smp"),
            "forward shader declares normal_smp at 18"
        );
        let entries = forward_normal_layout_entries();
        assert_eq!(entries[0].binding, 17, "layout carries normal texture at 17");
        assert_eq!(entries[1].binding, 18, "layout carries normal sampler at 18");
        for entry in &entries {
            assert_eq!(
                entry.visibility,
                wgpu::ShaderStages::FRAGMENT,
                "normal entries are fragment-only"
            );
            assert!(entry.count.is_none(), "normal entries are not arrays");
        }
        match entries[0].ty {
            wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled,
            } => {
                assert_eq!(
                    sample_type,
                    wgpu::TextureSampleType::Float { filterable: true },
                    "normal map is filterable float"
                );
                assert_eq!(
                    view_dimension,
                    wgpu::TextureViewDimension::D2,
                    "normal map is a 2D view"
                );
                assert!(!multisampled, "normal map is not multisampled");
            }
            ref other => panic!("binding 17 is a texture, got {other:?}"),
        }
        match entries[1].ty {
            wgpu::BindingType::Sampler(kind) => {
                assert_eq!(
                    kind,
                    wgpu::SamplerBindingType::Filtering,
                    "normal sampler filters"
                );
            }
            ref other => panic!("binding 18 is a sampler, got {other:?}"),
        }
    }

    #[test]
    fn neutral_fallback_keeps_geometric_normal() {
        assert_eq!(
            normal_fallback_rgba(),
            [128, 128, 255, 255],
            "neutral texel is 128,128,255,255"
        );
        let raw = normal_fallback_rgba();
        let texel = [
            f32::from(raw[0]) / 255.0,
            f32::from(raw[1]) / 255.0,
            f32::from(raw[2]) / 255.0,
        ];
        let geom = [0.0, 0.0, 1.0];
        let decoded =
            super::super::forward_bind::decode_normal(texel, geom).expect("neutral decodes");
        let mapped = super::super::forward_bind::world_normal(
            geom,
            [1.0, 0.0, 0.0, 1.0],
            decoded,
        )
        .expect("neutral maps");
        let dot = mapped[0] * geom[0] + mapped[1] * geom[1] + mapped[2] * geom[2];
        assert!(
            dot > 0.999,
            "neutral keeps the geometric normal, dot {dot} is near 1"
        );
    }

    #[test]
    fn white_texel_tilts_and_non_finite_refuses() {
        let geom = [0.0, 0.0, 1.0];
        let white = super::super::forward_bind::decode_normal([1.0, 1.0, 1.0], geom)
            .expect("white decodes");
        let mapped = super::super::forward_bind::world_normal(
            geom,
            [1.0, 0.0, 0.0, 1.0],
            white,
        )
        .expect("white maps");
        let dot = mapped[0] * geom[0] + mapped[1] * geom[1] + mapped[2] * geom[2];
        assert!(
            dot < 0.99,
            "white tilts the normal, dot {dot} stays below neutral"
        );
        assert!(
            super::super::forward_bind::decode_normal([f32::NAN, 0.5, 0.5], geom).is_err(),
            "NaN texel refuses"
        );
    }
}
