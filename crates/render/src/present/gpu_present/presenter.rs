//! presenter: `Presenter` assembly, lifecycle and cache accessors.
//!
//! Units: pixels for sizes, GPU handles for cached entries. Methods that
//! encode or submit frames live in `submit`; free helpers there too. The
//! per-frame fog and GTAO tranche lives in `fog_tranche`.
use super::super::fog_fwd::{FogFwd, FOG_FWD_UNIFORM_BYTES};
use super::super::*;
use super::forward_group::{forward_bind_with_sky, ForwardBindSources};
use super::frame::{gtao_err, sky_err, GpuMesh, GpuTexture, Targets};
use super::msaa::{
    msaa_effective_sample_count, msaa_sample_count, msaa_target_desc, msaa_transient_bytes,
    pipeline_multisample, MsaaConfig, MsaaSupport, MSAA_SAMPLE_COUNT,
};
use super::submit::{
    blit_pipeline, forward_pipeline, forward_skinned_pipeline, material_entry, padded_depth_row,
    skin_joints_entry, skin_params_entry, sky_pipeline, uniform_buffer, uniform_entry,
    MATERIAL_RING_BYTES, MATERIAL_RING_LEN, SKIN_BLOCKS, SKIN_BLOCK_BYTES, SKIN_BLOCK_JOINTS,
    SKIN_PARAMS_BYTES,
};
use super::timing::GpuTimer;
use super::{BLIT_WGSL, DEPTH_FORMAT, HDR_FORMAT, OUTPUT_FORMAT};
use crate::v1::{self};
use crate::v1_gpu::FORWARD_PBR_WGSL;
use crate::{decals, fog, gtao, ibl, material_layer, overlay, particles_gpu, sky};
use std::collections::BTreeMap;

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
    /// How this adapter was picked and whether that was a fallback. A caller
    /// that opened the device itself reports source `caller`.
    pub(crate) backend_selection: crate::adapter::BackendSelection,
    pub(crate) forward_layout: wgpu::BindGroupLayout,
    /// The one forward shader module: static, skinned and sky entries plus the
    /// look guard. Kept so a tier change can rebuild the forward pipelines at
    /// another guard preset without re-reading the source.
    pub(crate) forward_shader: wgpu::ShaderModule,
    pub(crate) forward_hdr: wgpu::RenderPipeline,
    /// 4x MSAA forward pipeline for the on path: same shader plus layout
    /// as `forward_hdr` with count 4; bound when `targets.msaa` is `Some`.
    pub(crate) forward_hdr_msaa: wgpu::RenderPipeline,
    /// `forward_hdr` with the `vs_skinned` vertex entry over the 80-byte
    /// skinned vertex: bound for a group whose mesh has a skin block.
    pub(crate) forward_skinned: wgpu::RenderPipeline,
    /// 4x MSAA twin of `forward_skinned` (one HDR target, like the static one).
    pub(crate) forward_skinned_msaa: wgpu::RenderPipeline,
    /// LF3 landing 2: the depth-only twin of the forward pipeline. The
    /// depth prepass writes the frame's depth before the forward pass, so
    /// the GTAO kernel can build the occlusion field the forward pass
    /// scales its ambient by.
    pub(crate) depth_pipeline: wgpu::RenderPipeline,
    /// The skinned twin of `depth_pipeline` (`vs_skinned` over the 80-byte
    /// skinned vertex).
    pub(crate) depth_skinned_pipeline: wgpu::RenderPipeline,
    /// Layout of the occlusion field group (group 1): the frame's GTAO
    /// output, bound once per forward pass.
    pub(crate) ao_layout: wgpu::BindGroupLayout,
    /// Joint palette: `SKIN_BLOCKS` blocks of 64 matrices, read by
    /// `vs_skinned` at forward binding 7 through a per-draw dynamic offset.
    /// One `write_skin_block` per skinned body per tick fills a block.
    pub(crate) skin_joint_buf: wgpu::Buffer,
    /// `SkinParams` uniform (forward binding 8): `has_skin` is 1, written
    /// once at build, so `vs_skinned` always blends.
    pub(crate) skin_params_buf: wgpu::Buffer,
    /// LF5: the previous joint palette (binding 22, same block layout).
    pub(crate) prev_skin_joint_buf: wgpu::Buffer,
    /// What each skin block last held: `write_skin_block` publishes it.
    skin_prev: Vec<[[f32; 4]; 4]>,
    /// True once a block saw its first palette (zero motion, not a jump).
    skin_prev_seen: [bool; SKIN_BLOCKS],
    /// Skin block of every resident mesh uploaded through
    /// `upload_skinned_mesh`. A mesh key listed here holds 80-byte skinned
    /// vertices and draws through `forward_skinned`; every other key holds
    /// the 48-byte static vertex.
    pub(crate) skin_blocks: BTreeMap<MeshKey, u8>,
    /// LF8 shading family per mesh; an absent key is stone.
    pub(crate) material_kinds: BTreeMap<MeshKey, crate::material_kind::MaterialKind>,
    pub(crate) blit_layout: wgpu::BindGroupLayout,
    pub(crate) blit_shader: wgpu::ShaderModule,
    pub(crate) blit_sampler: wgpu::Sampler,
    /// Filtering sampler for per-draw base-color textures (shared by
    /// every draw bind; created once, never per frame).
    pub(crate) base_sampler: wgpu::Sampler,
    /// Repeat-addressed twin of `base_sampler` for material sets, whose UVs
    /// tile (VR2). Plain draws keep the clamped one.
    pub(crate) repeat_sampler: wgpu::Sampler,
    /// Blit into the LDR twin; built once (no per-frame pipeline alloc).
    pub(crate) twin_pipeline: wgpu::RenderPipeline,
    pub(crate) surface_pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// HP-bar overlay pipelines by target format, built in the warm-up and not per frame (WEB13).
    pub(crate) overlay_pipes: std::sync::Mutex<Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
    pub(crate) surface: Option<wgpu::Surface<'static>>,
    pub(crate) surface_config: Option<wgpu::SurfaceConfiguration>,
    pub(crate) swap: v1::SwapState,
    pub(crate) targets: Option<Targets>,
    pub(crate) cam_buf: wgpu::Buffer,
    /// LF5: the previous frame's view-projection (binding 21), a frame behind `cam_buf`.
    pub(crate) prev_cam_buf: wgpu::Buffer,
    /// Last un-jittered camera bytes `prev_cam_buf` took; `None` before the first frame.
    pub(super) prev_cam_bytes: Option<[u8; 64]>,
    /// LF5b: the jitter switch, its un-jittered matrix (23) and last images.
    pub(crate) jitter: super::motion::JitterState,
    /// LF5c: the live TAA resolve history pair, pipeline and switch.
    pub(crate) taa: super::taa_resolve::TaaResolve,
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
    /// Neutral 1x1 ORM fallback (linear 255,255,255): occlusion open, the
    /// roughness and metallic factors pass through unscaled (VR2).
    pub(crate) orm_fallback: GpuTexture,
    /// Resident material sets (normal map + ORM + their forward bind group),
    /// keyed by the normal-map key a draw names. VR2.
    pub(crate) map_sets: Vec<MapSet>,
    /// Material set chosen for each draw of the frame: an index into
    /// `map_sets`, `None` for a draw that names no normal map. Reused scratch.
    pub(crate) draw_sets: Vec<Option<usize>>,
    /// Bind group chosen for each draw of the frame: an index into
    /// `texture_binds`, `None` for the fallback. Reused scratch.
    pub(crate) draw_binds: Vec<Option<usize>>,
    /// Buffers and bind groups created, per frame and in total.
    pub(crate) creates: CreateTally,
    pub(crate) meshes: CpuCache<MeshKey, GpuMesh>,
    /// Index fingerprint per resident mesh: same-shape re-uploads with the
    /// same topology rewrite vertices in place with no new buffers.
    pub(crate) mesh_index_hash: BTreeMap<MeshKey, u64>,
    pub(crate) textures: CpuCache<TextureKey, GpuTexture>,
    pub(crate) device_lost: Option<String>,
    pub(crate) frames_presented: u64,
    /// Draws the last staged frame submitted after the EG38 sphere cull: the
    /// list length minus the draws whose world bound is outside the frame
    /// frustum. Evidence only, never hashed.
    pub(crate) submitted_draws: usize,
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
    /// frame, same backing the fog horizon reads. Held for its lifetime: the
    /// bound view is the only reader.
    pub(crate) _sky_view: wgpu::Texture,
    /// View over `sky_view` bound at forward 15 for the sky pass.
    pub(crate) sky_view_view: wgpu::TextureView,
    /// LF1 live exposure: EV100 scale from the one `sky_forward_feed`
    /// call, staged into frame materials. Presentation only.
    pub(crate) sky_exposure: f32,
    /// The sky rig's EV100 anchor: the day rig's 12, the night rig's own
    /// moonlit value. The live sky feed reads it, so the frame's exposure
    /// follows the rig that baked it and never drifts from the sky.
    pub(crate) sky_exposure_ev100: f32,
    /// RL4 sky bind state (steady-state submits size nothing).
    pub(crate) sky_sub: sky::SkySubmitter,
    /// RL6 layer chart bind state (steady-state submits size nothing).
    pub(crate) layer_sub: material_layer::LayerSubmitter,
    /// RP2 live particle pool stepped once per presented frame.
    pub(crate) particles: particles_gpu::ParticlePool,
    /// FEEL5 fight particles: built on the first event, stepped per sim tick.
    pub(crate) fight_fx: Option<particles_gpu::ParticlePool>,
    /// RP2 particle bind state (sizes only when the word shape moves).
    pub(crate) particle_sub: particles_gpu::ParticleSubmitter,
    /// LF14 weapon-trail strip for the coming frame: flat verts plus the vert
    /// count of each slot's strip. Replaced every staged frame; empty clears.
    pub(crate) trail_verts: Vec<crate::draw::TrailStripVert>,
    /// Per-slot vert counts for `trail_verts`, so one draw never bridges two
    /// fighters.
    pub(crate) trail_lens: Vec<u32>,
    /// LF14 trail bind state (built on the first non-empty strip).
    pub(crate) trail_sub: crate::trail_draw_gpu::TrailSubmitter,
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
    /// Live GTAO on the forward-stored depth (LF3): both compute kernels
    /// plus the neutral-reset flag; the frame-size resources ride the targets.
    pub(crate) gtao_gpu: super::gtao_live::GtaoGpu,
    /// Dual-filter bloom (RL36): the three compute kernels; the mip textures
    /// and uniforms ride the targets, built once per frame size.
    pub(crate) bloom_gpu: super::bloom_dual::BloomDualGpu,
    /// GS43: contact-shadow strength (1 on, 0 off). Strength 0 copies the
    /// GTAO field through, so an off capture is byte-equal to no pass.
    pub(crate) contact_strength: f32,
    /// The device meter: the 64-bin histogram kernel over the HDR target
    /// and its one-frame-late readback. Built once with the device; the
    /// steady frame only dispatches and reads.
    pub(super) auto_exp_gpu: super::auto_exposure_gpu::AutoExposureGpu,
    /// The adapted stops: the meter state the frame ticks. One state per
    /// presenter, carried across frames.
    pub(crate) auto_exp_state: crate::post::AutoExposure,
    /// The last histogram the meter read, for the frame report.
    pub(crate) auto_exp_hist: Option<crate::post::ExposureHistogram>,
    /// The meter switch (off by default): off leaves every frame exactly
    /// as it was, on meters the HDR target and adapts the exposure.
    pub(crate) auto_exp_enabled: bool,
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
    /// RL9 CPU decode scratch for the depth readback (resized on shape
    /// change only, reused every fog frame so steady-state fog frames
    /// allocate nothing for the decode).
    pub(crate) depth_decode: Vec<f32>,
    /// RL9 scratch for fog/ao byte staging (resized on shape change
    /// only, reused across both uploads and the neutral clears).
    pub(crate) fog_scratch: Vec<u8>,
    /// RL9 kill switch (A/B evidence and operator fallback); default on.
    pub(crate) fog_enabled: bool,
    /// The blit fog pair holds non-neutral data (a tranche upload or a
    /// live-path AO upload) and must be reset once before it can be
    /// trusted as neutral. Fresh targets start neutral, `run_fog_tranche`
    /// sets this, `clear_fog_targets` clears it; the fog-off branches
    /// skip their re-upload while it stays false.
    pub(crate) fog_pair_dirty: bool,
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
    /// Packed live contact feet (`contact_feet_bytes`), written behind the sun
    /// splits of `twin_shadow_buf` every frame. All zero until the game feeds it.
    pub(crate) contact_feet: [u8; crate::contact_feet::CONTACT_FEET_BYTES],
    /// EG11 live sun depth map: one cached texture plus pipeline pair the
    /// live forward submit records before the forward draws, so a live frame
    /// pays the depth pass, never a texture plus pipeline build.
    /// Presentation only, never hashed.
    pub(crate) sun_sub: super::sun_shadow::SunShadowSubmitter,
    /// Shadowed spot lights (flashlights and lamps) the frame's spot pass
    /// bakes into its shadow atlas. Empty by default: no shipped scene
    /// feeds spots yet, so the live frame skips the pass.
    pub(crate) spot_lights: Vec<crate::spot::SpotLight>,
    /// Torch point lights the scene feeds: the placement actors become
    /// lights. Empty by default, and the live frame then uses the arena's
    /// built-in pair, so a scene that feeds none still lights the arena.
    pub(crate) torch_lights: Vec<crate::torch_light::TorchLight>,
    /// World triangles the spot pass rasterizes into the atlas (the walls
    /// and props that block a cone).
    pub(crate) spot_occluders: Vec<crate::spot::OccluderTri>,
    /// Atlas shape the spot pass bakes: tiles per side and tile edge.
    pub(crate) spot_config: crate::spot::SpotAtlasConfig,
    /// Device bake state for the spot atlas (pipeline, slot uniforms and
    /// the atlas texture): built on the first frame that feeds spots.
    pub(crate) spot_atlas: Option<crate::spot::SpotAtlasGpu>,
    /// The last spot receipt of the live frame: the row the frame report
    /// carries. `None` until the first frame baked an atlas.
    pub(crate) spot_row: Option<crate::spot::SpotEncode>,
    /// The quality tier applied by `apply_quality`; the live frame reads its cascade, AO and
    /// texture knobs here. Starts at the top tier, which changes nothing.
    pub(crate) quality: crate::quality::Knobs,
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
    /// What the adapter can build for EG13's 4x pair (colour, depth,
    /// resolve), read once at build from its per-format multisample flags.
    /// The switch reads it, so a pair the adapter cannot build stays on the
    /// 1x path instead of failing texture creation. Presentation only.
    pub(crate) msaa_support: MsaaSupport,
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
    /// 4x MSAA sky pipeline for the on path: same shader plus layout
    /// as `sky_pipeline` with count 4; bound when `targets.msaa` is `Some`.
    pub(crate) sky_msaa: wgpu::RenderPipeline,
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
    /// Wall time of the most recent surface acquire (us). The swapchain
    /// wait of one frame, so a perf run can name the frame that blocked.
    pub(crate) last_acquire_us: u64,
    /// The fog pair holds stale probe output and must be reset to neutral
    /// before the next live frame blits it.
    pub(crate) neutral_fog_pending: bool,
    /// CPU HUD quad ring in 0..1 space; drained by the next slice.
    pub(crate) overlay: overlay::OverlayRing,
    /// Debug line list of the frame, replaced by `push_debug_lines`; the
    /// live line pass reads it (PW13). Empty until the game pushes.
    pub(crate) debug_lines: crate::debug_draw::LineBatch,
    /// The after-blit debug line switch (`debug_line_enable`); off by default
    /// so a frame with no debug view stays bit-identical.
    pub(crate) debug_line_on: bool,
    /// LF4 slice 1 bloom state: enable flag plus pass thresholds, default
    /// on. Init only, no per-frame allocation; reuses the HDR target.
    pub(crate) bloom: BloomState,
    /// A frame capture asked for: waiting for its frame, or for its map (see `capture`).
    pub(crate) capture: super::capture::CaptureSlot,
    /// Validation-scope answers still on their way from the browser (browser build only; a
    /// native device answers at once and `pop_scope` blocks on it).
    #[cfg(target_arch = "wasm32")]
    pub(crate) scopes: super::scope_ledger::ScopeLedger,
}

/// One resident material set (VR2): a linear tangent-space normal map and an
/// ORM map (R occlusion, G roughness, B metallic) over one base-color view,
/// with the forward bind group built once at upload. A draw names it by
/// `DrawItem::normal_map`.
pub(crate) struct MapSet {
    /// The normal-map key a draw names to select this set.
    pub(crate) key: TextureKey,
    /// Keeps the allocations alive behind the bind group's views.
    pub(crate) _textures: [GpuTexture; 2],
    /// Forward group: base view, normal view, ORM view and the shared frame buffers.
    pub(crate) bind: wgpu::BindGroup,
}

/// Neutral ORM texel in 0..=255: 255,255,255 is open occlusion with the
/// roughness and metallic factors passing through unscaled.
fn orm_fallback_rgba() -> [u8; 4] {
    [255, 255, 255, 255]
}

/// Layout entry for the VR2 ORM map at group 0 binding 19 (filterable 2D
/// texture; the sampler is the normal map's at 18). Cold layout only.
fn forward_orm_layout_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 19,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

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

/// Layout of the LF3 occlusion field group (group 1, binding 0): the
/// full-resolution R32Float field the forward pass samples for its
/// ambient. The field is unfilterable, so the shader reads it with an
/// exact texel load.
pub(super) fn forward_ao_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ra1-ao-field"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    })
}

/// The four forward pipelines at one look guard preset: 1x and 4x MSAA, static
/// and skinned. `constants` comes from `quality::look_guard_constants`, so a
/// tier change rebuilds them by calling this again with other constants.
pub(super) fn forward_pipelines(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    ao_layout: &wgpu::BindGroupLayout,
    constants: &[(&str, f64)],
) -> [wgpu::RenderPipeline; 4] {
    // EG13 on path: 4x multisample with one shading run per pixel (cost is
    // bandwidth, not shading); off stays the 1x path. SP63: the MSAA pass
    // keeps a single HDR attachment, so its pipeline carries one target.
    // The skinned twins use `vs_skinned` over the 80-byte vertex, so a body
    // draws from bind-pose vertices plus one joint block.
    [
        forward_pipeline(
            device,
            shader,
            layout,
            ao_layout,
            &super::submit::forward_color_target_formats(),
            pipeline_multisample(&MsaaConfig::default()),
            constants,
        ),
        forward_pipeline(
            device,
            shader,
            layout,
            ao_layout,
            &[HDR_FORMAT],
            pipeline_multisample(&MsaaConfig { enabled: true }),
            constants,
        ),
        forward_skinned_pipeline(
            device,
            shader,
            layout,
            ao_layout,
            &super::submit::forward_color_target_formats(),
            pipeline_multisample(&MsaaConfig::default()),
            constants,
        ),
        forward_skinned_pipeline(
            device,
            shader,
            layout,
            ao_layout,
            &[HDR_FORMAT],
            pipeline_multisample(&MsaaConfig { enabled: true }),
            constants,
        ),
    ]
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

    /// One-texel linear RGBA8 texture: the identity a map slot binds when a
    /// draw names no map (VR2 ORM). Cold init path only; fixed bytes.
    fn linear_texel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &'static str,
        rgba: [u8; 4],
    ) -> GpuTexture {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
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
            &rgba,
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

    /// What the adapter can build for EG13's 4x pair: `MULTISAMPLE_X4` on the
    /// HDR colour format, `MULTISAMPLE_X4` on the depth format and
    /// `MULTISAMPLE_RESOLVE` on the colour format (the 1x resolve target).
    /// Owner: render lane; Caller: `Presenter::init_offscreen` plus
    /// `Presenter::init_windowed`, which hand it to `Presenter::build`.
    pub(super) fn msaa_support_for(adapter: &wgpu::Adapter) -> MsaaSupport {
        let color = adapter.get_texture_format_features(HDR_FORMAT).flags;
        let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
        MsaaSupport::from_flags(
            color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4),
            depth.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4),
            color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE),
        )
    }

    /// Build the presenter under one sky preset. `night` picks the arena
    /// night rig: the moonlit sky, its ambient and its fog, so the
    /// placement's torches are the light the frame reads. The day preset
    /// is the default and keeps every existing capture byte-equal.
    pub(super) fn build_with_preset(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: wgpu::AdapterInfo,
        support: MsaaSupport,
        night: bool,
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
            // Joint palette block (dynamic offset) plus skin params, read
            // by `vs_skinned` only; the static `vs` ignores both.
            skin_joints_entry(7),
            skin_params_entry(8),
            // LF1 live sky: the init-time sky-view texture (Rgba32Float,
            // unfilterable, manual bilinear in `fs_sky`). Free binding:
            // 9-12 owned by IBL, 13-14 the live sun map, 7-8 skin.
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
        forward_entries.push(forward_orm_layout_entry());
        forward_entries.extend_from_slice(&ibl::gpu::forward_ibl_layout_entries());
        // EG11b: the live sun depth map at 13, its comparison sampler at 14
        // and the pass frame at 20; the window `fs` entry samples the map.
        forward_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 13,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
        forward_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 14,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        });
        forward_entries.push(uniform_entry(20, wgpu::ShaderStages::FRAGMENT));
        // LF5 motion: the previous view-projection (21) and the previous
        // joint palette (22, one block picked by the same dynamic offset as
        // binding 7). Both vertex stage: the motion is built per vertex.
        forward_entries.push(uniform_entry(21, wgpu::ShaderStages::VERTEX));
        forward_entries.push(super::submit::skin_joints_entry(22));
        forward_entries.push(uniform_entry(23, wgpu::ShaderStages::VERTEX));
        let forward_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ra1-uniforms"),
            entries: &forward_entries,
        });
        // LF3 landing 2: the occlusion field lives in group 1, so the
        // forward group keeps its per-draw shape.
        let ao_layout = forward_ao_layout(&device);
        let look_constants = crate::quality::look_guard_constants(&crate::quality::knobs(
            crate::quality::Quality::High,
        ));
        let [forward_hdr, forward_hdr_msaa, forward_skinned, forward_skinned_msaa] =
            forward_pipelines(
                &device,
                &forward_shader,
                &forward_layout,
                &ao_layout,
                &look_constants,
            );
        let [depth_pipeline, depth_skinned_pipeline] = super::submit::forward_depth_pipelines(
            &device,
            &forward_shader,
            &forward_layout,
            &look_constants,
        );
        let skin_joint_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fc2b-skin-joints"),
            size: SKIN_BLOCK_BYTES * u64::try_from(SKIN_BLOCKS).unwrap_or(1),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // LF5: the previous palette, one block per body, same stride as the
        // live one so a draw passes the same dynamic offset to both.
        let prev_skin_joint_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-skin-joints-prev"),
            size: SKIN_BLOCK_BYTES * u64::try_from(SKIN_BLOCKS).unwrap_or(1),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // `SkinParams { joint_count, has_skin, pad, pad }`: always skin.
        let skin_params_buf = uniform_buffer(&device, "fc2b-skin-params", SKIN_PARAMS_BYTES);
        let mut skin_params = [0u8; 16];
        skin_params[0..4]
            .copy_from_slice(&u32::try_from(SKIN_BLOCK_JOINTS).unwrap_or(0).to_le_bytes());
        skin_params[4..8].copy_from_slice(&1u32.to_le_bytes());
        queue.write_buffer(&skin_params_buf, 0, &skin_params);
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
                // RL36: the dual-filter bloom halo, half the frame size and
                // sampled at the pixel centre by the one linear blit sampler.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
        // VR2: the same filtering, Repeat addressing, for material sets whose
        // box-projected UVs tile across a surface.
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vr2-repeat-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
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
        let orm_fallback =
            Self::linear_texel(&device, &queue, "ra1-orm-fallback", orm_fallback_rgba());
        let cam_buf = uniform_buffer(&device, "ra1-u-camera", 80);
        // LF5: the previous frame's view-projection.
        let prev_cam_buf = uniform_buffer(&device, "ra1-u-prev-camera", 64);
        let jitter = super::motion::JitterState::new(&device);
        let light_buf = uniform_buffer(&device, "ra1-u-light", 32);
        // The twin sun pass is std-only (no device work); a bad
        // shape fails init loudly, never a silent unlit twin.
        let twin_sun = crate::twin_shadow::twin_sun_pass()
            .map_err(|e| PresentError::Device(format!("rl19 twin sun: {e}")))?;
        // 32 bytes of sun splits, then the live contact feet block the forward
        // shader reads for the grounding shade (`contact_live`).
        let twin_shadow_buf = uniform_buffer(
            &device,
            "rl19-twin-shadow",
            super::contact_live::TWIN_SHADOW_BUF_BYTES,
        );
        // The day capture is the arena day rig (one sun preset feeding
        // the look bake and the capture bundle from one bake);
        // per-frame time-of-day bakes ride the same submit shape, so this
        // never grows the steady-state path. A failed bake fails init
        // loudly: the preset params are fixed valid inputs, so this is
        // unreachable unless the sky contract itself changed.
        let sky_rig = if night {
            crate::arena_day::bake_arena_night()
                .map_err(|e| PresentError::Device(format!("look night rig bake: {e}")))?
        } else {
            crate::arena_day::bake_arena_day()
                .map_err(|e| PresentError::Device(format!("look day rig bake: {e}")))?
        };
        let sky_capture = sky_rig.capture;
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
            sky::sky_forward_feed(&sky_look, sky_rig.params.exposure_ev100).map_err(sky_err)?;
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
        // The arena day sky bakes once here (the same sky the look and the fog
        // read; the photo-studio HDRI it replaces put softboxes and a white
        // cyclorama into the sand's reflections); per-frame charts sample this
        // bake, so the steady-state path never cooks. A failed bake fails init
        // loudly: the sky parameters are fixed valid inputs, so this is
        // unreachable unless the sky contract itself changed.
        let (ibl_bake, _) = if night {
            crate::ibl_arena_env::bake_arena_night()
        } else {
            crate::ibl_arena_env::bake_arena_day()
        }
        .map_err(|e| PresentError::Device(format!("look ibl bake: {e}")))?;
        // Live forward bake upload once through the bake owner.
        let (ibl_prefilter, ibl_brdf, ibl_sh_buf) =
            ibl::gpu::upload_live_bake(&device, &queue, &ibl_bake)
                .map_err(|e| PresentError::Device(format!("live ibl bake: {e}")))?;
        // Warm every scene variant before the first frame over
        // the real forward shader bytes, so the camera walk never
        // compiles mid-frame. Blobs are cache-residency markers keyed
        // by (shader, defines, layout, toolchain); device variant pipelines
        // ride their own pass rows. Load-path only: allocates the warm set.
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
        // SP63: the 1x pass carries HDR plus the id target; the MSAA pass
        // keeps its single HDR attachment.
        let sky_1x = sky_pipeline(
            &device,
            &forward_shader,
            &forward_layout,
            &super::submit::forward_color_target_formats(),
            pipeline_multisample(&MsaaConfig::default()),
        );
        let sky_msaa = sky_pipeline(
            &device,
            &forward_shader,
            &forward_layout,
            &[HDR_FORMAT],
            pipeline_multisample(&MsaaConfig { enabled: true }),
        );
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
        let sun_sub = super::sun_shadow::SunShadowSubmitter::build(&device);
        let fallback_bind = forward_bind_with_sky(
            &device,
            &ForwardBindSources {
                layout: &forward_layout,
                cam: &cam_buf,
                light: &light_buf,
                material: &material_ring,
                material_bytes: super::submit::MATERIAL_BYTES,
                shadow: &twin_shadow_buf,
                sampler: &base_sampler,
                map_sampler: &base_sampler,
                fog: &fog_fwd_buf,
                torch: &torch_buf,
                pre_view: &ibl_prefilter_view,
                brdf_view: &ibl_brdf_view,
                sh_buf: &ibl_sh_buf,
                sky_view: &sky_view_view,
                normal_view: &normal_fallback.view,
                orm_view: &orm_fallback.view,
                skin_joints: &skin_joint_buf,
                skin_params: &skin_params_buf,
                prev_cam: &prev_cam_buf,
                prev_skin_joints: &prev_skin_joint_buf,
                steady_cam: &jitter.steady_buf,
                sun_view: sun_sub.view(),
                sun_smp: sun_sub.sampler(),
                sun_buf: sun_sub.fwd_uniform(),
            },
            &base_fallback.view,
        );
        let gpu_timer = GpuTimer::new(&device, &queue);
        // EG13: the MSAA on/off switch. Off (default) keeps today's 1x HDR
        // plus depth targets and the 1x submit path; on renders the 4x HDR
        // plus depth pair and resolves into 1x HDR before the blit.
        // Never waits on the GPU.
        let msaa = MsaaConfig::default();
        debug_assert_eq!(
            msaa_sample_count(&msaa),
            1,
            "MSAA defaults off: the 1x path"
        );
        // The desc names the HDR plus depth formats with the chosen count;
        // `ensure_targets` builds the 4x pair from it (resolve before blit).
        let msaa_desc = msaa_target_desc(&msaa);
        debug_assert_eq!(msaa_desc.sample_count, 1);
        Ok(Self {
            material_ring,
            material_scratch: Vec::with_capacity(MATERIAL_RING_LEN),
            vertex_scratch: Vec::new(),
            texture_binds: Vec::new(),
            fallback_bind,
            normal_fallback,
            orm_fallback,
            map_sets: Vec::new(),
            draw_sets: Vec::with_capacity(MAX_DRAWS_PER_FRAME),
            draw_binds: Vec::with_capacity(MAX_DRAWS_PER_FRAME),
            creates: CreateTally::default(),
            submitted_draws: 0,
            look_probes: false,
            msaa,
            msaa_support: support,
            fog_fwd,
            fog_fwd_buf,
            torch_buf,
            sky_pipeline: sky_1x,
            sky_msaa,
            waits: WaitStats::default(),
            frame_waits: FrameWaits::default(),
            gpu_timer,
            acquire_us: 0,
            acquire_frames: 0,
            last_acquire_us: 0,
            neutral_fog_pending: false,
            overlay: overlay::OverlayRing::new(),
            debug_lines: crate::debug_draw::LineBatch::default(),
            debug_line_on: false,
            bloom: BloomState::on(),
            capture: super::capture::CaptureSlot::default(),
            #[cfg(target_arch = "wasm32")]
            scopes: super::scope_ledger::ScopeLedger::default(),
            sky_sub: sky::SkySubmitter::new(&device),
            layer_sub: material_layer::LayerSubmitter::new(&device),
            particles: particles_gpu::ParticlePool::new(super::quality_apply::POOL_SEED),
            fight_fx: None,
            particle_sub: particles_gpu::ParticleSubmitter::new(&device),
            trail_verts: Vec::new(),
            trail_lens: Vec::new(),
            trail_sub: crate::trail_draw_gpu::TrailSubmitter::new(),
            decal_pool: decals::DecalPool::new(),
            decal_sub: decals::DecalSubmitter::new(&device),
            fog_sub: fog::FogSubmitter::new(&device),
            gtao_sub: gtao::GtaoSubmitter::new(&device),
            gtao_gpu: super::gtao_live::GtaoGpu::new(&device),
            bloom_gpu: super::bloom_dual::BloomDualGpu::new(&device),
            contact_strength: 1.0,
            auto_exp_gpu: super::auto_exposure_gpu::AutoExposureGpu::new(&device, &queue),
            auto_exp_state: crate::post::AutoExposure::default(),
            auto_exp_hist: None,
            auto_exp_enabled: false,
            ibl_sub: ibl::gpu::IblSubmitter::new(&device),
            ibl_bake,
            ibl_prefilter,
            ibl_brdf,
            ibl_sh_buf,
            fog_plan: None,
            depth_rb: None,
            depth_decode: Vec::new(),
            fog_scratch: Vec::new(),
            fog_enabled: true,
            fog_pair_dirty: false,
            fog_top_mean: 0.0,
            fog_bottom_mean: 0.0,
            fog_contact_min: 1.0,
            fog_ao_top_mean: 1.0,
            fog_ao_bottom_mean: 1.0,
            fog_submit_us: 0,
            gtao_submit_us: 0,
            twin_sun,
            twin_shadow_buf,
            contact_feet: [0u8; crate::contact_feet::CONTACT_FEET_BYTES],
            sun_sub,
            spot_lights: Vec::new(),
            torch_lights: Vec::new(),
            spot_occluders: Vec::new(),
            spot_config: crate::spot::SpotAtlasConfig {
                grid: crate::spot::ATLAS_GRID,
                tile_px: 64,
            },
            spot_atlas: None,
            spot_row: None,
            quality: crate::quality::knobs(crate::quality::Quality::High),
            pipelines,
            pipeline_keys,
            taa: super::taa_resolve::TaaResolve::new(&device),
            device,
            queue,
            instance: None,
            adapter_name: info.name.clone(),
            adapter_driver: format!("{} {}", info.driver, info.driver_info),
            backend_selection: crate::adapter::BackendSelection::caller_opened(
                crate::presenter_pick::summary_of(&info),
            ),
            forward_layout,
            forward_shader,
            forward_hdr,
            forward_hdr_msaa,
            forward_skinned,
            forward_skinned_msaa,
            depth_pipeline,
            depth_skinned_pipeline,
            ao_layout,
            skin_joint_buf,
            skin_params_buf,
            prev_skin_joint_buf,
            skin_prev: vec![[[0.0f32; 4]; 4]; SKIN_BLOCKS * SKIN_BLOCK_JOINTS],
            skin_prev_seen: [false; SKIN_BLOCKS],
            skin_blocks: BTreeMap::new(),
            material_kinds: BTreeMap::new(),
            blit_layout,
            blit_shader,
            blit_sampler,
            base_sampler,
            repeat_sampler,
            twin_pipeline,
            surface_pipeline: None,
            overlay_pipes: std::sync::Mutex::new(Vec::new()),
            surface: None,
            surface_config: None,
            swap: v1::SwapState::new(0, 0),
            targets: None,
            cam_buf,
            prev_cam_buf,
            prev_cam_bytes: None,
            jitter,
            light_buf,
            meshes: CpuCache::new(MAX_CACHED_MESHES),
            mesh_index_hash: BTreeMap::new(),
            textures: CpuCache::new(MAX_CACHED_TEXTURES),
            device_lost: None,
            frames_presented: 0,
            probe_every: 1,
            fog_every: 1,
            sky_capture,
            sky_look,
            _sky_view: sky_view,
            sky_view_view,
            sky_exposure,
            sky_exposure_ev100: sky_rig.params.exposure_ev100,
        })
    }

    /// Open the device and queue on `adapter` (see [`Self::device_descriptor`]).
    /// Blocks: native callers only.
    pub(super) fn request_device(
        adapter: &wgpu::Adapter,
    ) -> Result<(wgpu::Device, wgpu::Queue), PresentError> {
        pollster::block_on(adapter.request_device(&Self::device_descriptor(adapter)))
            .map_err(|e| PresentError::Device(format!("request_device: {e}")))
    }

    /// Pick the swapchain present mode: the shortest acquire wait the
    /// driver offers (Immediate tears but never waits for vsync), then
    /// Pick the present mode: Fifo (vsync) by default for stable frame pacing,
    /// Immediate or Mailbox when `uncapped` for captures. Units: none.
    /// Caller: `Presenter::attach_surface`.
    pub fn pick_present_mode(modes: &[wgpu::PresentMode], uncapped: bool) -> wgpu::PresentMode {
        if uncapped {
            if modes.contains(&wgpu::PresentMode::Immediate) {
                return wgpu::PresentMode::Immediate;
            }
            if modes.contains(&wgpu::PresentMode::Mailbox) {
                return wgpu::PresentMode::Mailbox;
            }
        }
        if modes.contains(&wgpu::PresentMode::Fifo) {
            return wgpu::PresentMode::Fifo;
        }
        modes.first().copied().unwrap_or(wgpu::PresentMode::Fifo)
    }

    /// Feed the frame's shadowed spot lights and the world triangles that
    /// occlude them (the room walls and props). The next live frame bakes
    /// their shadow atlas into its own command buffer; an empty feed skips
    /// the pass, so a frame without spots costs nothing.
    /// Units: meters for positions and ranges. Presentation only, never hashed.
    /// Caller: the scene owner (the window path's light feed).
    pub fn set_spot_lights(
        &mut self,
        spots: Vec<crate::spot::SpotLight>,
        occluders: Vec<crate::spot::OccluderTri>,
    ) {
        self.spot_lights = spots;
        self.spot_occluders = occluders;
    }

    /// Feed the frame's torch point lights: the scene's placement actors
    /// become lights. An empty list keeps the arena's built-in night pair,
    /// so a scene that feeds none still lights the arena. The list is
    /// capped by the shader block (8) and validated by the frame; a
    /// refused light fails the frame loudly with the reason named.
    /// Units: meters for positions and ranges, linear RGB for color.
    /// Presentation only, never hashed.
    /// Caller: the scene owner (the window path's light feed).
    pub fn set_torch_lights(&mut self, lights: Vec<crate::torch_light::TorchLight>) {
        self.torch_lights = lights;
    }

    /// The torch lights the next frame will pack.
    pub fn torch_lights(&self) -> &[crate::torch_light::TorchLight] {
        &self.torch_lights
    }

    /// The last spot receipt of the live frame: the pass, the number of
    /// shadowed spots and the encode cost. `None` before the first frame
    /// baked an atlas (no scene feeds spots yet).
    /// Units: microseconds for the encode. Presentation only, never hashed.
    /// Caller: the frame report and the pass-truth gate.
    pub fn spot_row(&self) -> Option<crate::spot::SpotEncode> {
        self.spot_row
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
        uncapped: bool,
    ) -> Result<(), PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let adapter = Self::compatible_adapter(instance, &surface)?;
        self.attach_surface_for(instance, &adapter, surface, width, height, uncapped)
    }

    /// [`Self::attach_surface`] for a caller that already holds the adapter
    /// that supports `surface`: nothing is enumerated, so nothing blocks.
    pub fn attach_surface_for(
        &mut self,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        uncapped: bool,
    ) -> Result<(), PresentError> {
        if SizeClass::classify(width, height) != SizeClass::Sizes {
            return Err(PresentError::InvalidSize { width, height });
        }
        let caps = surface.get_capabilities(adapter);
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
            present_mode: Self::pick_present_mode(&caps.present_modes, uncapped),
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
            .map(|(adapter, _)| adapter)
    }

    /// Enumerate `instance`, keep the surface-compatible adapters and
    /// rank-pick the `E2040_ADAPTER` choice through
    /// [`crate::adapter::select`], which also reports how it was picked.
    /// Cold path only; the pick never falls back silently: an empty
    /// enumerate, a software-only list without opt-in, no compatible
    /// adapter, or a fallback under `E2040_REQUIRE_GPU=1` is `NoAdapter`.
    /// Caller: `init_windowed` plus `compatible_adapter` (the GS6 surface
    /// sites); `init_offscreen` uses `pick_adapter` (no surface to test).
    pub(super) fn pick_surface_adapter(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        backends: wgpu::Backends,
    ) -> Result<(wgpu::Adapter, crate::adapter::BackendSelection), PresentError> {
        let adapters = pollster::block_on(instance.enumerate_adapters(backends));
        let list: Vec<crate::adapter::AdapterSummary> = adapters
            .iter()
            .map(|found| crate::presenter_pick::summary_of(&found.get_info()))
            .collect();
        let compat: Vec<bool> = adapters
            .iter()
            .map(|found| found.is_surface_supported(surface))
            .collect();
        let (index, selection) = crate::adapter::select(
            &list,
            Some(&compat),
            &crate::adapter::SelectionRequest::from_env(),
        )
        .map_err(|_| PresentError::NoAdapter)?;
        let adapter = adapters
            .into_iter()
            .nth(index)
            .ok_or(PresentError::NoAdapter)?;
        Ok((adapter, selection))
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

    /// Fingerprint of an index list for the upload skip: FNV-1a over LE bytes.
    /// Pure, no allocation. Presentation only, never hashed.
    fn mesh_index_hash(indices: &[u32]) -> u64 {
        let mut h = engine_core::Fnv64::new();
        for &i in indices {
            h.write_u32(i);
        }
        h.finish()
    }

    /// Reuse verdict for a resident mesh: true only when a fingerprint is
    /// stored and the incoming one matches. Pure, no allocation.
    fn mesh_upload_should_reuse(stored: Option<u64>, incoming: u64) -> bool {
        matches!(stored, Some(known) if known == incoming)
    }

    /// Upload a mesh under `key`. The mesh is revalidated on upload (its
    /// fields are public, so hand-built data cannot sneak past), then
    /// packed into GPU buffers. A changed digest replaces the resident
    /// buffers; eviction drops the oldest entry first.
    pub fn upload_mesh(&mut self, key: MeshKey, mesh: &v1::MeshData) -> Result<(), PresentError> {
        // Skip switch: same shape plus same topology rewrites vertices in
        // place (one write, no new buffers); bytes pack into `vertex_scratch`
        // with the hash first, so a moved topology skips validation.
        let reuse_hit = match self.meshes.get(&key) {
            // A skinned key holds 64-byte vertices: never rewrite it in
            // place with the 48-byte static image.
            Some(_) if self.skin_blocks.contains_key(&key) => false,
            Some(resident) => {
                let want_vertices = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
                let want_count = u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX);
                resident.vertices == want_vertices
                    && resident.count == want_count
                    && Self::mesh_upload_should_reuse(
                        self.mesh_index_hash.get(&key).copied(),
                        Self::mesh_index_hash(&mesh.indices),
                    )
            }
            None => false,
        };
        if reuse_hit {
            let validated =
                v1::upload_mesh_tiled(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices)
                    .map_err(|e| PresentError::InvalidMesh(format!("{e}")))?;
            self.vertex_scratch.clear();
            self.vertex_scratch.reserve(validated.positions.len() * 48);
            for index in 0..validated.positions.len() {
                for component in validated.positions[index] {
                    self.vertex_scratch
                        .extend_from_slice(&component.to_le_bytes());
                }
                for component in validated.normals[index] {
                    self.vertex_scratch
                        .extend_from_slice(&component.to_le_bytes());
                }
                for component in validated.uvs[index] {
                    self.vertex_scratch
                        .extend_from_slice(&component.to_le_bytes());
                }
                for component in validated.tangents[index] {
                    self.vertex_scratch
                        .extend_from_slice(&component.to_le_bytes());
                }
            }
            if let Some(resident) = self.meshes.get(&key) {
                // LF5: publish the positions this key held before the write,
                // so the motion target carries the mesh's own move.
                if let Some(prev) = &resident.prev_vertex {
                    super::motion::write_prev_positions_alloc(
                        &self.queue,
                        prev,
                        &resident.last_positions,
                    );
                }
                self.queue
                    .write_buffer(&resident.vertex, 0, &self.vertex_scratch);
            }
            if let Some(resident) = self.meshes.get_mut(&key) {
                resident.last_positions.clear();
                resident
                    .last_positions
                    .extend_from_slice(&validated.positions);
            }
            return Ok(());
        }
        let gpu = self.pack_mesh(mesh)?;
        let incoming = Self::mesh_index_hash(&mesh.indices);
        if let Some(evicted) = self.meshes.insert(key, gpu) {
            debug_assert_ne!(evicted, key, "reinsert never evicts itself");
            self.mesh_index_hash.remove(&evicted);
            self.skin_blocks.remove(&evicted);
        }
        // The new buffers hold the static vertex: the key leaves the skin set.
        self.skin_blocks.remove(&key);
        self.mesh_index_hash.insert(key, incoming);
        Ok(())
    }

    /// Upload a skinned body part under `key`: the bind-pose `mesh` plus its
    /// `skin` rows packed as 80-byte vertices (pos, normal, uv, tangent,
    /// four joint indices, four weights). A draw of `key` blends the joint
    /// matrices that [`Self::write_skin_block`] last wrote to `block`, so a
    /// posed body costs one 45-matrix write per tick and no vertex upload.
    /// Revalidated like [`Self::upload_mesh`]; a skin row count that differs
    /// from the vertex count, a joint index past [`Self::SKIN_BLOCK_JOINTS`]
    /// or a block past [`Self::SKIN_BLOCKS`] is refused and changes nothing.
    /// A re-upload replaces the buffers. Cold path: call it once per part,
    /// not per frame.
    pub fn upload_skinned_mesh(
        &mut self,
        key: MeshKey,
        mesh: &v1::MeshData,
        skin: &crate::v1_skin::SkinWeights,
        block: usize,
    ) -> Result<(), PresentError> {
        use wgpu::util::DeviceExt;
        let block_id = u8::try_from(block)
            .ok()
            .filter(|_| block < SKIN_BLOCKS)
            .ok_or_else(|| {
                PresentError::InvalidMesh(format!(
                    "skinned mesh {}: block {block} past the {SKIN_BLOCKS} blocks",
                    key.id
                ))
            })?;
        let mesh = v1::upload_mesh(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices)
            .map_err(|e| PresentError::InvalidMesh(format!("{e}")))?;
        if skin.joints.len() != mesh.positions.len() || skin.weights.len() != mesh.positions.len() {
            return Err(PresentError::InvalidMesh(format!(
                "skinned mesh {}: {} vertices but {} joint rows and {} weight rows",
                key.id,
                mesh.positions.len(),
                skin.joints.len(),
                skin.weights.len()
            )));
        }
        let stride = usize::try_from(super::submit::SKINNED_VERTEX_STRIDE).unwrap_or(80);
        let mut bytes = Vec::with_capacity(mesh.positions.len() * stride);
        for (index, row) in skin.joints.iter().enumerate() {
            if row
                .iter()
                .any(|joint| usize::from(*joint) >= SKIN_BLOCK_JOINTS)
            {
                return Err(PresentError::InvalidMesh(format!(
                    "skinned mesh {}: vertex {index} names a joint past {SKIN_BLOCK_JOINTS}",
                    key.id
                )));
            }
            let floats = mesh.positions[index]
                .iter()
                .chain(&mesh.normals[index])
                .chain(&mesh.uvs[index]);
            for value in floats {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            // EG12 slice c: the mesh tangent rides the same draw, so a
            // normal map on a skinned body maps through the real frame.
            for value in mesh.tangents[index] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for joint in row {
                bytes.extend_from_slice(&u32::from(*joint).to_le_bytes());
            }
            for weight in &skin.weights[index] {
                bytes.extend_from_slice(&weight.to_le_bytes());
            }
        }
        let vertex = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fc2b-skinned-vb"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fc2b-skinned-ib"),
                contents: &mesh.index_bytes(),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.creates.buffer();
        self.creates.buffer();
        let gpu = GpuMesh {
            vertex,
            // A skinned body takes its previous pose from the previous
            // joint palette, never a second vertex stream.
            prev_vertex: None,
            last_positions: Vec::new(),
            index,
            count: u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX),
            vertices: u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX),
            bounds: crate::draw::bound_sphere(&mesh.positions),
            indices: Vec::new(),
        };
        if let Some(evicted) = self.meshes.insert(key, gpu) {
            debug_assert_ne!(evicted, key, "reinsert never evicts itself");
            self.mesh_index_hash.remove(&evicted);
            self.skin_blocks.remove(&evicted);
        }
        // No index fingerprint: a skinned key never takes the in-place path.
        self.mesh_index_hash.remove(&key);
        self.skin_blocks.insert(key, block_id);
        // A new body in this block: its first palette write reports zero
        // motion instead of the previous occupant's pose.
        self.skin_prev_seen[block] = false;
        Ok(())
    }

    /// Joints per skin block: the palette length `vs_skinned` indexes.
    pub const SKIN_BLOCK_JOINTS: usize = SKIN_BLOCK_JOINTS;
    /// Skin blocks the joint buffer holds: skinned bodies drawn per frame.
    pub const SKIN_BLOCKS: usize = SKIN_BLOCKS;

    /// Write one skinned body's joint matrices into `block`: AN1 row-major
    /// matrices (`inverse bind * joint * root`, translation in the last row),
    /// uploaded verbatim as the shader's columns, exactly as
    /// `SkinningUniform::to_bytes` lays them out. One `queue.write_buffer`
    /// of `matrices.len() * 64` bytes; no buffer or bind group is created.
    /// Every skinned draw of a mesh uploaded with this `block` reads it, and
    /// the next submit sees the latest write. A block past
    /// [`Self::SKIN_BLOCKS`], more than [`Self::SKIN_BLOCK_JOINTS`] matrices
    /// or a non-finite component is refused loudly and writes nothing.
    pub fn write_skin_block(
        &mut self,
        block: usize,
        matrices: &[[[f32; 4]; 4]],
    ) -> Result<(), PresentError> {
        if block >= SKIN_BLOCKS || matrices.len() > SKIN_BLOCK_JOINTS {
            return Err(PresentError::InvalidMesh(format!(
                "skin block {block} with {} matrices: the buffer holds {SKIN_BLOCKS} blocks of {SKIN_BLOCK_JOINTS}",
                matrices.len()
            )));
        }
        let base = block * SKIN_BLOCK_JOINTS;
        let mut bytes = [0u8; SKIN_BLOCK_JOINTS * 64];
        let mut at = 0usize;
        for (joint, matrix) in matrices.iter().enumerate() {
            for value in matrix.iter().flatten() {
                if !value.is_finite() {
                    return Err(PresentError::InvalidMesh(format!(
                        "skin block {block}: joint {joint} matrix is not finite"
                    )));
                }
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
                at += 4;
            }
        }
        // LF5: the palette this block took last time, or today's matrix on
        // its first write: a fresh body reports zero motion, never a jump
        // from the bind pose.
        let prev_bytes = super::motion::prev_skin_bytes(
            base,
            matrices,
            &mut self.skin_prev,
            self.skin_prev_seen[block],
        );
        if at > 0 {
            let offset = u64::try_from(block).unwrap_or(0) * SKIN_BLOCK_BYTES;
            self.queue
                .write_buffer(&self.prev_skin_joint_buf, offset, &prev_bytes[..at]);
            self.queue
                .write_buffer(&self.skin_joint_buf, offset, &bytes[..at]);
            self.skin_prev_seen[block] = true;
        }
        Ok(())
    }

    /// Skin block the resident mesh `key` draws through, `None` for a static
    /// mesh or an absent key.
    #[must_use]
    pub fn skin_block_of(&self, key: &MeshKey) -> Option<usize> {
        self.skin_blocks.get(key).map(|block| usize::from(*block))
    }

    /// Pack a mesh into GPU buffers without caching it: `upload_mesh`
    /// inserts the result, and the arena crowd keeps its own copy.
    pub(super) fn pack_mesh(&self, mesh: &v1::MeshData) -> Result<GpuMesh, PresentError> {
        use wgpu::util::DeviceExt;
        // Revalidate: `MeshData` fields are public, so only the validator
        // decides what reaches the GPU. Uploads are rare (cached), so the
        // re-pack cost is off the frame path.
        let mesh = v1::upload_mesh_tiled(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices)
            .map_err(|e| PresentError::InvalidMesh(format!("{e}")))?;
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-mesh-vb"),
                contents: &mesh.vertex_bytes(),
                // Test builds add `COPY_SRC` so a test reads the bytes back.
                usage: if cfg!(test) {
                    wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC
                } else {
                    wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST
                },
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ra1-mesh-ib"),
                contents: &mesh.index_bytes(),
                usage: wgpu::BufferUsages::INDEX,
            });
        // LF5: the same positions as a compact stream the motion target
        // subtracts; a refresh rewrites it before taking the new pose.
        let prev_vb = super::motion::create_prev_buffer(&self.device, &mesh.positions);
        self.creates.buffer();
        self.creates.buffer();
        self.creates.buffer();
        let count = u32::try_from(mesh.indices.len()).unwrap_or(u32::MAX);
        let vertices = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
        Ok(GpuMesh {
            vertex: vb,
            prev_vertex: Some(prev_vb),
            last_positions: mesh.positions.clone(),
            index: ib,
            count,
            vertices,
            bounds: crate::draw::bound_sphere(&mesh.positions),
            indices: mesh.indices,
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
        let capped = upload.capped_to(u32::from(self.quality.texture_max_px));
        let upload = capped.as_ref().unwrap_or(upload);
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

    /// Upload a linear (non-sRGB) data map with its box-filter mip chain.
    /// Cold path only; the normal and ORM maps of a material set use it.
    fn pack_linear_texture(
        &self,
        upload: &TextureUpload,
        label: &'static str,
    ) -> Result<GpuTexture, PresentError> {
        upload.validate()?;
        let capped = upload.capped_to(u32::from(self.quality.texture_max_px));
        let upload = capped.as_ref().unwrap_or(upload);
        let size = wgpu::Extent3d {
            width: upload.width,
            height: upload.height,
            depth_or_array_layers: 1,
        };
        let mips = upload.mip_chain();
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: u32::try_from(mips.len() + 1).unwrap_or(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let base = (upload.width, upload.height, upload.rgba.as_slice());
        let levels =
            std::iter::once(base).chain(mips.iter().map(|(w, h, d)| (*w, *h, d.as_slice())));
        for (level, (width, height, rgba)) in (0u32..).zip(levels) {
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
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok(GpuTexture {
            texture,
            view,
            width: upload.width,
            height: upload.height,
        })
    }

    /// Register a material set (VR2): `normal` (linear tangent-space, GL
    /// convention) and an optional `orm` map (R occlusion, G roughness, B
    /// metallic, linear), bound over the already-resident base-color texture
    /// `base` (`None` binds the white fallback). A draw selects the set by
    /// naming `key` as its `normal_map`; the set's bind group carries its own
    /// base, so a re-upload of `base` needs the set registered again. The
    /// ORM factors multiply the draw's material factors; no ORM keeps them as
    /// they are. Cold path only: one bind group, never created inside a frame.
    /// A `base` that is not resident fails with `MissingTexture`.
    pub fn upload_material_set(
        &mut self,
        key: TextureKey,
        base: Option<TextureKey>,
        normal: &TextureUpload,
        orm: Option<&TextureUpload>,
    ) -> Result<(), PresentError> {
        let normal_gpu = self.pack_linear_texture(normal, "ra1-set-normal")?;
        let orm_gpu = match orm {
            Some(upload) => self.pack_linear_texture(upload, "ra1-set-orm")?,
            None => Self::linear_texel(
                &self.device,
                &self.queue,
                "ra1-set-orm",
                orm_fallback_rgba(),
            ),
        };
        let white;
        let base_view = match &base {
            Some(base_key) => self.require_texture_view(base_key)?.0,
            None => {
                white = Self::fallback_white(&self.device, &self.queue);
                &white.view
            }
        };
        let bind = self.forward_bind_maps(
            base_view,
            &normal_gpu.view,
            &orm_gpu.view,
            &self.repeat_sampler,
        );
        self.creates.bind_group();
        self.map_sets.retain(|set| set.key != key);
        self.map_sets.push(MapSet {
            key,
            _textures: [normal_gpu, orm_gpu],
            bind,
        });
        Ok(())
    }

    /// Drop one material set; true when it was registered.
    pub fn evict_material_set(&mut self, key: &TextureKey) -> bool {
        let before = self.map_sets.len();
        self.map_sets.retain(|set| set.key != *key);
        self.map_sets.len() != before
    }

    /// Keys of the registered material sets, in registration order.
    pub fn material_set_keys(&self) -> Vec<TextureKey> {
        self.map_sets.iter().map(|set| set.key).collect()
    }

    /// Forward bind group over `view` and the shared frame buffers plus
    /// the live baked lighting in the forward group, with the LF1 sky-view
    /// texture at 15 for the sky pass (opaque `fs` ignores it). The neutral
    /// normal fallback rides at 17 until per-draw normal maps land.
    pub(crate) fn forward_bind(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.forward_bind_maps(
            view,
            &self.normal_fallback.view,
            &self.orm_fallback.view,
            &self.base_sampler,
        )
    }

    /// [`Self::forward_bind`] with an explicit normal map and ORM map (VR2
    /// material sets): the same group, only the two map views differ.
    pub(crate) fn forward_bind_maps(
        &self,
        view: &wgpu::TextureView,
        normal_view: &wgpu::TextureView,
        orm_view: &wgpu::TextureView,
        map_sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
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
            &ForwardBindSources {
                layout: &self.forward_layout,
                cam: &self.cam_buf,
                light: &self.light_buf,
                material: &self.material_ring,
                material_bytes: super::submit::MATERIAL_BYTES,
                shadow: &self.twin_shadow_buf,
                sampler: &self.base_sampler,
                map_sampler,
                fog: &self.fog_fwd_buf,
                torch: &self.torch_buf,
                pre_view: &pre_view,
                brdf_view: &brdf_view,
                sh_buf: &self.ibl_sh_buf,
                sky_view: &self.sky_view_view,
                normal_view,
                orm_view,
                skin_joints: &self.skin_joint_buf,
                skin_params: &self.skin_params_buf,
                prev_cam: &self.prev_cam_buf,
                prev_skin_joints: &self.prev_skin_joint_buf,
                steady_cam: &self.jitter.steady_buf,
                sun_view: self.sun_sub.view(),
                sun_smp: self.sun_sub.sampler(),
                sun_buf: self.sun_sub.fwd_uniform(),
            },
            view,
        )
    }

    /// Rewrite the vertices of the resident mesh `key` in place: pack
    /// position, normal, uv and tangent into the mesh's own 48-byte vertex
    /// buffer with one `queue.write_buffer`, no new buffer, no revalidation
    /// of the index list. The tangents are recomputed from the new
    /// positions, the uvs and the resident triangle list by the rule
    /// [`Presenter::upload_mesh`] uses, so a refresh leaves the same bytes a
    /// fresh upload of the same mesh would. This is the per-frame path of a
    /// CPU-skinned body: create the mesh once with
    /// [`Presenter::upload_mesh`], refresh it every frame.
    /// Returns `Ok(false)` when `key` is not resident (the caller makes
    /// the one-time upload), `Ok(true)` once written. The vertex count is
    /// fixed for the buffer's life; a different count, a non-finite
    /// component, a degenerate normal or a skinned key (64-byte vertices)
    /// is refused loudly and writes nothing.
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
        if self.skin_blocks.contains_key(&key) {
            return Err(PresentError::InvalidMesh(format!(
                "refresh mesh {}: a skinned mesh holds 64-byte vertices, upload it again instead",
                key.id
            )));
        }
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
        }
        // CODELAW one-owner: tangents come from the import lane, as in
        // `v1::upload_mesh`; every input is validated above.
        // LF5: publish the pose this key held before the refresh.
        if let Some(prev) = &resident.prev_vertex {
            super::motion::write_prev_positions(
                &self.queue,
                &mut self.vertex_scratch,
                prev,
                &resident.last_positions,
            );
        }
        let proxy = engine_assets::CookedMesh {
            positions: positions.to_vec(),
            normals: normals.to_vec(),
            uv0: uvs.to_vec(),
            tangents: Vec::new(),
            indices: resident.indices.clone(),
        };
        let tangents = engine_assets::import_repair::recompute_tangents(&proxy);
        self.vertex_scratch.clear();
        self.vertex_scratch.reserve(expected * 48);
        let rows = positions.iter().zip(normals).zip(uvs).zip(&tangents);
        for (((p, n), uv), t) in rows {
            for c in p.iter().chain(n.iter()).chain(uv.iter()).chain(t.iter()) {
                self.vertex_scratch.extend_from_slice(&c.to_le_bytes());
            }
        }
        debug_assert_eq!(self.vertex_scratch.len(), expected * 48);
        let Some(resident) = self.meshes.get(&key) else {
            return Ok(false);
        };
        self.queue
            .write_buffer(&resident.vertex, 0, &self.vertex_scratch);
        // The new pose becomes the next frame's previous one.
        if let Some(resident) = self.meshes.get_mut(&key) {
            resident.last_positions.clear();
            resident.last_positions.extend_from_slice(positions);
            // The world bound follows the positions it describes.
            resident.bounds = crate::draw::bound_sphere(positions);
        }
        Ok(true)
    }

    /// Buffers and bind groups created since the presenter was built
    /// (cumulative): a steady frame leaves both unchanged.
    pub fn gpu_create_totals(&self) -> (u64, u64) {
        self.creates.totals()
    }

    /// Draws the last staged frame submitted after the EG38 sphere cull: the
    /// forward pass drew this many of the list's items. A list whose only
    /// draw sits outside the frame frustum reads zero.
    pub fn submitted_draws(&self) -> usize {
        self.submitted_draws
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

    /// LF14: replace the weapon-trail strip the coming frame draws. `verts`
    /// is the flat list of per-slot strips and `lens` the vert count of each
    /// slot's strip, so one draw never bridges two fighters. An empty list
    /// clears it, so an idle frame draws no ribbon. View only, never hashed.
    /// Both lists are copied into reuse buffers: no per-frame allocation
    /// once the shape settles.
    pub fn set_trail_strips(&mut self, verts: &[crate::draw::TrailStripVert], lens: &[u32]) {
        self.trail_verts.clear();
        self.trail_verts.extend_from_slice(verts);
        self.trail_lens.clear();
        self.trail_lens.extend_from_slice(lens);
    }

    /// Verts in the trail strip staged for the coming frame (receipts and
    /// tests read it; 0 means no ribbon).
    pub fn trail_vert_count(&self) -> usize {
        self.trail_verts.len()
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
        if !self.look_probes && on {
            // Entering the probe composite: the pair may hold live-path
            // uploads, so the fog-off branch resets it once below.
            self.fog_pair_dirty = true;
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
    /// today's 1x path; on builds the 4x targets plus resolve only while the
    /// adapter can build the whole pair, so a weak adapter reads back off
    /// instead of failing texture creation. Copyable state, no per-frame
    /// allocation, never waits on the GPU.
    pub fn set_msaa_enabled(&mut self, on: bool) {
        // The adapter owns the count: `msaa_effective_sample_count` reads the
        // requested switch plus the support read at build, so an unbuildable
        // pair never reaches `ensure_targets`.
        let want = MsaaConfig { enabled: on };
        let on = msaa_effective_sample_count(&want, self.msaa_support) == MSAA_SAMPLE_COUNT;
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

    /// Drop every timing sample gathered so far, so the next report covers
    /// only the frames rendered after this call (A/B cost runs).
    pub fn reset_gpu_timing(&mut self) {
        if let Some(timer) = self.gpu_timer.as_mut() {
            timer.clear_samples();
        }
    }

    /// Block until the device has finished every submitted frame, so the
    /// late timestamp reads land. For cost measurement only: a window run
    /// never calls it, and it counts as no frame.
    pub fn settle_gpu_timing(&self) -> Result<(), PresentError> {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_status| ())
            .map_err(|e| PresentError::Readback(format!("settle poll: {e}")))
    }

    /// Mean wall time of the swapchain acquire in us over acquired frames:
    /// how long the CPU parked waiting for the display (vsync-bound
    /// present modes park here).
    pub fn acquire_mean_us(&self) -> u64 {
        self.acquire_us / self.acquire_frames.max(1)
    }

    /// Wall time of the last swapchain acquire in us: the wait of the most
    /// recent frame alone, so a perf run can name the frame that blocked.
    pub fn last_acquire_us(&self) -> u64 {
        self.last_acquire_us
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

    /// Stage one text line as glyph quads in 0..1 space, top-left origin;
    /// false when the ring takes no glyph. Window leg only; no GPU work.
    pub fn push_text(&mut self, x: f32, y: f32, height: f32, col: [u8; 4], line: &str) -> bool {
        stage_text_line(&mut self.overlay, x, y, height, col, line)
    }

    /// Replace this frame's debug line list with `batch`; an empty batch
    /// clears it, so the caller pushes once per frame after the tick (the
    /// one call `debug_draw.rs` names as the live sink). The batch is
    /// copied into buffers the next push reuses; the after-blit line pass
    /// draws it while [`Presenter::debug_line_enable`] is on.
    pub fn push_debug_lines(&mut self, batch: &crate::debug_draw::LineBatch) {
        super::debug_lines::stage_debug_lines(&mut self.debug_lines, batch);
    }

    /// This frame's debug line list in group order: the source the live
    /// line pass draws. Window leg only.
    pub fn debug_lines(&self) -> &crate::debug_draw::LineBatch {
        &self.debug_lines
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
        super::live::set_live_bloom_enabled(on);
    }

    /// LF4 bloom switch state.
    pub fn bloom_enabled(&self) -> bool {
        self.bloom.enabled
    }

    /// The auto-exposure meter switch. On, the live frame histograms the
    /// HDR target and adapts the camera exposure toward the key; off,
    /// every frame is exactly what it was. Presentation only, never
    /// hashed.
    pub fn set_auto_exposure_enabled(&mut self, on: bool) {
        self.auto_exp_enabled = on;
    }

    /// True when the meter runs.
    pub fn auto_exposure_enabled(&self) -> bool {
        self.auto_exp_enabled
    }

    /// The adapted stops the meter holds.
    pub fn auto_exposure_ev(&self) -> f32 {
        self.auto_exp_state.ev()
    }

    /// The histogram the meter read last, `None` until the first
    /// readback lands.
    pub fn auto_exposure_histogram(&self) -> Option<&crate::post::ExposureHistogram> {
        self.auto_exp_hist.as_ref()
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
        super::super::fog_params_for_look_bake_at(&self.sky_look, self.sky_exposure_ev100)
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
    fn msaa_edge_live_resolve_splits() {
        use super::super::msaa::resolve_average;
        use super::super::msaa::{edge_has_intermediate, msaa_forward_choice, MsaaConfig};
        let off = MsaaConfig::default();
        let on = MsaaConfig { enabled: true };
        // Live branch: on plus built resolves 4x into 1x HDR, else 1x.
        let (_, on_built_resolves) = msaa_forward_choice(&on, true);
        let (_, on_unbuilt_resolves) = msaa_forward_choice(&on, false);
        let (_, off_built_resolves) = msaa_forward_choice(&off, true);
        let (_, off_unbuilt_resolves) = msaa_forward_choice(&off, false);
        assert!(on_built_resolves, "on plus built resolves before blit");
        assert!(!on_unbuilt_resolves, "on plus unbuilt stays 1x");
        assert!(!off_built_resolves, "off plus built stays 1x");
        assert!(!off_unbuilt_resolves, "off plus unbuilt stays 1x");
        // Half-covered edge quad resolves to a half on the on path only.
        let mixed = [0.0f32, 0.0, 1.0, 1.0];
        let flat_one = [1.0f32; 4];
        let on_tex = if on_built_resolves {
            [resolve_average(mixed)]
        } else {
            [resolve_average(flat_one)]
        };
        let off_tex = [resolve_average(flat_one)];
        assert!(
            edge_has_intermediate(&on_tex, &off_tex),
            "on set blends, off set stays stepped through live branch"
        );
        assert!(
            !edge_has_intermediate(&off_tex, &off_tex),
            "flat off-set stays stepped"
        );
        // Same split twice: same inputs give same verdict.
        assert_eq!(
            edge_has_intermediate(&on_tex, &off_tex),
            edge_has_intermediate(&on_tex, &off_tex),
            "same inputs give same split twice"
        );
        assert_eq!(
            msaa_forward_choice(&on, true),
            msaa_forward_choice(&on, true),
            "same inputs give same choice twice"
        );
    }

    #[test]
    fn msaa_toggle_drops_targets() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(crate::present::PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(e) => panic!("presenter init failed: {e}"),
        };
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("msaa_toggle_drops_targets", probe)
        else {
            return;
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
        let picked = crate::present::Presenter::pick_present_mode(
            &[wgpu::PresentMode::Fifo, wgpu::PresentMode::Mailbox],
            true,
        );
        assert_eq!(picked, wgpu::PresentMode::Mailbox);
    }

    #[test]
    fn present_mode_prefers_immediate_over_mailbox() {
        // Caller: `Presenter::attach_surface` window present path.
        // Behavior: Immediate (tearing, no vsync wait) is the shortest
        // acquire wait, so it wins when the driver offers it; pixels and
        // draws are unchanged, only the wait is skipped.
        let picked = crate::present::Presenter::pick_present_mode(
            &[
                wgpu::PresentMode::Fifo,
                wgpu::PresentMode::Mailbox,
                wgpu::PresentMode::Immediate,
            ],
            true,
        );
        assert_eq!(picked, wgpu::PresentMode::Immediate);
    }

    #[test]
    fn present_mode_keeps_old_path_when_not_ready() {
        let picked = crate::present::Presenter::pick_present_mode(&[wgpu::PresentMode::Fifo], true);
        assert_eq!(picked, wgpu::PresentMode::Fifo);
        let picked = crate::present::Presenter::pick_present_mode(&[], true);
        assert_eq!(picked, wgpu::PresentMode::Fifo);
    }

    #[test]
    fn present_mode_defaults_to_fifo_when_not_uncapped() {
        // OPT35: the window is paced by vsync (Fifo) by default; `--uncapped`
        // picks Immediate or Mailbox for captures.
        let picked = crate::present::Presenter::pick_present_mode(
            &[
                wgpu::PresentMode::Fifo,
                wgpu::PresentMode::Mailbox,
                wgpu::PresentMode::Immediate,
            ],
            false,
        );
        assert_eq!(picked, wgpu::PresentMode::Fifo);
    }
}

#[cfg(test)]
mod forward_normal_tests {
    use super::{forward_normal_layout_entries, normal_fallback_rgba};

    #[test]
    fn normal_layout_covers_shader_bindings() {
        assert!(
            crate::v1_gpu::FORWARD_PBR_WGSL.contains("@group(0) @binding(17) var normal_map"),
            "forward shader declares normal_map at 17"
        );
        assert!(
            crate::v1_gpu::FORWARD_PBR_WGSL.contains("@group(0) @binding(18) var normal_smp"),
            "forward shader declares normal_smp at 18"
        );
        let entries = forward_normal_layout_entries();
        assert_eq!(
            entries[0].binding, 17,
            "layout carries normal texture at 17"
        );
        assert_eq!(
            entries[1].binding, 18,
            "layout carries normal sampler at 18"
        );
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
        let mapped = super::super::forward_bind::world_normal(geom, [1.0, 0.0, 0.0, 1.0], decoded)
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
        let mapped = super::super::forward_bind::world_normal(geom, [1.0, 0.0, 0.0, 1.0], white)
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

/// Stage one line into `ring`; true when at least one glyph quad landed.
/// Split from [`Presenter::push_text`] so the ring rule is testable without
/// a GPU.
fn stage_text_line(
    ring: &mut overlay::OverlayRing,
    x: f32,
    y: f32,
    height: f32,
    col: [u8; 4],
    line: &str,
) -> bool {
    ring.push_text(x, y, height, col, line) > 0
}

#[cfg(test)]
mod overlay_text_tests {
    use super::*;
    use crate::overlay::{Quad, OVERLAY_MAX_QUADS};

    #[test]
    fn push_text_stages_a_line_and_refuses_a_full_ring() {
        let mut ring = overlay::OverlayRing::new();
        assert!(stage_text_line(
            &mut ring,
            0.05,
            0.06,
            0.05,
            [230, 235, 240, 255],
            "hash 0"
        ));
        assert_eq!(ring.quad_count(), 5, "spaces advance without a quad");
        let first = ring.quads()[0];
        assert_eq!(first.rect[0], 0.05);
        assert_eq!(first.rect[1], 0.06);
        assert!((first.rect[2] - 0.0875).abs() < 1e-6);
        assert!((first.rect[3] - 0.11).abs() < 1e-6);
        assert_eq!(first.col, [230, 235, 240, 255]);
        assert_eq!(
            first.uv,
            crate::overlay::text::glyph_uv(b'h').expect("h is printable")
        );

        ring.clear();
        let full = Quad::default();
        for _ in 0..OVERLAY_MAX_QUADS {
            assert!(ring.push_quad(full));
        }
        assert!(!stage_text_line(&mut ring, 0.05, 0.06, 0.05, [255; 4], "x"));
        assert_eq!(ring.quad_count(), OVERLAY_MAX_QUADS);
    }
}

#[cfg(test)]
mod mesh_upload_reuse_tests {
    #[test]
    fn same_topology_reuses_different_rebuilds() {
        let a = super::Presenter::mesh_index_hash(&[0, 1, 2, 2, 1, 3]);
        let b = super::Presenter::mesh_index_hash(&[0, 1, 2, 2, 1, 3]);
        let c = super::Presenter::mesh_index_hash(&[0, 1, 2, 2, 1, 4]);
        assert_eq!(a, b, "same indices give same fingerprint twice");
        assert_ne!(a, c, "changed index moves the fingerprint");
        assert!(
            super::Presenter::mesh_upload_should_reuse(Some(a), b),
            "same topology reuses"
        );
        assert!(
            !super::Presenter::mesh_upload_should_reuse(Some(a), c),
            "changed topology rebuilds"
        );
        assert!(
            !super::Presenter::mesh_upload_should_reuse(None, b),
            "first upload rebuilds"
        );
    }

    #[test]
    fn second_identical_upload_sizes_no_new_buffers() {
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(crate::present::PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(e) => panic!("presenter init failed: {e}"),
        };
        let Some(mut presenter) =
            crate::test_gpu::require_or_skip("second_identical_upload_sizes_no_new_buffers", probe)
        else {
            return;
        };
        let key = crate::present::MeshKey::new(0x5EED_0001, [0x5E; 32]);
        let mesh = crate::v1::cube();
        presenter
            .upload_mesh(key, &mesh)
            .expect("first upload builds");
        let (buffers, _) = presenter.gpu_create_totals();
        presenter
            .upload_mesh(key, &mesh)
            .expect("second identical reuses");
        let (buffers_after, _) = presenter.gpu_create_totals();
        assert_eq!(
            buffers_after, buffers,
            "identical re-upload sizes no new buffers"
        );
        let mut bad = mesh.clone();
        bad.positions[0] = [f32::NAN, 0.0, 0.0];
        assert!(
            presenter.upload_mesh(key, &bad).is_err(),
            "non-finite refuses"
        );
    }
}

#[cfg(test)]
mod skin_window_tests {
    use super::super::submit::{
        skin_block_offset, SKINNED_VERTEX_STRIDE, SKIN_BLOCKS, SKIN_BLOCK_BYTES, SKIN_BLOCK_JOINTS,
        SKIN_BLOCK_STRIDE,
    };
    use crate::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
    use crate::v1::{self, MeshData};
    use crate::v1_skin::{SkinPipeline, SkinWeights};

    type Mat = [[f32; 4]; 4];

    const IDENTITY: Mat = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    /// AN1 row-vector turn about +Z by `angle`, then a move by `t`.
    fn turn_z(angle: f32, t: [f32; 3]) -> Mat {
        let (s, c) = angle.sin_cos();
        [
            [c, s, 0.0, 0.0],
            [-s, c, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [t[0], t[1], t[2], 1.0],
        ]
    }

    /// The unit cube bound to two joints: weight on joint 1 grows with
    /// height and a little with x, so the top follows joint 1, the bottom
    /// stays on joint 0 and the middle band blends.
    fn two_joint_cube() -> (MeshData, SkinWeights) {
        let mesh = v1::cube();
        let mut joints = Vec::new();
        let mut weights = Vec::new();
        for p in &mesh.positions {
            let w1 = (0.5 + p[1] + 0.4 * p[0]).clamp(0.0, 1.0);
            joints.push([0u16, 1, 0, 0]);
            weights.push([1.0 - w1, w1, 0.0, 0.0]);
        }
        let mut pipe = SkinPipeline::new();
        pipe.upload_skeleton(2).expect("two joints");
        let skin = pipe.upload_weights(&joints, &weights).expect("rows valid");
        (mesh, skin)
    }

    /// The CPU skin the old window path uploaded: posed positions and
    /// normals over the bind uvs and indices.
    fn cpu_posed(mesh: &MeshData, skin: &SkinWeights, matrices: &[Mat]) -> MeshData {
        let mut pipe = SkinPipeline::new();
        pipe.upload_skeleton(matrices.len()).expect("joints");
        let uniform = pipe.skinning_uniform(matrices).expect("matrices");
        let posed = pipe
            .render_skinned(mesh, Some(skin), &uniform)
            .expect("skins");
        v1::upload_mesh(&posed.positions, &posed.normals, &mesh.uvs, &mesh.indices)
            .expect("posed mesh is valid")
    }

    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(mut presenter) => {
                presenter.resize(128, 128).expect("valid size applies");
                Ok(presenter)
            }
            Err(PresentError::NoAdapter) => Err(crate::test_gpu::NoAdapter(
                "no wgpu adapter on this machine".into(),
            )),
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        crate::test_gpu::require_or_skip(test, probe)
    }

    fn list_of(keys: &[MeshKey]) -> DrawList {
        let scene = v1::default_scene(128, 128).expect("default scene is valid");
        DrawList {
            camera: scene.camera,
            light: scene.light,
            items: keys
                .iter()
                .map(|key| DrawItem {
                    mesh: *key,
                    material: scene.material.clone(),
                    base_color: None,
                    normal_map: None,
                    orm: None,
                    emissive: None,
                })
                .collect(),
        }
    }

    fn pixels(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
        let out = presenter.render_offscreen(list).expect("frame renders");
        assert!(
            out.receipt.is_clean(),
            "no validation errors: {:?}",
            out.receipt
        );
        let png = out.png.expect("capture has pixels");
        image::load_from_memory(&png)
            .expect("capture is a PNG")
            .to_rgba8()
            .into_raw()
    }

    /// (largest channel difference, pixels differing by more than `slack`).
    fn compare(a: &[u8], b: &[u8], slack: u8) -> (u8, usize) {
        assert_eq!(a.len(), b.len(), "same frame size");
        let mut worst = 0u8;
        let mut over = 0usize;
        for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let d = pa
                .iter()
                .zip(pb)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            worst = worst.max(d);
            over += usize::from(d > slack);
        }
        (worst, over)
    }

    /// Pixels that left the background color.
    fn covered(rgba: &[u8]) -> usize {
        rgba.chunks_exact(4)
            .filter(|p| {
                let off = (f32::from(p[0]) - 7.0)
                    .abs()
                    .max((f32::from(p[1]) - 8.0).abs())
                    .max((f32::from(p[2]) - 15.0).abs());
                off > 5.0
            })
            .count()
    }

    #[test]
    fn a_presenter_on_a_caller_opened_device_draws_the_frame_init_offscreen_draws() {
        let Some(mut classic) = presenter_or_skip("from-device parity") else {
            return;
        };
        // The route the browser takes: the caller picks the adapter and opens the device with
        // the shared descriptor, then hands all three over.
        let (choice, _) = crate::backend::resolve_from_env(None);
        let pick = crate::backend::pick_backend(choice, None);
        let instance = crate::backend::new_instance(crate::backend::wgpu_backends(pick.primary));
        let (adapter, _) = pollster::block_on(crate::presenter_pick::pick_adapter(
            &instance,
            crate::backend::wgpu_backends(pick.primary),
            &crate::adapter::SelectionRequest::from_env(),
        ))
        .expect("the adapter init_offscreen found is found again");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Presenter::device_descriptor(&adapter)))
                .expect("the shared descriptor opens a device");
        let mut parts = Presenter::init_offscreen_from_device(&adapter, device, queue)
            .expect("a caller-opened device builds a presenter");
        parts.resize(128, 128).expect("valid size applies");
        assert_eq!(parts.adapter_name(), classic.adapter_name());
        assert_eq!(
            parts.backend_selection().source,
            crate::adapter::SelectionSource::Caller,
            "a caller-opened device reports that the caller picked it"
        );
        assert_eq!(
            parts.backend_selection().selected,
            classic.backend_selection().selected,
            "both routes name the same adapter"
        );
        let key = MeshKey::new(0x5C00_0100, [0x44; 32]);
        for presenter in [&mut classic, &mut parts] {
            presenter
                .upload_mesh(key, &v1::cube())
                .expect("cube uploads");
        }
        let list = list_of(&[key]);
        let toggles = crate::pass_truth::LookToggles::all_on();
        let a = classic
            .capture_live_pixels(&list, &toggles)
            .expect("classic live frame");
        let b = parts
            .capture_live_pixels(&list, &toggles)
            .expect("caller-device live frame");
        assert!(a.receipt.is_clean() && b.receipt.is_clean());
        assert!(
            a.rgba.chunks_exact(4).any(|px| px[0] > 40),
            "the cube is lit"
        );
        assert_eq!(
            a.rgba, b.rgba,
            "both routes draw the same live frame, byte for byte"
        );
        assert_eq!(
            pixels(&mut classic, &list),
            pixels(&mut parts, &list),
            "and the same capture frame"
        );
    }

    #[test]
    fn skin_block_layout_matches_the_shader_palette() {
        let wgsl = crate::v1_gpu::FORWARD_PBR_WGSL;
        assert!(
            wgsl.contains(
                "@group(0) @binding(7) var<storage, read> skin_joints: array<mat4x4<f32>, 64>"
            ),
            "the shader palette is 64 matrices at binding 7"
        );
        assert!(wgsl.contains("@location(3) joints: vec4<u32>"));
        assert!(wgsl.contains("@location(4) weights: vec4<f32>"));
        assert_eq!(SKIN_BLOCK_JOINTS, 64);
        assert_eq!(SKIN_BLOCK_BYTES, 64 * 64, "one mat4 is 64 bytes");
        assert_eq!(u64::from(SKIN_BLOCK_STRIDE), SKIN_BLOCK_BYTES);
        assert_eq!(SKINNED_VERTEX_STRIDE, 12 + 12 + 8 + 16 + 16 + 16);
        assert!(wgsl.contains("@location(5) tangent: vec4<f32>"));
        for block in 0..SKIN_BLOCKS {
            let offset = skin_block_offset(u8::try_from(block).expect("block fits u8"));
            assert_eq!(offset % 256, 0, "block {block} is a legal storage offset");
            assert_eq!(
                u64::from(offset),
                SKIN_BLOCK_BYTES * u64::try_from(block).expect("block fits u64")
            );
        }
    }

    #[test]
    fn skin_window_draw_matches_the_cpu_skinned_mesh() {
        let Some(mut presenter) = presenter_or_skip("skin window draw") else {
            return;
        };
        let (mesh, skin) = two_joint_cube();
        let matrices = [IDENTITY, turn_z(0.55, [0.25, 0.1, 0.0])];
        let cpu_key = MeshKey::new(0x5C00_0001, [0x11; 32]);
        let gpu_key = MeshKey::new(0x5C00_0002, [0x22; 32]);
        presenter
            .upload_mesh(cpu_key, &cpu_posed(&mesh, &skin, &matrices))
            .expect("cpu skinned mesh uploads");
        presenter
            .upload_skinned_mesh(gpu_key, &mesh, &skin, 0)
            .expect("bind mesh uploads");
        presenter
            .write_skin_block(0, &matrices)
            .expect("block writes");
        assert_eq!(presenter.skin_block_of(&gpu_key), Some(0));
        assert_eq!(presenter.skin_block_of(&cpu_key), None);
        let cpu = pixels(&mut presenter, &list_of(&[cpu_key]));
        let gpu = pixels(&mut presenter, &list_of(&[gpu_key]));
        assert!(covered(&cpu) > 400, "the posed cube is on screen");
        assert!(covered(&gpu) > 400, "the GPU-skinned cube is on screen");
        // The golden rule (mean error and far-off share), with the far-off
        // level this test always held: 3 levels, tighter than the default 12.
        let rule = crate::pass_truth::frame_tolerance::FrameTolerance {
            over_level: 3,
            ..crate::pass_truth::frame_tolerance::FrameTolerance::GOLDEN
        };
        let verdict = crate::pass_truth::frame_tolerance::frames_match(&cpu, &gpu, &rule)
            .expect("same-size RGBA frames compare");
        eprintln!("skin window vs cpu: {}", verdict.describe());
        assert!(
            verdict.matches(),
            "GPU skin drifts from the CPU skin: {}",
            verdict.describe()
        );
        assert!(
            verdict.worst <= 40,
            "no pixel is far off, got {}",
            verdict.worst
        );
        // The joints drive the picture: identity matrices draw the bind
        // pose, which must differ from the posed frame on many pixels.
        presenter
            .write_skin_block(0, &[IDENTITY, IDENTITY])
            .expect("bind pose writes");
        let bind = pixels(&mut presenter, &list_of(&[gpu_key]));
        let (_, moved) = compare(&bind, &gpu, 3);
        assert!(
            moved > 150,
            "the pose changes the frame, got {moved} pixels"
        );
    }

    #[test]
    fn skin_blocks_pose_two_bodies_independently_in_one_frame() {
        let Some(mut presenter) = presenter_or_skip("skin two bodies") else {
            return;
        };
        let (mesh, skin) = two_joint_cube();
        let a = MeshKey::new(0x5C00_0010, [0x31; 32]);
        let b = MeshKey::new(0x5C00_0011, [0x32; 32]);
        let plain = MeshKey::new(0x5C00_0012, [0x33; 32]);
        presenter
            .upload_skinned_mesh(a, &mesh, &skin, 0)
            .expect("a");
        presenter
            .upload_skinned_mesh(b, &mesh, &skin, 3)
            .expect("b");
        presenter.upload_mesh(plain, &v1::cube()).expect("plain");
        let pose_a = [IDENTITY, turn_z(0.5, [0.2, 0.0, 0.0])];
        let pose_b = [IDENTITY, turn_z(-0.5, [-0.2, 0.0, 0.0])];
        presenter.write_skin_block(0, &pose_a).expect("block 0");
        presenter.write_skin_block(3, &pose_b).expect("block 3");
        // Mixed order: static and skinned groups interleave in the list.
        let list = list_of(&[a, plain, b]);
        let frame = pixels(&mut presenter, &list);
        // Each body alone reads its own block: the two-body frame holds
        // at least the pixels either body covers.
        let only_a = pixels(&mut presenter, &list_of(&[a]));
        let only_b = pixels(&mut presenter, &list_of(&[b]));
        assert!(covered(&only_a) > 300 && covered(&only_b) > 300);
        let (_, apart) = compare(&only_a, &only_b, 3);
        assert!(
            apart > 100,
            "two blocks pose two shapes, got {apart} pixels"
        );
        assert!(covered(&frame) >= covered(&only_a).max(covered(&only_b)));
        // The live window frame takes the same draws without a validation error.
        let receipt = presenter
            .render_live_offscreen(&list)
            .expect("live frame renders");
        assert!(receipt.is_clean(), "{receipt:?}");
        assert_eq!(receipt.draws, 3);
        // And so does the 4x path (the plain 1x frame stands in when the
        // adapter cannot build the multisample pair).
        presenter.set_msaa_enabled(true);
        let receipt = presenter
            .render_live_offscreen(&list)
            .expect("4x live frame renders");
        assert!(receipt.is_clean(), "{receipt:?}");
    }

    #[test]
    fn skin_uploads_and_blocks_refuse_bad_input() {
        let Some(mut presenter) = presenter_or_skip("skin refusals") else {
            return;
        };
        let (mesh, skin) = two_joint_cube();
        let key = MeshKey::new(0x5C00_0020, [0x41; 32]);
        let before = presenter.mesh_count();
        // A block past the buffer.
        assert!(matches!(
            presenter.upload_skinned_mesh(key, &mesh, &skin, SKIN_BLOCKS),
            Err(PresentError::InvalidMesh(_))
        ));
        // A skin row count that differs from the vertex count.
        let mut short = skin.clone();
        short.joints.pop();
        short.weights.pop();
        assert!(matches!(
            presenter.upload_skinned_mesh(key, &mesh, &short, 0),
            Err(PresentError::InvalidMesh(_))
        ));
        // A joint index past the palette.
        let mut wild = skin.clone();
        wild.joints[0][0] = u16::try_from(SKIN_BLOCK_JOINTS).expect("fits");
        assert!(matches!(
            presenter.upload_skinned_mesh(key, &mesh, &wild, 0),
            Err(PresentError::InvalidMesh(_))
        ));
        assert_eq!(presenter.mesh_count(), before, "a refusal changes nothing");
        assert_eq!(presenter.skin_block_of(&key), None);
        // Block writes: past the buffer, too many matrices, a NaN component.
        assert!(presenter
            .write_skin_block(SKIN_BLOCKS, &[IDENTITY])
            .is_err());
        let too_many = vec![IDENTITY; SKIN_BLOCK_JOINTS + 1];
        assert!(presenter.write_skin_block(0, &too_many).is_err());
        let mut nan = IDENTITY;
        nan[3][1] = f32::NAN;
        assert!(presenter.write_skin_block(0, &[IDENTITY, nan]).is_err());
        presenter
            .write_skin_block(SKIN_BLOCKS - 1, &vec![IDENTITY; SKIN_BLOCK_JOINTS])
            .expect("the last block takes a full palette");
        presenter
            .write_skin_block(0, &[])
            .expect("empty write is a no-op");
    }

    #[test]
    fn skin_key_reuploaded_static_leaves_the_skin_path() {
        let Some(mut presenter) = presenter_or_skip("skin key replaced") else {
            return;
        };
        let (mesh, skin) = two_joint_cube();
        let key = MeshKey::new(0x5C00_0030, [0x51; 32]);
        presenter
            .upload_skinned_mesh(key, &mesh, &skin, 1)
            .expect("skinned");
        assert_eq!(presenter.skin_block_of(&key), Some(1));
        // The same key as a 48-byte static mesh: the 80-byte buffer is
        // replaced, never rewritten in place, and the draw uses `vs`.
        presenter.upload_mesh(key, &v1::cube()).expect("static");
        assert_eq!(presenter.skin_block_of(&key), None);
        let frame = pixels(&mut presenter, &list_of(&[key]));
        assert!(covered(&frame) > 400, "the replaced cube draws");
        // And back: a skinned re-upload takes the key over again.
        presenter
            .upload_skinned_mesh(key, &mesh, &skin, 2)
            .expect("skinned again");
        assert_eq!(presenter.skin_block_of(&key), Some(2));
    }

    #[test]
    fn presenter_text_line_reaches_the_capture() {
        let Some(mut presenter) = presenter_or_skip("presenter text line") else {
            return;
        };
        let list = list_of(&[]);
        presenter.clear_overlay();
        let bare = pixels(&mut presenter, &list);
        presenter.clear_overlay();
        assert!(presenter.push_text(0.1, 0.1, 0.2, [255, 255, 255, 255], "III"));
        let text = pixels(&mut presenter, &list);
        let (worst, over) = compare(&bare, &text, 3);
        assert!(over > 0, "text pixels differ, worst {worst}");
    }
}
