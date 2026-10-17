# Render reference (`crates/render`)

Detail for humans and AIs. Every name, number and command here was read from the code on 2026-10-17. A value marked "measured" has a dated source file. A value marked "target" is a budget in a plan, not a result.

Crate: `engine-render` (`crates/render/Cargo.toml`). Default build is std-only (no GPU, no external crate). Features:

| Feature | Adds | Implies |
|---|---|---|
| (none) | `NullRenderer`, the CPU mirrors, the GPU-free contracts | |
| `v1` | glam camera math, PNG capture, JSON manifest, `v1.rs`, `v1_ibl_shadow.rs`, `v1_skin.rs`, `v1_lighting.rs`, `twin_fighters.rs`, `twin_arena.rs` (needs `engine-assets` import) | |
| `window` | wgpu, winit, pollster, DXGI budget read on Windows, `present/gpu_present/*`, `window.rs`, `gpu.rs`, `v1_gpu.rs`, `gpu_cull.rs`, `pass_truth/harness.rs`, `presenter_pick.rs`, `fog_fwd.rs` | `v1` |

The web build needs `window`, which compiles for `wasm32-unknown-unknown` today. It does not run there yet: see `ref-web.md` section 6 (blocking GPU calls).

## 1. Module map

Crate root `crates/render/src/lib.rs`. "Row" is the board row that owns the module (ids as written in the module doc).

### 1.1 Frame, presenter and device

| Module | Row | Role (from the module doc) |
|---|---|---|
| `present.rs` | RA1 | Windowed presenter over a draw list. Constants `MAX_FRAME_DIM = 2048`, `MAX_TEXTURE_DIM = 2048`, `MAX_DRAWS_PER_FRAME = 256`, `MAX_CACHED_MESHES = 512`, `MAX_CACHED_TEXTURES = 512` (insertion-order eviction above the last two). |
| `present/gpu_present.rs` + `gpu_present/*` | RA1 | GPU presenter (`window` only). Files: `presenter.rs` (type `Presenter`), `submit.rs` (`render_offscreen`, encode), `live.rs` (live window frame and its switches), `frame.rs` (`FrameOutput`, `FrameReceipt`, `LookPassReceipt`, `LivePixels`), `gate.rs` (`capture_live_pixels`), `gtao_live.rs`, `sun_shadow.rs`, `forward_bind.rs`, `bind_memo.rs`, `fog_tranche.rs`, `msaa.rs`, `resolve.rs`, `timing.rs`. |
| `present/surface.rs` | RA1, AH2 | `SurfaceEvent`, `SurfaceAction`, `RecoveryAction`, `PresentError`. Typed errors with a recovery action. |
| `present/aa.rs` | RP6 | `AaChoice::{Taa, SmaaFallback}` and `select_aa`. |
| `present/look.rs` | RL8, RL9 | Look bake helpers for captures (`fog_params_for_capture`, `ibl_ambient_for_capture`, `skybox_sample_for_capture`, `contact_for_frame`). |
| `present/wait_stats.rs` | | Wait counters for live frames. |
| `window.rs` | | `WindowShell`: frame count and fixed-dt accumulator. Window events never feed `state_hash`. |
| `gpu.rs` (window) | | Instance, surface and device. Device request uses `DeviceDescriptor { label, ..Default::default() }` (wgpu default limits). Blocks with `pollster::block_on`. |
| `presenter_pick.rs` (window) | GS6 | wgpu glue over `adapter.rs`. |
| `adapter.rs` | GS6 | Adapter choice; software adapters refused unless opted in. |
| `backend.rs` | GS6 | Backend selector: `--backend` or `WGPU_BACKEND` over the compiled default; headless 600-frame bench. |
| `heal.rs` | AH2 | Surface Outdated/Lost and device-loss recovery through the observe supervisor. |
| `cache.rs` | RA5 | Persisted pipeline cache: async compile, last-good fallback. |
| `executor.rs` | RA2 | CPU frame-graph executor over recorded pass fns. Refuses GPU submit on the headless twin. |
| `graph.rs` | RA2 | Render graph as data; null order equals wgpu order; `submitter` on window builds. |
| `frame.rs`, `frame_alloc.rs` | RA2 | Reused per-frame draw list and uniform encode over one bump arena. |
| `draw.rs`, `draw_id.rs` | RA3, SP63 | Draw list with frustum culling and hysteresis LOD select; draw-id target contract. |
| `lod_fade.rs` | GS17 | LOD dither cross-fade (Bayer keep rule with WGSL mirror). |
| `dynres_gov.rs` | GS12 | Integer dynamic-resolution governor: GPU microseconds to scale percent. |
| `aa_scale.rs` | RP4 | SMAA fallback and `ScaleController` (hysteresis on frame ms). |
| `manifest.rs` | RA8, PF2 | Capture manifest: adapter, driver, resolution and quality, scene digest. |
| `variants.rs` | RA7 | Shader variant manifest with a count gate. |
| `test_gpu.rs` | | `require_or_skip`: prints `SKIPPED`; `E2040_REQUIRE_GPU=1` turns a skip into a failure (only for tests that call it). |

### 1.2 Lighting and shadows

| Module | Row | Role |
|---|---|---|
| `sky.rs` | RL4 | Physical sky bake (Hillaire 2020 school, reimplemented) and EV100 exposure. Feeds IBL and fog. |
| `ibl.rs` | RL2 | Image-based light from the HDRI: prefiltered cubemap, irradiance SH, BRDF LUT (baked at cook time). Window descriptors behind `window`. |
| `arena_day.rs` | | One EV100 arena day light-rig preset as data. |
| `shadow_cluster.rs` | RL1 | Cascaded sun shadows plus clustered torch lights; `LightError`; ms per pass. |
| `shadow_cascade.rs` | RL27 | Cascade split selection and per-cascade map sizes for the arena-day sun. |
| `shadow_post.rs` | | CSM parameters, ACES, Sunset and Noir presets (data). |
| `present/gpu_present/sun_shadow.rs` | | Sun shadow depth and bind on the live frame. |
| `twin_shadow.rs` | | Sun-shadow bind for the window twin (`arena_capture`). |
| `gtao.rs` | RL3 | Half-resolution GTAO plus the shared `LookFrame` bind shape. `GTAO_BUDGET_MS = 0.8`. |
| `gtao` device path | LF3 | `present/gpu_present/gtao_live.rs`: GTAO on the device writes `ao_tex`; no readback. |
| `v1_lighting.rs` | R4 | Cascaded shadows, probe GI, bloom factors, Halton TAA jitter (`v1`). |
| `gi.rs` | | Baked GI: hemisphere lightmap and SH probes; runtime is lookup only. |
| `baked_gi.rs`, `baked_bake.rs` | RL5 | Arena lightmaps with validity mask and fighter SH probes; bake is identical bytes twice. |
| `gi_probe.rs` | RL-RT | Dynamic GI probe stub over the arena slice (32 probes). |
| `rt_shadows.rs` | RL-RT | Ray-traced sun shadow factors over the sand (CPU, pinned occluders). |
| `accel.rs` | RT1 | BLAS per proxy mesh and per-frame TLAS refit. |
| `tier.rs` | RT0 | Startup lighting tier `Base`, `Rt`, `Ultra` (see section 6.2). |
| `restir_di.rs` | RT | Direct-illumination reservoir oracle and denoise cost. |

### 1.3 Post, AA and particles

| Module | Row | Role |
|---|---|---|
| `post.rs` | RP1 | TAA resolve (`taa_resolve`), TAAU (`taa_upsample`), bloom (`bloom_should_run`, `apply_bloom`), exposure (`exposure_mult`, `AutoExposure`), ACES (`tonemap_aces`), FSR1 EASU/RCAS oracle (`fsr_upscale`), `PostChain`. Holds `PORT()` markers for donor shapes. |
| `post_jitter_motion.rs` | CQ24 | Halton 2/3 jitter and motion vectors; re-exported by `post::`. |
| `present/gpu_present/resolve.rs`, `msaa.rs` | EG13 | 4x MSAA plan and box resolve. Does not touch `post::taa_resolve`. |
| `particles.rs` | RP2 | Seeded CPU mirror of GPU FX at 60 Hz, fixed emitter order, hashed. |
| `particles_gpu.rs` | RP2 | Seeded GPU pool for dust, torch flame, sparks. Bind submit is `particle_ms` in `LookPassReceipt`. |
| `particle_draw_gpu.rs`, `vfx_hanabi.rs` | RP2 | Flipbook draw bytes and one batched effect dispatch over the pool. |
| `fluids_gpu.rs` | | Seeded presentation fluid pool (blood, water droplets). |
| `decals.rs` | RP3 | Deferred blood decals with depth reject and roughness override. Bind submit is `decal_ms`. |
| `fog.rs`, `fog_fwd.rs` | RL7 | Height fog and contact shadows (`fog_factor`, `contact_ao`); per-fragment fog in the forward shader (window). |
| `material_layer.rs` | RL6 | Cloth, sand and wet-blood layers over the PBR core (reflectance math). |
| `trail.rs` | RL53 | Sword ribbon ring from the sim's integer sweeps. |
| `wound_map.rs` | | Wound paint budget queue, capped drain, oldest-map eviction. |
| `overlay.rs` | EG14 | HUD quad ring (CPU staging). |
| `debug_draw.rs` | RL58 | Debug line groups with depth bias. |
| `crowd_inst.rs`, `crowd_seats.rs`, `crowd_impostor.rs` | | Crowd staging, 96-seat stands layout, octahedral far-row select. |
| `upscale_easu.rs` | FC3 | Pinned 720p to 1080p spatial upscale. |
| `meshlet.rs` | VG | Virtual-geometry cluster-cut oracle; no draw path. |

### 1.4 Window twin and v1 receipts

| Module | Feature | Role |
|---|---|---|
| `v1.rs`, `v1_gpu.rs` | `v1`, `window` | Forward PBR receipt (R1). `v1_gpu.rs` renders the shared shader `shaders/forward.wgsl` offscreen. |
| `v1_skin.rs` | `v1` | GPU skinning joint matrices and weight validation (R3). |
| `v1_ibl_shadow.rs` | `v1` | IBL and single shadow map (R2). |
| `v1_temporal.rs` | test only | Temporal upscale parity oracle. |
| `twin_fighters.rs`, `twin_arena.rs` | `v1` | Binds for the window twin frame list in `crates/game/src/bin/arena_capture.rs`. |
| `pass_truth.rs` (+ `pass_truth/`) | EG8 | Pass-truth gate, section 4. |

Shader sources: `crates/render/src/shaders/` (`forward.wgsl`, `blit.wgsl`, `tonemap.wgsl`, `bloom.wgsl`, `ssao.wgsl`, `cull.wgsl`, `cluster.wgsl`, `skin.wgsl`, `particle.wgsl`, `trail.wgsl`, `fog_vol.wgsl`, `lod_dither.wgsl`, `crowd_vat.wgsl`, `fluids_present.wgsl`, `forward_maps.wgsl`, `post/`). Compute WGSL is also embedded in `sky.rs`, `gtao.rs`, `fog.rs`, `decals.rs`, `material_layer.rs`, `particles_gpu.rs`, `shadow_cascade.rs`, `accel.rs`, `aa_scale.rs`, `baked_gi.rs`.

## 2. Live window frame

The live frame is the window's path (`present_frame_live` in `present/gpu_present/live.rs`). Order as the code encodes it:

1. Stage the frame and write the uniforms (`write_frame_uniforms`, camera, light and the IBL flag from the look).
2. Bind the twin sun shadow (`bind_twin_shadow`), then write fog (`write_fog_forward`). Fog is per fragment in the forward shader.
3. Forward pass `ra1-forward` into the HDR target (`Rgba16Float`, linear). Sky is a far-plane draw in the same encode (`encode_forward`, label `ra1-sky`) and is skipped only when the sky row's look toggle is off (`live_sky_should_draw`). IBL is bound inside forward (`ra1-ibl-*`).
4. GTAO on the device (`encode_gtao`, label `ra1-gtao`) writes `ao_tex` after forward. No readback, no wait.
5. Bloom gate: `live_bloom_for_frame` decides from the CPU mirror of the HDR peak (`live_bloom_max_luma`). The flag rides the fog uniform's padding (`FogU::_pad0`): 0.0 keeps the frame identical, 1.0 lets the blit halo lift.
6. Blit (`ra1-blit`): one tone map (ACES then sRGB) into the swapchain view, multiplying the AO and fog pair on linear radiance.
7. Overlay (`ra1-overlay`), HUD quads.

Not on the live frame: TAA. `present/gpu_present/live.rs` has no TAA call; `post.rs` holds the TAA resolve and its CPU oracle, and `aa.rs` picks `Taa` or `SmaaFallback` by the `aa_scale` controller. Particles and decals bind in the live frame (`particle_ms`, `decal_ms`). Look prints per pass are in `LookPassReceipt` (`sky_ms`, `layer_ms`, `particle_ms`, `decal_ms`, and the rest).

Blocking calls on the live path: `live.rs` says the live window frame never waits on the GPU. Blocking probes (bind probes, IBL chart, depth readback with fog and GTAO) run only under `Presenter::set_look_probes(true)` or on the offscreen capture twin.

Public presenter entry points (`impl Presenter`):

| Method | Use |
|---|---|
| `resize(width, height)` | Resize targets; keeps last-good on failure. |
| `render_offscreen(&DrawList)` | Offscreen frame with all passes (`FrameOutput`). |
| `render_offscreen_with_toggles(&DrawList, &LookToggles)` | Same with look passes switched off. |
| `render_live_offscreen(&DrawList)` | Live-frame path into a device-only target; returns `FrameReceipt`. |
| `render_live_offscreen_with_toggles(&DrawList, &LookToggles)` | Live path with toggles (used by the gate). |
| `capture_live_pixels(&DrawList, &LookToggles)` | Readback of RGBA plus the draw-id target (`LivePixels`). |
| `set_look_probes(bool)` | Opt-in blocking probes. |
| `set_probe_every(u32)`, `set_fog_every(u32)` | Probe cadence. |
| `set_msaa_enabled(bool)`, `set_fog_enabled(bool)`, `set_bloom_enabled(bool)` | Pass switches. |
| `gpu_timing_supported() -> bool`, `settle_gpu_timing()`, `gpu_timing_report() -> Option<GpuTimingReport>` | GPU timestamps (`timing.rs`). Needs the `TIMESTAMP_QUERY` feature. |
| `present_mode_name() -> Option<String>` | Present mode label for captures. |

Formats: HDR `HDR_FORMAT = Rgba16Float`; LDR twin `OUTPUT_FORMAT = Rgba8Unorm`; depth `DEPTH_FORMAT = Depth32Float`; readback rows 256-byte aligned (`ROW_ALIGN`). Forward clear `HDR_CLEAR` is linear `(0.01607, 0.01783, 0.02701, 1.0)`. The studio HDRI embedded in the window build is `content/arena/hdri/polyhaven-studio-small-09-1k.hdr` (1.6 MB, `IBL_STUDIO_HDRI_BYTES`).

## 3. Surface events and recovery

`present/surface.rs`:

| `SurfaceEvent` | Meaning |
|---|---|
| `AcquiredClean` | Fresh frame, surface matches. |
| `AcquiredSuboptimal` | Fresh frame, surface no longer matches: present, then fix. |
| `Outdated` | Reconfigure, then retry. |
| `Lost` | Reconfigure, then retry. |
| timeout | Skip this frame. |

`SurfaceAction`: `Render`, `RenderThenReconfigure`, `ReconfigureThenRetry`, `SkipRetryNext`, skip-keep-last-good. `RecoveryAction`: `RetryFrame`, `ReconfigureSurface`, `ReacquireDevice` (drop presenter, build a new one), `KeepLastGood`, skip. Every failure is a typed `PresentError`; `NoAdapter` means headless CI skips, never fails.

## 4. Pass-truth gate (EG8)

Question it answers: does a look pass change the pixels the window draws? Owner: render lane. Files: `crates/render/src/pass_truth.rs` (pure core, std-only), `pass_truth/live_gate.rs` (pure, std-only), `pass_truth/harness.rs` (device leg, `window`), `pass_truth/reads.rs` (gates caller-owned on/off captures).

### 4.1 Verdicts

| Verdict | Rule (code) |
|---|---|
| LIVE | `diff_pixels > PASS_LIVE_FLOOR` (floor `0`). The on and off frames differ inside the pass's region. |
| PROBE | Everything else: no difference, no pair, ragged input, or a region with zero counted pixels. `verdict_label` returns the string. |

Per pixel: `widest = max over R,G,B,A of |on - off|`; the pixel counts when `widest > PASS_EPSILON` (`PASS_EPSILON = 5`, levels 0..=255). Pixels with a nonzero mask byte never count (`frame_diff_count`). An identical pair returns 0 at once.

Region (`PassRegion`), cut from the frame's draw-id target by `region_mask`:

| Region | Counted pixels | Used by |
|---|---|---|
| `Frame` | every pixel | graph, post, particles_gpu |
| `Sky` | pixels no draw covers (id == `NO_DRAW_ID` = `u32::MAX`) | sky |
| `Geometry` | pixels a draw covers | shadow_cluster, ibl, gtao, material_layer, decals |

The live-frame arm uses the region; the synthetic self-test uses `PassSpec::live_today` instead (that field is never a verdict on a real frame).

### 4.2 The nine look rows (`LOOK_PASSES`, registry order)

| # | row | submit entry | region | `live_today` (self-test only) |
|---|---|---|---|---|
| 0 | graph | `submit_gpu` | frame | true |
| 1 | shadow_cluster | `submit_gpu` | geometry | true |
| 2 | ibl | `submit_gpu` | geometry | true |
| 3 | post | `submit_gpu` | frame | true |
| 4 | gtao | `submit_gpu` | geometry | true |
| 5 | sky | `submit_gpu_sky` | sky | false |
| 6 | material_layer | `submit_gpu` | geometry | true |
| 7 | particles_gpu | `submit_gpu` | frame | true |
| 8 | decals | `submit_gpu` | geometry | false |

`LookToggles` holds one bool per row (`all_on()`, `disable(row)`, `is_on(row)`; unknown rows do nothing). `submit_skips_for(&toggles)` returns the submit entry name for each disabled row, or `None`.

### 4.3 Live-frame arm

`harness::gate_live_scene(&mut Presenter, &DrawList) -> Result<LiveGate, PresentError>`:

1. Render the all-on frame twice with `capture_live_pixels`. If the two differ, or the ids differ, it fails: `pass truth: the all-on frame differs from itself in N px`. A verdict is never guessed.
2. For each row, render with that row off and gate the pair with `live_gate::gate_live_pair`.
3. Return `LiveGate { rows, width, height, noise_px }`. `LiveGate::report(date, scene)` writes the markdown.

Costs: ten blocking frames and readbacks per scene (one extra for the repeat check), cold path only, never in a window run (module doc).

Pure entry points (no GPU): `gate_live_pair(&LivePair)`, `gate_live_rows(&[LivePair])`, `live_gate_report(date, scene, (w,h), noise_px, &rows)`, `write_live_gate_report(root, date, &report) -> io::Result<PathBuf>`, `diff_in_draws(on, off, ids, draws, w, h)` (surface-specific region test).

A pass with no pair reads PROBE with `paired = false` (report column `none`). Counted pixels of zero force PROBE.

### 4.4 Report format

Written to `<repo>/target/reports/pass-truth-<date>.md` (`pass_truth_filename`). Header and table (from `live_gate_report`):

```
# pass truth <date>

scene: <scene>, <w>x<h> px, the window's live frame path
noise: <n> px (all-on frame diffed against itself)

| row | verdict | diff | source | region | counted | pass |
| graph | LIVE | 10057 | live | frame | 196608 | submit_gpu |
...
Gate: EG8 live-frame arm (...). ... epsilon 5 with floor 0.
```

The offline table writer `pass_truth_report(date, table)` and `gate_dated_report(date)` (synthetic frames) use the same column order.

### 4.5 How to run it

| Goal | Command | Result |
|---|---|---|
| Pure rules (no GPU) | `node tools/gh-check.mjs cargo test -p engine-render --lib pass_truth` | Runs on GitHub. GPU tests in the filter run on the PC by themselves (gh-check rule for window filters). |
| Live gate on the render crate's test scene | `node tools/gh-check.mjs cargo test -p engine-render --features window --lib the_gate_reads_pixels_for_every_look_pass_and_repeats` | Needs an adapter. Without one the test prints `SKIPPED` (or fails with `E2040_REQUIRE_GPU=1`). |
| Live gate on the real arena kit frame | `node tools/gh-check.mjs cargo test -p engine-game --features app --lib pass_truth_gates_the_real_arena_kit_frame` | Writes `target/reports/pass-truth-<date>.md`. `<date>` is the env var `PASS_TRUTH_DATE` (default `arena`). Runs on the PC. |

Plan gate for the whole table: `docs/SPRINT-22-web-realism.md` row VR1 (LIVE for every look pass, native and web, same table twice). Not met yet (section 8).

### 4.6 Result on record

`target/reports/pass-truth-2026-10-17.md` (measured, arena kit placement, establishing camera, 512x384, live frame path, noise 0 px):

| row | verdict | diff px | counted px |
|---|---|---|---|
| graph | LIVE | 10057 | 196608 |
| shadow_cluster | PROBE | 0 | 10059 |
| ibl | LIVE | 10059 | 10059 |
| post | PROBE | 0 | 196608 |
| gtao | PROBE | 0 | 10059 |
| sky | LIVE | 186549 | 186549 |
| material_layer | PROBE | 0 | 10059 |
| particles_gpu | PROBE | 0 | 196608 |
| decals | PROBE | 0 | 10059 |

Result: 3 LIVE, 6 PROBE. The same dated file records the gate's own tests (`pass-truth-arena-like.md`, `pass-truth-arena.md`) and older runs (2026-10-13, 2026-10-14).

## 5. Present path notes

| Item | Value | Where |
|---|---|---|
| Frame size bound | `MAX_FRAME_DIM = 2048` (equal to `v1::MAX_CAPTURE_DIM`, asserted in debug) | `present.rs` |
| Draws per frame | 256; over the limit is refused | `present.rs` |
| Tone map | once, in the blit (`shaders/tonemap.wgsl` prepended to `blit.wgsl`) | `gpu_present.rs` |
| `LookPassReceipt` | field doc says "4 on every live frame" while `LOOK_PASSES` has 9 rows; the two counts are not reconciled in the code | `gpu_present/frame.rs` |

## 6. Quality tiers

### 6.1 `quality.rs` (GS10 slice 1): render presets as data

Units per knob are in `quality::unit`. All integer. `knobs(Quality) -> Knobs` is `const fn`.

| Knob | unit | Potato | Comfort | High |
|---|---|---|---|---|
| `render_scale_pct` | % | 67 | 100 | 100 |
| `shadow_cascades` | count | 1 | 3 | 4 |
| `shadow_map_px` | px | 1024 | 2048 | 4096 |
| `gtao` | level 0 off, 1 half, 2 full | Off | Half | Full |
| `bloom_half_res` | bool | false | true | true |
| `taa` | bool | false | true | true |
| `msaa` | x | 0 | 0 | 4 |
| `aniso` | x | 0 | 4 | 8 |
| `mip_bias_x100` | 1/100 | 100 | 100 | 100 |
| `stream_pool_mb` | MB | 200 | 700 | 1400 |
| `particle_cap` | count | 512 | 4096 | 8192 |
| `decal_cap` | count | 64 | 256 | 512 |
| `crowd_cap` | cards | 1500 | 5000 | 10000 |

What each tier cuts, in words: Potato renders at 67 percent, one cascade, no GTAO, no TAA, no MSAA, no anisotropy, and the smallest caps. Comfort adds half-res GTAO, TAA, three cascades, and mid caps. High adds full GTAO, four cascades, 4x MSAA, and the largest caps.

`CUT_ORDER` (what goes first, fixed): `crowd`, `particles`, `aniso`, `motion_blur`, `ssr`, `ao`, `shadow_cascades`, `render_scale`.

`table_hash(q) -> u64`: FNV-1a over the 13 fields in struct order (bools as 0/1, `gtao` as its level, `mip_bias` little-endian). Test `quality_hash_stable_x2`.

Tests (`cargo test -p engine-render --lib quality`): `quality_monotone_tiers`, `quality_units_nonempty`, `quality_cut_order_fixed`, `quality_hash_stable_x2`.

Status: no crate outside `quality.rs` reads `knobs`, `table_hash` or `CUT_ORDER` (grep of `crates/`). The web quality tier WEB8 (`docs/SPRINT-22-web-realism.md`) is not in this file. Treat the table as data waiting for its consumer.

### 6.2 `tier.rs` (RT0): lighting tiers

Not the same as 6.1. Startup probe, one per process.

| Tier | Needs (`Tier::supports`) |
|---|---|
| `Base` | nothing (any adapter, software too) |
| `Rt` | `ray_query` and `accel` |
| `Ultra` | `ray_query`, `accel`, and device memory at least `ULTRA_MIN_MEMORY_BYTES = 8 GiB` |

`missing_reason` gives the sentence: for example `adapter lacks wgpu EXPERIMENTAL_RAY_QUERY support`, `adapter lacks acceleration-structure support`, `adapter reports less than 8 GiB of device memory`. Module doc: the RTX 3050 6GB reference lands on `Rt`. Entry points: `probe(&AdapterCaps)`, `probe_detailed`, `resolve(&caps, forced: Option<Tier>)`, `resolve_name(&caps, Option<&str>)`, `stamp_summary(&manifest, tier)`.

## 7. WebGPU default limits (`webgpu_limits.rs`)

Test-only (`#[cfg(test)] mod webgpu_limits;` in `lib.rs`). It parses every `.wgsl` under `crates/render/src/shaders/` by hand (comments dropped), collects `@group/@binding` declarations and `@compute`, `@vertex`, `@fragment` entry points, and charges each file's bindings to each of its entry points (an upper bound).

Limits the shaders must fit (WebGPU default limits, the ones the web build requests):

| Constant | Limit |
|---|---|
| `MAX_BIND_GROUPS` | 4 |
| `MAX_SAMPLED_TEXTURES_PER_STAGE` | 16 |
| `MAX_SAMPLERS_PER_STAGE` | 16 |
| `MAX_STORAGE_BUFFERS_PER_STAGE` | 8 |
| `MAX_STORAGE_TEXTURES_PER_STAGE` | 4 |
| `MAX_UNIFORM_BUFFERS_PER_STAGE` | 12 |
| `MAX_VERTEX_ATTRIBUTES` | 16 |

`EXCEPTIONS` is an empty list. A listed exception that fits the limits is an error ("stale exception"), so exceptions cannot outlive their reason. `maxVertexBuffers` is not checked (the host picks the buffer count).

Command: `node tools/gh-check.mjs cargo test -p engine-render --lib shaders_fit_webgpu_default_limits`. Prints one table row per entry point (`file`, `entry`, `stage`, `stor`, `stex`, `tex`, `samp`, `unif`, `grp`, `attr`, verdict), then a count line.

Failure text: `WebGPU default limits exceeded:` then one line per entry point, for example `<file> [<entry>]: storage buffers 9 > 8`.

Other tests in the file: `nine_storage_buffers_in_one_stage_are_over_the_limit`, `eight_storage_buffers_in_one_stage_fit`, `commented_bindings_are_skipped_and_vertex_inputs_are_capped`.

Why 8: the shadow cluster bind needs three storage buffers per stage, and the defaults give eight (`crates/web/src/limits.rs` doc). The web shell requests `Limits::defaults()` (`crates/web/src/limits.rs`). The render device request uses wgpu's default descriptor (`gpu.rs`).

Also in `lib.rs` (test): `aces_one_source_gate`, which keeps one ACES source in `shaders/tonemap.wgsl`.

## 8. The one-render-writer rule

Where it is written: `docs/plan/web-and-realism.md` section 3 ("VR rows share `crates/render/src/present/**` with the Look chain, so they queue behind LF1-LF3 and EG11-EG13 (one render writer at a time)"), and `docs/plan/render-look.md` line 23 ("One render writer at a time still holds") and line 70 ("one writer for the 32-byte look uniform"). `AGENTS.md` does not contain this rule. Its render-related lines are the golden hash and the proof commands only.

What it means in practice (from those files):

| Rule | Scope |
|---|---|
| One agent edits `crates/render/src/present/**` at a time | the Look chain and the VR rows wait in line |
| One writer for the 32-byte look uniform and the `LookFrame` image | `render-look.md` line 228 places the one writer in `graph.rs`; mirrors are a slim violation (line 72) |
| `sky.rs` takes plain arguments on the twin and reads `gtao` and graph bytes only behind `window` | RL8 and RL9 move binds, never imports |

Status 2026-10-17: the rule is a plan rule, not enforced by a tool.

## 9. Measured costs (720p and 1280x720)

| Date | Quantity | Value | Resolution, device | Source |
|---|---|---|---|---|
| 2026-10-17 | GTAO on the device (landing 1, commit `1bb6097`) | 0.165 ms | 720p | `docs/SPRINT-18-gaps.md` line 1415 |
| 2026-10-17 | look print, per pass, 300 frames | sky 24.092, layers 0.682, particles 0.852, decals 0.784, ibl 9.370, fog 7.864, gtao 6.192, shadow 0.037, forward 0.000 (ms) | 1280x720, RTX 3050 6GB Laptop, driver 32.0.16.1062, present mode Immediate | `target/reports/look-8c502dd6-20261017T0525Z.txt`, quoted in `docs/design/STAGES.md` lines 166-170 |
| 2026-10-17 | frame split, mean over 300 frames | present 45.962, fighters 4.973, sim 0.085 (ms); cpu p50 9.640, p99 266.078 | same | same file |

Reading the 2026-10-17 print: the `shadow` and `forward` columns are zero or near zero because those rows are open (EG11, EG12). `sky` at 24 ms in that print is a real print of an unoptimised slice, not a target.

## 10. Budgets (targets, not measurements)

Plans set these for the reference GPU (RTX 3050 6 GB) at 1080p. They are not checked by any gate today except `GTAO_BUDGET_MS`.

| Pass | Target ms, 1080p | Source |
|---|---|---|
| GTAO | 0.8 (`GTAO_BUDGET_MS`, checked in `gtao.rs`) | `gtao.rs:45`; `docs/plan/render-look.md` line 218 |
| sky | 0.4 | `render-look.md` line 219 |
| full post | 1.5 | `render-look.md` line 219 |
| particles | 1.0 | `render-look.md` line 219 |
| decals | 0.5 | `render-look.md` line 220 |
| fog | 0.2 (contact 0.3, SMAA 0.5) | `render-look.md` line 220 |
| shadows | 1.0 | `render-look.md` line 302 |
| bloom | 0.5 | `render-look.md` line 302 |
| TAA plus post | 1.5 | `render-look.md` line 302 |
| trails | 0.15 | `render-look.md` line 302 |
| crowd at 20k | 1.5 | `render-look.md` line 302 |
| froxel fog | 0.8 | `render-look.md` line 302 |
| 32 torches | 1.5 | `render-look.md` line 302 |

Frame goal: p95 under 16.6 ms at 1080p on the reference GPU with kit and two fighters (`render-look.md` line 531). Potato floor (gate G9): arena on Intel UHD with downlevel limits at proposal p50 50 ms, p99 80 ms, working set 3 GB (`docs/plan/potato.md` line 29, marked "proposals").

## 11. Tests and commands

| What | Command | Notes |
|---|---|---|
| Crate unit tests (headless) | `node tools/gh-check.mjs cargo test -p engine-render --lib` | No GPU. |
| One module | `node tools/gh-check.mjs cargo test -p engine-render --lib <filter>` | Filter by module path, for example `pass_truth`, `quality`, `shaders_fit`. |
| Window and GPU tests | `node tools/gh-check.mjs cargo test -p engine-render --features window --lib <filter>` | Runs on the PC by itself. |
| Integration tests | `node tools/gh-check.mjs cargo test -p engine-render --test it` | Modules of one binary in `crates/render/tests/it/` (AGENTS rule). Also a top-level `crates/render/tests/gi_vectors.rs`, which the AGENTS rule does not allow for new files. |
| Iterate locally | `cargo check -p engine-render` | AGENTS: local cargo only to iterate. |
| Clippy, format | `node tools/gh-check.mjs cargo clippy -p engine-render --all-targets -- -D warnings`; `rustfmt --edition 2021 --check <files>` | Never `cargo fmt`. |
| Pass-truth report | see section 4.5 | |
| Wasm leg (check only) | `cargo check -p engine-render --features window --target wasm32-unknown-unknown` | Type-checks; see `ref-web.md` section 7. |

## 12. Not done or suspicious

1. `quality.rs` knobs have no consumer. `CUT_ORDER` names `motion_blur` and `ssr`, which have no knob in `Knobs`.
2. The web quality tier (WEB8) does not exist in the code.
3. TAA is not on the live window frame (no call in `present/gpu_present/live.rs`). The `post` row covers the post chain the live path reaches (the bloom halo flag).
4. The pass-truth gate reports 3 LIVE of 9 on the 2026-10-17 kit frame (section 4.6). The VR1 target is all 9 LIVE on native and web.
5. `LookPassReceipt` says "4 on every live frame"; the gate has 9 rows. Reconcile the wording or the count.
6. `render` has 124 hazard hits for the browser (`node tools/wasm-gate.mjs`: 65 clock, 51 blocking GPU, 7 fs, 1 thread), listed in `ref-web.md` section 7.
7. `gpu.rs` (window) blocks on the adapter and device request with `pollster::block_on`. Most other `pollster::block_on` sites in the crate are GPU test scopes. The web build cannot use this path (`ref-web.md` section 7).
