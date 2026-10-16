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

pub mod window;

pub mod graph;

pub mod quality;

/// V1 forward-PBR receipt (R1): CPU rasterizer, behind the `v1` feature so
/// the headless default stays std-only.
#[cfg(feature = "v1")]
pub mod v1;

/// Live shader, material and texture reload (HR2): versioned swap over HR1
/// notices with last-good kept on failure. Behind `v1`; consumer is HR5
/// practice-mode wiring.
#[cfg(feature = "v1")]
pub mod live;

/// Shader variant manifest with count gate + frame-exact staged reload
/// (RA7): enumerates static defines, refuses over-cap variants loudly.
/// Std-only; feeds `cache::PipelineKey` defines and `live` swaps.
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

/// VFX (R5): dust and torch particles plus blood decals, with instanced-quad
/// upload reusing the v1 mesh path. Behind `v1`; consumers are GM1/CB2.
#[cfg(feature = "v1")]
pub mod v1_vfx;

/// V1 forward-PBR on wgpu (R1): offscreen capture with zero validation errors.
#[cfg(feature = "window")]
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
pub mod sprite_pack;

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

/// Demand-paged tile residency on the streaming path:
/// tile-LRU cap-192 plus pinned L0 beside the frozen `CpuCache` in
/// `stream`, carrying the LADDER:528 ACCEPTED spike. Std-only; consumer is
/// the RA6 staging-belt upload path, which pages tiles instead of whole
/// textures. The `present.rs:300-356` FIFO is untouched.
pub mod stream;

/// Persisted pipeline cache with async compile and last-good fallback
/// (RA5): versioned on-disk blobs keyed by shader hash + define set +
/// layout id. Std-only; consumer is the RA2 executor's pipeline path.
pub mod cache;

/// Scene draw list with culling and hysteresis LOD (RA3): pure function
/// of (snapshot, camera, lod table), integer-stable order. Std-only;
/// consumer is the RA2 executor, which feeds back `lod` into `prev_lod`.
pub mod draw;

/// Draw-id target CPU contract: index to texel id plus readback tally.
pub mod draw_id;

/// Draw-id readback reduce to per-body pixel counts (SP63 slice b).
/// Std-only; consumer is the slice-c visibility score.
pub mod draw_id_reduce;

/// Reused per-frame draw list plus uniform and draw encode over one
/// bump arena. Std-only; the submit path reads the encoded bytes.
pub mod frame;

/// One reusable byte buffer as a per-frame bump arena. Std-only;
/// consumer is the sibling `frame` module, which encodes into it.
pub mod frame_alloc;

/// GPU frustum cull plus its CPU oracle.
pub mod gpu_cull;

/// Instanced reacting crowd staging: seat layout, hysteresis LOD, sector
/// cull hook and caller-owned instance builds over the present seat math.
/// Std-only; consumer is the game feed plus the next present bind.
pub mod crowd_inst;

/// LOD dither cross-fade, no pop, no alpha sort (GS17): fade share plus
/// the 4x4 Bayer keep rule with its WGSL mirror. Std-only; consumer is
/// the `draw` LOD path, which binds `keep` per pixel next slice.
pub mod lod_fade;

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
/// Halton-sampled arena lightmap bake (RL5 bake-once path): Halton
/// hemisphere plus adaptive refine into the `baked_bake` owners; consumer
/// is `baked_gi::sample_arena_frame` via `halton_bake`.
pub mod baked_halton;
pub use baked_halton::{
    halton, halton23_scrambled, halton_bake, hemisphere_dir, num_samples, BOUNCE_1, BOUNCE_2,
    ERROR_TOL, INITIAL_SAMPLES, MAX_SAMPLES,
};
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
#[cfg(feature = "window")]
pub mod presenter_pick;

/// Wound paint budget queue (GS19 slice 1): one paint list per character,
/// capped drain per frame in stable order. Std-only; the next slice binds
/// the drain to the decal atlas.
pub mod wound_map;

/// Sword ribbon ring (RL53 slice 1): pure CPU trail staging in mm.
/// Std-only; consumer is the slice-2 GPU draw, which reads the ring.
pub mod trail;

/// Height fog + contact shadows live on the GPU (RL7): fog ramps from the
/// bound RL4 day capture, contact pass for small details. Std-only;
/// consumer is the WIN1 forward graph via `compile_fog_for_forward`.
pub mod fog;

/// Volumetric cloud layer over the RL4 sky bake: slab raymarch with
/// hashed offsets, deterministic reruns. Std-only; consumer is the
/// sky capture via `composite_clouds_over_capture`.
pub mod clouds;
/// Analytic volumetric fog plus one cloud slice over the height lane:
/// stepped extinction march with a quality-table step hook, CPU reference
/// plus the embedded device shader. Std-only; the root helper below reads
/// the march budget, the forward graph binds the pass next.
pub mod fog_vol;
/// GTAO ambient occlusion at half resolution + the shared LookFrame
/// bind shape (RL3): bilateral upsample, CPU golden bytes, GPU submit
/// refused until the RA1 bind groups land. Std-only.
pub mod gtao;
/// PF2 capture manifest (RA8): adapter/driver/resolution+quality/scene
/// digest, all four required. Std-only; consumer is PF2/Q1 captures.
pub mod manifest;
/// Layered PBR materials (RL6): cloth, sand, wet blood over the R1 core
/// by composition, no mirrored types. Std-only.
pub mod material_layer;
/// Render-side bins from draws to compiled materials (RL46 slice 1):
/// bin keys, frame bin sets and the 12-pipeline budget. Std-only;
/// consumer is the forward-pass bind, which looks compiled pipelines
/// up by bin key once the orchestrator wires the material dependency.
pub mod material_pipeline;
/// Seeded GPU particle pool (RP2): dust/flame/sparks, CPU-sim parity
/// bytes. Std-only; consumer is the arena match through WIN1.
pub mod particles_gpu;
/// Cascade-split selection plus per-cascade map sizes for the arena-day
/// sun (RL27): pinned splits over the camera range, std-only; consumer
/// is the day-sweep fixture, then RL30.
pub mod shadow_cascade;
/// Skin, eye and hair shade slice: wrapped-diffuse subsurface approx plus
/// wet-eye and strand glint, gated by the quality table. Std-only CPU
/// reference plus the slice WGSL; consumer is the forward-pass bind next.
pub mod skin;
/// Physical sky + EV100 exposure (RL4): Hillaire-school radiance, day
/// param identical bytes x2. Std-only; feeds RL2 IBL and RL7 fog.
pub mod sky;
/// Static Gaussian backdrop, presentation only (splat): PLY cook plus tile
/// composite with depth test on and depth write off. Std-only; consumer is
/// the tile first-pass presenter plus tests/it/splat_backdrop.rs.
pub mod splat;
/// Test GPU gate: a GPU test skips with a `SKIPPED` line, or fails under
/// `E2040_REQUIRE_GPU=1`. Consumers are the GPU-backed tests.
pub mod test_gpu;
/// Startup lighting-tier probe Base/Rt/Ultra (RT0): adapter ray-query plus
/// accel read, forced tiers refused loudly, tier stamped into RA8 manifests.
/// Std-only; consumer is RT9 tier-aware graph wiring (WIN1 match).
pub mod tier;
/// Display-transform CPU mirror plus the one-source hue gate (test-only).
#[cfg(test)]
mod tonemap;

/// Pure integer dynres governor (GS12): GPU microseconds to scale percent.
pub mod dynres_gov;
/// Tunnel-mouth exposure split: one baked frame holding sunlit sand
/// plus the dim gate-tunnel mouth off the frozen day rig. Std-only;
/// consumer is the window forward bind plus the capture bundle.
pub mod tunnel_mouth;

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
/// SDFGI-cascade CPU reference tables (E38b spike, flag-gated, never default-on).
pub mod gi_dyn;
/// Dynamic GI probe stub over the arena slice (RL-RT slice 1): a 32-probe
/// RGB grid refreshed per frame from the live rig, no bake. Std-only;
/// consumer is `GiRuntime::irradiance_mixed` in `gi`.
pub mod gi_probe;
/// Virtual-geometry cluster-cut oracle (ideas sketch): hierarchical triangle
/// groups swapped per view beside the oracle; the draw path stays out.
/// Std-only; consumer is the later submit pass, deferred here.
pub mod meshlet;
pub mod particles;
/// Half-res ReSTIR-GI reservoir oracle (CPU reference, flag-gated, never default-on).
pub mod restir_gi;
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

/// Twin fighter draw bind (RL26): one box body per fighter slot (the hero
/// stand-in plus staged opponents) for the window twin's
/// `--fighters 8 --scene arena` frame list. Bind glue over the v1 draw
/// types; consumer is the twin `frame_list` in
/// `crates/game/src/bin/arena_capture.rs`.
#[cfg(feature = "v1")]
pub mod twin_fighters;

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

/// Volumetric step hint for the window forward pass: march steps (count)
/// plus the cloud-slice toggle read from the quality table. Thin glue over
/// [`fog_vol`]; the march math and the table hook live there.
pub fn volumetric_hint(quality: quality::Quality) -> (u8, bool) {
    let config = fog_vol::config_for_quality(quality);
    (config.steps, config.cloud_on)
}

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

    #[test]
    fn draw_id_reduce_is_wired() {
        // Workspace-audit pin (SP63 slice b): the reduce module must stay
        // declared, or its unit tests silently stop compiling.
        assert_eq!(crate::draw_id_reduce::CLEAR_MIRROR, u32::MAX);
    }

    #[test]
    fn splat_is_wired() {
        // Workspace-audit pin (splat): src/splat.rs must stay declared in
        // lib.rs, or the backdrop silently stops compiling into the library
        // (the `#[path]` shim in tests/it/splat_backdrop.rs masked this).
        let row = crate::splat::LicenseRow {
            source: "fixture-local",
            terms: "test-only",
            capture: "wiring-pin",
        };
        assert_eq!(row.capture, "wiring-pin");
        assert!(crate::splat::SplatBackdrop::try_new(None, &[]).is_err());
    }
}
