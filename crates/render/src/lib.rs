//! render: NullRenderer (headless proof). 16x16 tiles, MAX lights/tile.
//!
//! R1 adds the V1 forward-PBR receipt in `v1` (mesh upload, depth, glam
//! camera, one directional light, metallic-roughness PBR, ACES, PNG capture
//! with a JSON manifest over a procedural cube, behind the `v1` feature so
//! the headless default stays std-only) and its wgpu offscreen port
//! in `v1_gpu` (behind the `window` feature, which implies `v1`). The procedural receipt alone is
//! not a V1 exit: MV1 needs the A1/A2 imported fixtures plus diagnostic views.

// The one counting allocator of this test binary: zero-allocation tests read
// it through `engine_core::alloc_probe::measure`.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

pub mod types;

pub mod window;

/// Fixed 2.5D ground projection: locked 45 degree yaw, `atan(0.5)` pitch and
/// 64 x 32 pixel diamonds. Std-only; consumers are the windowed proof and the
/// 2.5D game's ground pass.
pub mod kam2_iso;

/// Sprite layer over the fixed 2.5D view: eight facings, painter depth
/// order, independent turret aim and the team-mask tint. Std-only.
pub mod kam2_sprites;

/// Procedural stand-in sprite atlas for the KAM1b proof: four rows of eight
/// facing cells plus the team mask. Std-only.
pub mod kam2_sprite_atlas;

/// Helpers shared by the windowed KAM2 proofs; native only, like them.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
mod kam2_proof_util;

/// Windowed proof of the 2.5D ground field: real window, PNG receipt.
/// Native only: it writes the PNG to disk and blocks on the GPU adapter.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
pub mod kam2_iso_proof;

/// Windowed proof of the sprite layer: field, walking sprites, tracking
/// turret and team tint, with a PNG receipt. Native only, as above.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
pub mod kam2_sprite_proof;

/// The one PNG path a windowed proof writes (KAM1a/b/c): the repository's
/// `target/` folder under the crate's manifest dir. Native only, like the
/// proofs that call it.
#[cfg(not(target_arch = "wasm32"))]
pub mod capture_path;

pub mod graph;

/// Named history textures across frames with a validity flag (RL62): take,
/// export, inert; a resize, dyn-res step or camera cut clears the flag.
/// Std-only; consumer is the TAA resolve through `post::TaaParams`.
pub mod temporal;

/// The arena's one armed LOD policy: a pixel-error table projected from the
/// shipped distance bands, the live camera's field of view and frame height
/// in the pick, and `E2040_LOD=bands` to switch back for comparison. Std-only.
pub mod arena_lod;

/// The planar ground-state map under the pit: blood and scuff stamps keep
/// their mass through a 128-slot live pool and settle into a 1024 x 1024
/// four-byte-per-texel map that outlives the pool. Std-only; consumer is the
/// arena's gore and footfall stamps, and the forward floor pass uploads
/// `GroundState::texels`.
pub mod ground_state;
pub mod footprint;

/// The cooked height-grid terrain: integer millimetres in, one named mesh in
/// meters out, posted as one draw-list node. Std-only.
pub mod terrain;

pub mod quality;

/// Which shading family one draw takes (stone, skin, metal, cloth), stored as
/// the material uniform's kind word and the cook's kind byte. Stone is word 0
/// and every earlier frame's path; the shader branch lives in
/// `shaders/forward.wgsl` `direct_light`. Consumers: `v1::MaterialUniform`,
/// `Presenter::set_material_kind`, the game's per-part table.
pub mod material_kind;

/// RL51 sand and walls without a visible repeat: the CPU twin of the forward
/// shader's hex-tiling grid, its PCG2D hash and its macro value noise, plus
/// the `SurfaceDetail` word a stone mesh carries in its material uniform.
/// Consumers: `Presenter::set_surface_detail`, the game's kit upload.
pub mod hextile;

/// LF8: the CPU twin of `forward.wgsl` `direct_light`'s per-kind branches,
/// called by `v1::shade_pbr`. It imports `crate::v1`, so it rides the same
/// feature: the headless default stays std-only.
#[cfg(feature = "v1")]
mod v1_kind;

/// WEB13: a ledger of every pipeline build, so a build after the loader is a loud `late_compile`.
pub mod pipeline_warmup;

pub mod display_grade;
/// The crate's one display transform, as the CPU twin of
/// `shaders/tonemap.wgsl`: the per-channel fit, the hue-keeping tone map
/// and the fixed dither. The WGSL chunk is the source; this module is its
/// Rust mirror, so the CPU oracle and the shader agree on the numbers.
pub mod tonemap;

/// SW150: the locked target's screen-space x-ray outline, the CPU twin of
/// `shaders/outline_mask.wgsl` plus `outline_edge.wgsl`: half-res mask over a
/// depth prefill, Sobel edge in the visible or the hidden (behind-berm) colour,
/// two separable blurs and one additive composite. Presentation only.
pub mod outline;

/// V1 forward-PBR receipt (R1): CPU rasterizer, behind the `v1` feature so
/// the headless default stays std-only.
#[cfg(feature = "v1")]
pub mod v1;

/// Shader variant manifest with count gate + frame-exact staged reload
/// (RA7): enumerates static defines, refuses over-cap variants loudly.
/// Std-only; feeds `cache::PipelineKey` defines.
pub mod variants;

/// V1 IBL + shadow map (R2): HDRI environment sampling with a roughness
/// prefilter stub plus a single depth-map shadow path. Behind `v1` like the
/// rest of R1; consumers are R4 (cascades/probes) and GM1.
#[cfg(feature = "v1")]
pub mod v1_ibl_shadow;

/// V1 GPU skinning (R3): AN1-layout joint matrices plus A3-style weight
/// validation, skinned frames, joint/normal diagnostics and the magenta
/// missing-skin error path. Behind `v1`; consumer is GM1 fighters.
#[cfg(feature = "v1")]
pub mod v1_skin;

/// V2 lighting (R4): cascaded shadows over the R2 ortho path, baked probe
/// grids with trilinear lookup, CPU-side bloom factors, Halton TAA jitter
/// and tonemap-path grading. Behind `v1`; consumers are R6/PA1/GM1/Q1.
#[cfg(feature = "v1")]
pub mod v1_lighting;

/// Temporal upscaling parity oracle (test-only): sub-1080p render
/// targets, R4-jitter uniforms, exact history buffers and the bilinear
/// reference the live `post::taa_upsample` matches. Test-only so the
/// window build cannot reach it; consumers are the `post` parity test
/// plus the in-module oracle. Behind `v1`.
#[cfg(all(test, feature = "v1"))]
pub mod v1_temporal;

/// V1 forward-PBR on wgpu (R1): offscreen capture with zero validation errors.
#[cfg(feature = "gpu")]
pub mod v1_gpu;

/// Windowed presenter over a draw list (RA1): surface/swapchain plus HDR,
/// depth and LDR-twin targets, multi-draw forward PBR reusing the `v1_gpu`
/// shader, GPU mesh/texture caches, resize/minimize/Outdated/Lost recovery
/// keeping last-good. The error, key, cache-policy, size and surface-event
/// core is std-only so it unit-tests headless; the GPU presenter itself is
/// behind `window` (which implies `v1`). Consumers: WIN1 and ED1 next.
pub mod present;

/// CPU-side HUD overlay quad ring (EG14 slice 1): staged 0..1 textured
/// quads drained to one triangle list. Std-only; consumer is the slice-2
/// present-path feed.
pub mod overlay;

/// Flat HUD command pass (UIX13): rects, borders, text quads and scissors
/// from an `engine-ui` frame, rasterized over an RGBA8 scene. Std-only;
/// consumer is the KAM1c windowed proof and the `hud` graph pass next.
pub mod hud_pass;

/// EG8 pass-truth gate (GX6 successor): a look pass counts live only when
/// its on/off frames differ above epsilon. Std-only; consumer is the EG8
/// report plus the proof gate's per-pass flags.
pub mod pass_truth;

/// Surface Outdated/Lost plus device-loss heal through the observe
/// supervisor (AH2): reconnect with backoff, last-good kept, quality floor
/// on out-of-memory. Consumers: WIN1/ED1 frame loops; the in-crate
/// `SimulatedSurface` forced-loss tests are the first consumer.
pub mod heal;

/// Generational staging-belt uploads (RA6): ring of in-flight frame
/// generations, retire-after-submit discipline, `invalidate_all` for the
/// AH2 resize/loss hook. Std-only; consumers are the RA2 executor and the
/// WIN1/ED1 frame loops.
pub mod uploads;

/// Persisted pipeline cache with async compile and last-good fallback
/// (RA5): versioned on-disk blobs keyed by shader hash + define set +
/// layout id. Std-only; consumer is the RA2 executor's pipeline path.
pub mod cache;

/// Debug line sink (RL58 slice 1): named line groups with a per-group depth
/// bias, a per-frame recorder and the pure line-vertex build. Std-only; the
/// consumer is the RL58 live line pass plus the Rapier backend (later slices).
pub mod debug_draw;

/// Scene draw list with culling and hysteresis LOD (RA3): pure function
/// of (snapshot, camera, lod table), integer-stable order. Std-only;
/// consumer is the RA2 executor, which feeds back `lod` into `prev_lod`.
pub mod draw;

/// Software Hi-Z occlusion for the tiers with no depth prepass (SW4): a
/// 256x144 max-depth buffer, nine mips to one texel, AABB against the four
/// parent texels. Std-only, no GPU call.
pub mod occl_cpu;

/// Static prop batching for the potato tier (SW153): the arena's dressing
/// baked into one vertex/index pair at load, with a quantised vertex weld.
/// Std-only, load-time only.
pub mod static_batch;

/// Draw-id target CPU contract: index to texel id plus readback tally.
pub mod draw_id;

/// GPU frustum cull slice 1 (GS18) device runner: one compute pass writes a
/// 0/1 flag per static instance plus the 5-u32 indirect args over the shared
/// `shaders/cull.wgsl`, then refuses when the device set misses the `draw`
/// CPU oracle. Needs the `window` feature; the GS18b
/// `draw_indexed_indirect` hook is the caller (the row states no draw path
/// uses the cull yet).
#[cfg(feature = "gpu")]
pub mod gpu_cull;

/// Reused per-frame draw list plus uniform and draw encode over one
/// bump arena. Std-only; the submit path reads the encoded bytes.
pub mod frame;

/// One reusable byte buffer as a per-frame bump arena. Std-only;
/// consumer is the sibling `frame` module, which encodes into it.
pub mod frame_alloc;

/// Instanced reacting crowd staging: seat layout, hysteresis LOD, sector
/// cull hook and caller-owned instance builds over the present seat math.
/// Std-only; consumer is the game feed plus the next present bind.
pub mod crowd_inst;

/// LOD dither cross-fade, no pop, no alpha sort (GS17): fade share plus
/// the 4x4 Bayer keep rule with its WGSL mirror. Std-only; consumer is
/// the `draw` LOD path, which binds `keep` per pixel next slice.
pub mod lod_fade;

/// Octahedral impostors, CPU half: direction <-> atlas UV, the 3 nearest
/// baked views with weights, atlas layout under a byte budget and the
/// screen-space LOD switch with hysteresis. Std-only; `shaders/impostor.wgsl`
/// mirrors it, and the bake and draw passes under `present/` bind both.
pub mod impostor;

/// Octahedral impostor GPU half (RL64): bake a posed mesh into an albedo and
/// normal atlas once, draw it as instanced camera-facing quads inside the
/// forward pass. The CPU math is [`impostor`]; window build only.
#[cfg(feature = "gpu")]
#[path = "present/impostor_gpu.rs"]
pub mod impostor_gpu;

/// Wound paint budget queue: one paint list per character with a capped
/// per-frame drain and oldest-map eviction. Std-only; the decal atlas bind
/// drains it next slice.
pub mod wound_map;

/// Severed-limb stump cap and limb cylinder meshes (RZ3): one closed stub per
/// cut plus world placement, std-only. Consumer is the game limb look, which
/// bakes a cap per cut and a cylinder per falling limb into resident keys.
pub mod stump;

/// CPU-side frame-graph executor (RA2): runs the compiled graph over
/// recorded pass fns, plans transient aliasing, refuses GPU submit with
/// a typed error until the bind groups land. Std-only.
pub mod executor;

/// BLAS per CT2 proxy mesh plus per-frame TLAS refit (RT1): CPU mirror plus
/// digests with a window device validation submit. Std-only on the headless
/// twin; consumer is the WIN1 match (arena plus 8 fighters), then RT2/RT3/RT4.
pub mod accel;
/// Choose the GPU adapter on purpose (GS6): env-driven choice ranked over an
/// enumerated list, software refused unless opted in. Std-only; consumer is
/// `presenter_pick` plus the presenter request sites.
pub mod adapter;
/// Arena day light-rig preset: one sun angle with the sky bake driving
/// ambient level and fog color as data. Std-only; consumer is the window
/// presenter through the shared look bake plus the capture bundle.
pub mod arena_day;
/// GPU backend selector plus headless 600-frame bench: `--backend` /
/// `WGPU_BACKEND` over the compiled-in default, pilot-report winner with the
/// other as fallback; the fixed arena600 orbit report. Std-only; consumers
/// are the `gpu` window path and the `present` offscreen/windowed entries.
pub mod backend;
/// Bake slice of the baked-GI pass (validity mask + fixture inputs):
/// bake identical bytes x2; consumer is the arena frame plus captures.
pub mod baked_bake;
/// Baked lightmaps with validity mask + SH probes reusing the ibl SH path
/// (RL5): bake identical bytes x2; consumer is the WIN1/ED1 arena + Q1.
pub mod baked_gi;
/// Debug G-buffer frame (A3): octahedral normal, depth, albedo and material
/// id with one digest each, built from the slices the crate already holds.
/// Std-only and headless; the GPU readback seam is named, not built.
pub mod buffers;
/// Hero motion-hold clip: fixed 120-frame slow push-in over the twin
/// establishing view through the frozen temporal resolve. Std-only;
/// consumer is the twin establishing view through `present::look`.
pub mod clip_hold;
/// Deferred blood decals with depth reject (RP3): sand-not-air fixtures,
/// roughness drying curve. Std-only; consumer is WIN1 CB2 gore mapping.
pub mod decals;
/// wgpu glue over the adapter choice (GS6): enumerate and pick. Behind
/// `window` so the headless default stays std-only; consumer is the
/// presenter request sites.
#[cfg(feature = "gpu")]
pub mod presenter_pick;

/// The quarter-res screen-space flow field (SW41): one stable-fluids step on
/// the CPU (curl, vorticity, divergence, 20 Jacobi pressure sweeps, gradient
/// subtract, advect velocity then dye) that the WGSL pass must match. Std-only,
/// view only.
pub mod flow_field;

/// Height fog + contact shadows live on the GPU (RL7): fog ramps from the
/// bound RL4 day capture, contact pass for small details. Std-only;
/// consumer is the WIN1 forward graph via `compile_fog_for_forward`.
pub mod fog;

/// SW91: quarter-res sun shafts, the CPU parity oracle of `godray.wgsl`.
pub mod godray;
/// GTAO ambient occlusion at half resolution + the shared LookFrame
/// bind shape (RL3): bilateral upsample, CPU golden bytes, GPU submit
/// refused until the RA1 bind groups land. Std-only.
pub mod gtao;
/// QA5: GTAO tier table (half on potato and web), half-res target sizing.
pub mod gtao_half;
/// PF2 capture manifest (RA8): adapter/driver/resolution+quality/scene
/// digest, all four required. Std-only; consumer is PF2/Q1 captures.
pub mod manifest;
/// Layered PBR materials (RL6): cloth, sand, wet blood over the R1 core
/// by composition, no mirrored types. Std-only.
pub mod material_layer;
/// Render-side bins from draws to compiled materials (RL46 slice 1):
/// bin keys, frame bin sets and the 12-pipeline budget (std-only), plus on
/// the window leg `pipeline_for` (NS31c2): one wgpu pipeline per compiled
/// material structure. Consumer is the forward-pass bind (RL46), which
/// looks pipelines up by structure key.
pub mod material_pipeline;
/// Seeded GPU particle pool (RP2, QL12): dust/flame/sparks/blood, per-tier
/// kind caps, CPU-sim parity bytes. Std-only; consumer is the arena match
/// through WIN1.
pub mod particles_gpu;
/// Cascade-split selection plus per-cascade map sizes for the arena-day
/// sun (RL27): pinned splits over the camera range, std-only; consumer
/// is the day-sweep fixture, then RL30.
pub mod shadow_cascade;
/// SW92: the CPU parity oracle of the forward shader's sun compare filter
/// (3x3 grid, four rotated Bayer corners, two fixed diagonal taps).
pub mod shadow_pcf;
/// Physical sky + EV100 exposure (RL4): Hillaire-school radiance, day
/// param identical bytes x2. Std-only; feeds RL2 IBL and RL7 fog.
pub mod sky;
/// Test GPU gate: `require_or_skip` skips with a `SKIPPED` line (counted),
/// or fails under `E2040_REQUIRE_GPU=1`. Ad-hoc `SKIPPED` sites bypass it.
pub mod test_gpu;
/// Startup lighting-tier probe Base/Rt/Ultra (RT0): adapter ray-query plus
/// accel read, forced tiers refused loudly, tier stamped into RA8 manifests.
/// Std-only; consumer is RT9 tier-aware graph wiring (WIN1 match).
pub mod tier;
/// GPU facts to a quality tier through a first-match rule table in the Chromium
/// `gpu_control_list` shape (GS11). Std-only; consumer is the adapter pick (GS6).
pub mod tier_rules;

/// Pure integer dynres governor (GS12): GPU microseconds to scale percent.
pub mod dynres_gov;

/// Battery-aware quality cap (GS40): one Win32 power read beside the VRAM
/// leaf plus the pure rule that steps one tier down on a low or saver
/// battery, never onto `Web`. Cold path, off the tick, no hash input;
/// consumer is the startup tier pick.
pub mod power;

/// Mobile soak harness (BG8): the frame times and the resident set of a long
/// run, the p50 and the RSS it reports, and the thermal-cliff flag. Pure over
/// its inputs; the resident-set leaf reads `/proc/self/statm` where the host
/// has one. Consumer is the device soak on a phone.
pub mod soak;

#[cfg(feature = "window")]
pub mod gpu;

/// Forward+ tile grid: 1080p/16 = 120x68 = 8160 tiles. Overflow counter must stay 0.
pub const TILE: u32 = 16;
pub const MAX_LIGHTS_PER_TILE: usize = 24;
pub const GRID_W_1080P: usize = 120;
pub const GRID_H_1080P: usize = 68;
pub const TILE_COUNT_1080P: usize = GRID_W_1080P * GRID_H_1080P; // 8160

/// CPU mirror of GPU light grid: offset/count per tile + global index list.
/// Clamp push when `list.len() < MAX_LIGHTS_PER_TILE`, bump overflow otherwise.
#[derive(Default)]
pub struct LightGrid {
    pub counts: Vec<u8>,
    pub offsets: Vec<u32>,
    pub indices: Vec<u32>,
    pub overflow_tiles: u32,
}

impl LightGrid {
    pub fn new(tile_count: usize) -> Self {
        Self {
            counts: vec![0; tile_count],
            offsets: vec![0; tile_count],
            indices: vec![],
            overflow_tiles: 0,
        }
    }
    pub fn push(&mut self, tile: usize, light: u32) {
        if (self.counts[tile] as usize) < MAX_LIGHTS_PER_TILE {
            self.counts[tile] += 1;
            self.indices.push(light);
        } else {
            self.overflow_tiles += 1;
        }
    }
}

pub mod core;
pub mod gi;
/// Dynamic GI probe stub over the arena slice (RL-RT slice 1): a 32-probe
/// RGB grid refreshed per frame from the live rig, no bake. Std-only;
/// consumer is `GiRuntime::irradiance_mixed` in `gi`.
pub mod gi_probe;
/// Virtual-geometry cluster-cut oracle (ideas sketch): hierarchical triangle
/// groups swapped per view beside the oracle; the draw path stays out.
/// Std-only; consumer is the later submit pass, deferred here.
pub mod meshlet;
pub mod particles;
/// Ray-traced sun shadows over the arena sand (RL-RT slice 1): pinned
/// occluders, slab-tested shadow factors and the EG8 on/off pixel gate.
/// Std-only; consumer is `gi_probe` plus its frame shade.
pub mod rt_shadows;
pub mod shadow_post;

/// Engine-owned TAA/TAAU plus the GPU post chain (RP1): jittered
/// projection, motion vectors, reprojection with neighborhood clamp,
/// temporal upsampling, then bloom/exposure/ACES/grading reading the
/// RL1/RL2 passes. Consumers: WIN1/ED1 final frame, Q1 clips.
pub mod post;

/// Post jitter and motion leaf (CQ24): Halton-2/3 per-frame projection
/// jitter plus current-minus-history motion vectors, re-exported through
/// the `post` facade so `post::` callers keep their path.
pub mod post_jitter_motion;

/// RL36: the live frame's dual-filter bloom. A Karis-weighted bright mip,
/// a 13-tap downsample ladder and a 3x3 tent upsample ladder, plus the CPU
/// oracle the device chain is proven against. Consumers: the window frame's
/// blit (the halo it adds before the one tone map), the capture tests.
pub mod bloom_dual;

/// The bodycam lens: barrel distortion, chromatic aberration, vignette and
/// film grain in one post pass, with the eye, helmet and chest presets.
/// The eye is the identity and the default. Std-only oracle plus the WGSL;
/// the GPU pass and the capture helper ride the `window` feature.
pub mod lens;

/// The first-person helmet rim: the aperture outline table, the closed rim
/// mesh it builds, the rim's own depth range and the GPU pass that draws
/// it on its own depth target. Std-only core; the capture helper rides the
/// `window` feature.
pub mod viewmodel;

/// SMAA fallback plus dynamic resolution around RP1 (RP4): edge-detect
/// area blend for the TAA-off path plus a hysteresis scale controller on
/// PF2 frame ms. Std-only; consumer is the WIN1 viewport TAA-off path.
pub mod aa_scale;

/// Cascaded sun shadows plus clustered torch lights (RL1): validated sun
/// configs with R4-parity bakes behind `v1`, std-only cluster assignment,
/// typed `LightError` and per-pass ms receipts. Consumers: WIN1/ED1 lit
/// frame through the RA1 presenter, RP1 post chain.
pub mod shadow_cluster;

/// Twin sun-shadow bind: the window twin forward path samples
/// the landed production cascaded sun under the arena day-rig sun, so lit
/// faces carry the cascade shade and backfaces keep day ambient instead
/// of black. Std-only CPU twin plus the `window` uniform/shader glue;
/// consumer is the window twin forward bind and the capture bundle.
pub mod twin_shadow;

/// Live contact shade under the fighters' feet: the feet block the forward
/// shader reads (`shadow.feet`), its pack and the CPU oracle of
/// `sun_contact_factor`. Std-only; fed by `Presenter::set_contact_feet`.
pub mod contact_feet;

/// Screen-space contact shadows: the 16-step, 0.3 m, 0.1 m-thick march that
/// recovers the last centimetres where a body meets the ground. The CPU
/// oracle here pins the constants the live GTAO upsample runs.
pub mod contact_shadow;

/// Per-pass report: each pass declares reads and writes; the plan fails when
/// a read has no live writer, and the JSON names ran, culled and failed
/// passes with their reasons. Std-only, no clock.
pub mod pass_report;

/// Shadowed spot lights (flashlights and lamps) on the clustered path.
pub mod spot;

/// Torch point lights the scene feeds the forward pass (VS7): the
/// placement actors become lights, with a bounded presentation-only
/// flicker and the arena's night preset.
pub mod torch_light;

/// Shoulder camera geometry: how far behind the seat the eye sits, how far
/// to the side and how high. Std-only constants, so the headless crates that
/// place a fight inside a room read the same numbers the renderer uses.
pub mod shoulder;

/// Twin fighter draw bind (RL26): one box body per fighter slot (the hero
/// stand-in plus staged opponents) for the window twin's
/// `--fighters 8 --scene arena` frame list. Bind glue over the v1 draw
/// types; consumer is the twin `frame_list` in
/// `crates/game/src/bin/arena_capture.rs`.
#[cfg(feature = "v1")]
pub mod twin_fighters;

/// A lit arena ring with eight GPU-skinned fighters built from code alone (WEB5): the browser's
/// first arena frame, and the native twin of it. Consumer is `crates/web`.
#[cfg(feature = "v1")]
pub mod procedural_arena;

/// Twin arena kit bind: the recooked arena kit (CT8) replaces the synthetic
/// central cube when resident, else the cube stays as fallback. Bind glue
/// over the v1 draw types; consumer is the twin `frame_list` in
/// `crates/game/src/bin/arena_capture.rs`.
#[cfg(feature = "v1")]
pub mod twin_arena;

/// GPU image-based lighting from the Poly Haven HDRI (RL2): prefiltered
/// cubemap, irradiance SH and BRDF LUT baked at cook time behind `v1` with
/// exact R2 prefilter parity, std-only samplers plus the typed `IblError`,
/// GPU descriptors and the forward-pass sampling chunk behind `window`.
/// Consumers: WIN1/ED1 material frame, Q1 chart.
pub mod ibl;
/// QL2: the white-furnace test of the rough-metal energy gain (test only).
#[cfg(all(test, feature = "v1"))]
mod metal_energy;

/// The arena's own day sky as the live image-based light (replaces the photo
/// studio HDRI the sand used to reflect: white softbox streaks, straight-edged
/// white patches). Behind `v1` like the bake it feeds; consumer is the window
/// presenter's one-time IBL bake.
#[cfg(feature = "v1")]
pub mod ibl_arena_env;

/// Sword ribbon ring (RL53 slice 1): the sim's integer sweep segments ringed
/// and staged as a vert strip; consumer is the `crates/game` trail feed.
pub mod trail;

/// Sword ribbon GPU draw (RL53 slice 2): the staged uniform and strip buffer
/// the window presenter binds. Window build only; the CPU ring stays std-only.
/// Lives under `present/` with the crate's other window device code (the
/// GPU boundary gate reads the path, not the module name).
#[cfg(feature = "gpu")]
#[path = "present/trail_draw_gpu.rs"]
pub mod trail_draw_gpu;

/// Soft-depth seam of the blended sprite and ribbon passes: the depth bind group,
/// the per-frame view-depth constants and the light the particle quads and the
/// sword ribbon read, so neither slices through a body or the floor. Window
/// build only.
#[cfg(feature = "gpu")]
#[path = "present/vfx_soft.rs"]
pub mod vfx_soft;

/// Ground-splat blood draw (FEEL5): the world-space splat field a fight leaves
/// on the floor, staged and recorded by the window presenter. Window build
/// only; the CPU field stays in [`decals`].
#[cfg(feature = "gpu")]
#[path = "present/decal_draw_gpu.rs"]
pub mod decal_draw_gpu;

/// CPU naga gate over the shipped WGSL (CQ23): every file under `src/shaders`
/// validates without a GPU, and the window's hot-reload cook refuses an edit
/// that does not. Needs `shader-validate`; the default graph stays std-only.
#[cfg(feature = "shader-validate")]
pub mod shader_gate;

#[cfg(test)]
mod webgpu_limits;

/// Headless frame counter: `begin_frame` counts one frame per call and
/// `draw_count` reports the total. Deterministic; no IO, no allocation.
pub struct NullRenderer {
    frames: u64,
}

impl Default for NullRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl NullRenderer {
    /// Fresh counter at zero frames.
    pub fn new() -> Self {
        Self { frames: 0 }
    }

    /// Count one submitted frame for `tick` (tick is informational only).
    pub fn begin_frame(&mut self, _tick: u64) {
        self.frames += 1;
    }

    /// Total frames counted so far.
    pub fn draw_count(&self) -> u64 {
        self.frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_count_is_8160() {
        assert_eq!(
            (TILE, GRID_W_1080P, GRID_H_1080P, TILE_COUNT_1080P),
            (16, 120, 68, 8160)
        );
    }

    #[test]
    fn null_renderer_counts_frames() {
        let mut r = NullRenderer::new();
        assert_eq!(r.draw_count(), 0);
        r.begin_frame(0);
        r.begin_frame(1);
        r.begin_frame(2);
        assert_eq!(r.draw_count(), 3);
    }

    #[test]
    fn light_grid_push_clamp_overflow() {
        let mut g = LightGrid::new(TILE_COUNT_1080P);
        assert_eq!(
            (g.counts.len(), g.offsets.len(), g.overflow_tiles),
            (TILE_COUNT_1080P, TILE_COUNT_1080P, 0)
        );
        for l in 0..(MAX_LIGHTS_PER_TILE as u32) {
            g.push(0, l);
        }
        assert_eq!(
            (g.counts[0] as usize, g.indices.len(), g.overflow_tiles),
            (MAX_LIGHTS_PER_TILE, MAX_LIGHTS_PER_TILE, 0)
        );
        g.push(0, 999);
        g.push(0, 1000);
        assert_eq!(
            (g.counts[0] as usize, g.indices.len(), g.overflow_tiles),
            (MAX_LIGHTS_PER_TILE, MAX_LIGHTS_PER_TILE, 2)
        );
    }
}
