# 06 New and young Rust 3D engines (steal hunt, 2026-10-18)

Scope: long tail of new and young Rust 3D engines (created or first pushed 2025-2026). Method: `gh api` search and repo reads on 2026-10-18, read-only; no clone, build or run. Gates: G1 golden, G2 speed, G3 lit frame, G6 same hash, G9 potato, G10 web. Read depth: FILE BODY READ means the body was fetched (partial where stated); LISTING ONLY means tree or README only.

## Engines scanned

repo | stars | created | pushed | license | one phrase | verdict
- steph4n-gh/voxel-demolition | 0 | 2026-10-10 | 2026-10-11 | Apache-2.0 | voxel demolition sandbox, siege engines, crush thresholds | KEEP (physics.rs, siege.rs read)
- Anandb71/minecraft-2 | 3 | 2026-09-15 | 2026-10-04 | MIT | sparse-voxel Minecraft in Rust, golden frames, AgX | KEEP (DECISIONS.md, calibration.rs read)
- valeriyr/ferrets | 4 | 2026-06-01 | 2026-10-17 | Apache-2.0 | deterministic lockstep RTS, replay crate | KEEP (replay.rs read)
- johannesPettersson80/scena | 26 | 2026-05-07 | 2026-10-12 | NOASSERTION (README: MIT or Apache) | scene-graph renderer, headless, strict GPU constructors | KEEP (headless-rendering.md read, first 5 KB)
- matthewjberger/webgpu-worker | 0 | 2026-06-04 | 2026-06-04 | Apache-2.0 | wgpu render loop inside a web worker on OffscreenCanvas | KEEP (worker.ts read)
- balaurengine/balaur | 96 | 2026-08-26 | 2026-10-17 | MIT | deterministic 2D/3D node engine, hot reload | KEEP for ARCHITECTURE.md (accumulator rule already held)
- RustingStudio/RustingEngine | 12 | 2026-08-26 | 2026-10-17 | NOASSERTION | GPU-simulated cubes, f32 determinism rules | KEEP for docs/determinism.md (mostly already have)
- milos-agathon/forge3d | 815 | 2025-07-30 | 2026-10-16 | MIT | wgpu/WebGPU renderer exposed to Python, terrain tiles | KEEP for terrain/page_table/scheduler.rs (first 3.5 KB)
- gmrdad82/pfx | 1 | 2026-10-08 | 2026-10-16 | MIT | game and render engine, 4K live-renderer claims | KEEP for README and live/demand.rs (first 2 KB)
- Nilbong/bevy_atmos | 0 | 2026-10-16 | 2026-10-17 | Apache-2.0 | sky, clouds, water, wet surfaces, look knobs | KEEP for KNOBS.md (first 3 KB)
- thesupersupersigma/rustyAC | 5 | 2026-10-06 | 2026-10-17 | Apache-2.0 (API) | Assetto Corsa rewrite claiming bit-exact physics | SKIP: built by reverse-engineering a commercial binary; nothing taken
- gh503/quench3d | 0 | 2026-10-15 | 2026-10-17 | Apache-2.0 | pure-Rust 3D engine, own physics, wgpu 30, web, MCP | LISTING + README only; solver crate worth a later read
- ludenea/ludenea | 0 | 2026-10-15 | 2026-10-17 | Apache-2.0 | deterministic engine and editor, ADR set | LISTING only; ADR titles overlap our determinism rules (0003 strict IEEE model)
- outerframehq/frame-engine | 1 | 2026-09-27 | 2026-10-17 | MPL-2.0 | headless fixed-step sim core | SKIP: DESIGN.md read, generic; sim/render split already ours
- panxinmiao/myth | 222 | 2026-01-04 | 2026-10-12 | Apache-2.0 | Rust rendering engine | LISTING only; render graph covered by known zenfg row
- larvance/telos | 1 | 2026-10-05 | 2026-10-17 | MIT | Vulkan voxel engine, telos-lod crate | LISTING only; voxel world is not our shape
- lobinuxsoft/kooch | 0 | 2025-07-05 | 2026-10-17 | NOASSERTION | GPU-driven meshlet renderer and editor | LISTING only; meshlets already in engine-landscape
- B4rtekk1/Minerust | 6 | 2025-12-27 | 2026-10-15 | MIT | voxel GPU indirect rendering | LISTING only; we have gpu_cull.rs
- IgorKonovalov/voxel-engine | 1 | 2026-10-16 | 2026-10-17 | Apache-2.0 | voxel ray-march in wgpu compute | LISTING only
- catchouli/starcity | 0 | 2026-10-11 | 2026-10-13 | none | browser transport and terrain game | SKIP: ENGINE.md read, ideas only, no license
- jarvis-jhk/towerfall-port, rounds-port | 0 | 2026-10-17 | 2026-10-17 | Apache-2.0 | clean-room rollback ports of commercial games | SKIP: game ports; netcode is known
- glashoff/noob_tube | 0 | 2026-09-05 | 2026-09-05 | AGPL-3.0 | Bevy FPS with rollback and prediction | SKIP: net is known; not read
- bhubbard/box3d-rs | 0 | 2026-10-04 | 2026-10-05 | MIT | Box3D port | SKIP: same donor family as known box3d-rust
- lazyxeon/AstraWeave | 7 | 2025-09-04 | 2026-10-17 | MIT | deterministic ECS for AI agents | SKIP: AI door, not this slice
- AmbientRun/Ambient | 3907 | 2022-12-19 | 2025-01-07 | Apache-2.0 | ECS game engine | SKIP: stale since 2025-01
- Name checks only, no directory read: Fyrox (9600 stars, pushed 2026-10-17), kiss3d (1789, pushed 2026-10-04), StardustXR/server (186, pushed 2026-10-17), veloren (7629, pushed 2026-10-17). Dust3D and Blue Engine: no repo found by name. hephaestus-rs: stale (2021). miniquad and Vx: no hit.

## Steals, ranked by value per effort

| # | donor repo@path (license, stars, pushed_at, gh api 2026-10-18) | exact thing to take | our file it lands in | grep proof (pattern -> hits) | gate moved | Value / Effort | read depth |
|---|---|---|---|---|---|---|---|
| 1 | matthewjberger/webgpu-worker@main web/src/worker.ts (Apache-2.0, 0 stars, pushed 2026-06-04) | Run the wgpu render loop in a dedicated worker on a transferred OffscreenCanvas (wgpu takes it as the surface, no winit); rAF lives in the worker; main thread forwards pointer and resize over postMessage (use raw postMessage, not Comlink); stats pushed at most every 250 ms; adapter info sent once at create | crates/web/src/shell.rs and arena.rs (wgpu request options at shell.rs:328, arena.rs:447 move into the worker); tools/web-build.mjs gets a worker bundle | "OffscreenCanvas\|new Worker\|Worker::new\|postMessage" over C:\engine2040 -> 0 (code and docs) | G10, plus G9 (no main-thread stall freezes input) | 4 / M | FILE BODY READ (worker.ts full, README); lib.rs not read |
| 2 | Anandb71/minecraft-2@main DECISIONS.md section D3 (MIT, 3 stars, pushed 2026-10-04) | Pin golden runs to one software adapter on dev box and CI (their WARP switch on Windows); tolerance is two numbers: mean abs channel error 1.5 AND at most 0.5% of pixels off by more than 12; prove the gate by perturbing one coefficient by 0.02 and checking the gate fails (their mean error 2.48) | crates/render/src/adapter.rs (opt-in E2040_ALLOW_SOFTWARE=1 already exists) and the golden harness in crates/render/tests/it | "request_adapter\|force_fallback\|software" in crates/render/tests -> 0; "FORCE_FALLBACK\|WGPU_ADAPTER_NAME" in crates -> 0 | G1, G3 | 3 / S | FILE BODY READ (DECISIONS.md D3 in full; file cut after D6 start) |
| 3 | steph4n-gh/voxel-demolition@main src/physics.rs (Apache-2.0, 0 stars, pushed 2026-10-11) | Crush energy = threshold x 20 (concrete 15 -> 300, rebar 25 -> 500); per-material crush threshold table with a penetrator column (glass 1, brick 4, concrete 6, rebar 10, heavy armor 35, else infinite); contact under 2.0 m/s deals no damage (stops frame-rate grinding); their calibration: a 12-voxel concrete chunk at 6 m/s (KE ~630) breaks ~2 floor voxels, a 3-voxel chip at 5 m/s (KE ~90) cannot break concrete. Port as Q16 integers, not floats | crates/destruction/src/material.rs (break resistance Q16 at :474) and cluster.rs (cascade at :195) | "crush\|resisted\|penetrat" in crates/destruction -> 0; "min_speed\|MIN_SPEED\|grind" -> 0 | G6 held (golden d57b5c8ae3096614 x2) plus a stacked-slab cascade count | 3 / S | FILE BODY READ (physics.rs lines 1-164; siege.rs 1-142) |
| 4 | valeriyr/ferrets@master crates/ferrets-replay/src/replay.rs (Apache-2.0, 4 stars, pushed 2026-10-17) | Reader keeps every complete tick record and drops only a truncated trailing one; ticks in an ordered map; checksum sampled per tick; the recorder streams and flushes each tick so a crash keeps the prefix | crates/proof/src/tape.rs (238 lines: whole-blob decode with declared counts, errors at :20, :43, :47, :60; no streaming writer) | "fn (write\|append\|push\|flush)" in tape.rs -> 0 | G6 (replay x2 same hash) plus a new torn-tail test | 3 / M | FILE BODY READ (replay.rs full); crate doc in lib.rs read; recorder.rs not read |
| 5 | Anandb71/minecraft-2@main DECISIONS.md D4 and crates/mc2-render/src/calibration.rs (MIT) | Display transform AgX instead of ACES: saturated emitters (lava, torches) path to white without hue skew; a calibration pass (exposure ramp, saturated bars, moving bar) as a golden image | crates/render/src/post.rs and post/gpu.rs (display transform); calibration pattern under crates/render/tests/it | "agx\|AgX" in crates -> 0; "aces" in at least 12 files under crates/render/src | G3 (lit frame look); all goldens re-bless | 3 / M | DECISIONS.md D4 in full; calibration.rs head; AgX WGSL body not located, not read |
| 6 | johannesPettersson80/scena@main docs/headless-rendering.md (MIT or Apache per README, 26 stars, pushed 2026-10-12) | Strict constructor (headless_gpu fails) and prefer constructor (headless_prefer_gpu returns a selection report); each render envelope carries backend_selection {source, requested, selected, fallback_used, reason, remedy}; proof jobs must use the strict one and assert the backend | crates/render/src/adapter.rs (pure choice with software flag, :5-10) and the render envelope in crates/endpoint and playtest proof jobs | "backend_selection\|prefer_gpu\|fallback_used\|remedy" in crates -> 0 | G2, G3 (honest proof), G9 (no silent CPU fallback on the potato tier) | 3 / S | FILE BODY READ (first 5 KB) |
| 7 | balaurengine/balaur@main ARCHITECTURE.md (MIT, 96 stars, pushed 2026-10-17) | Pin libm and scalar-math with the rapier enhanced-determinism features, so a build with physics and one without produce the same transcendental bits | root Cargo.toml (rapier3d enhanced-determinism, cited at Cargo.toml:193 in 02-github.md) and crates/physics/src/rapier.rs | "libm\|scalar-math" in all Cargo.toml -> 0 | G6 cross-build, only if the rapier output feeds a hash | 2 / S | FILE BODY READ (first 7 KB, cut mid-section) |
| 8 | Nilbong/bevy_atmos@main KNOBS.md (Apache-2.0, 0 stars, pushed 2026-10-17) | Every look value is a named knob with a default; a level saves only the knobs it changes as a small text file; unset knob = the game's own look | new knob table beside the render look config (saved per level under content/) | no grep run for a knob registry; decals.rs and material_layer.rs already carry wetness (partly have) | G3 (look reproducible per level) | 2 / M | FILE BODY READ (first 3 KB) |
| 9 | milos-agathon/forge3d@main src/terrain/page_table/scheduler.rs (MIT, 815 stars, pushed 2026-10-16) | Tile requests carry {tile_id, generation}; terminals Complete, Error, Cancelled; bounded (ack-gated) vs coalescing (replace pending) policies; wasm32 build compiles out threads; counters per outcome (dropped_by_policy, canceled, send_fail) | crates/assets/src/handle.rs (generations exist, 54 hits) and cell streaming in crates/world; WEB6 coarse-first load | "generation" in crates/assets -> 54 in handle.rs; coalescing and dropped_by_policy not counted | G10 (web streaming), G9 | 2 / M | FILE BODY READ (first 3.5 KB of scheduler.rs) |
| 10 | gmrdad82/pfx@main crates/live/src/demand.rs (MIT, 1 star, pushed 2026-10-16) | Region from float bounds: clamp, floor left/top, ceil right/bottom; non-finite bounds fall back to the whole screen; a region settles for one TAA cycle with a 4 px margin and 16 depth probes | crates/render/src/present/gpu_present/live.rs and presenter.rs (dirty and demand paths) | "dirty\|demand\|scissor" in crates/render/src -> 25 lines (partly have; non-finite fallback not checked) | G2 (ms per frame at partial redraw) | 2 / M | FILE BODY READ (first 2 KB) plus README |

## Already have

- One integer tick accumulator: crates/core/src/tick.rs:2 says the only accumulator is IntAcc. Balaur's one-accumulator rule is held.
- Saturating float to int, NaN to 0: crates/core/src/checked.rs:46-66 (f32_from_f64, saturating_f64_to_u64). RustingEngine's sim_to_int rule is held.
- Transcendentals in integers: crates/core/src/fixed.rs:181 sqrt, :200 sin_cos, :219 atan2, :339 isqrt.
- Software adapters refused unless E2040_ALLOW_SOFTWARE=1: crates/render/src/adapter.rs:7-8 and :20-21 (the base of row 2 and row 6).
- Pipeline warm-up: crates/render/src/pipeline_warmup.rs (264 lines). GPU cull with indirect args: crates/render/src/gpu_cull.rs and shaders/cull.wgsl.
- Cascade over bonds: crates/destruction/src/cluster.rs:195. Per-material break resistance Q16: crates/destruction/src/material.rs:474 (the base of row 3).
- Wetness: crates/render/src/material_layer.rs:172 (no puddle code found in that file).
- Dev hot patch: crates/game/src/dev_hotpatch.rs.
- Per-tick checkpoints in the tape: crates/proof/src/tape.rs (not streamed; the gap in row 4).

## Traps

- Crush values and the 2.0 m/s floor are floats in the donor. Port them as Q16 integers. destruction is in the hashed path (golden d57b5c8ae3096614): any change must hold that golden x2.
- WebGPU worker: OffscreenCanvas needs Chromium 113+ and Firefox 141+ (donor README). Safari is unverified. Keep the main-thread path as fallback. postMessage input adds latency; measure it before claiming a win.
- WARP goldens prove reproducibility only. Their numbers say nothing about the 3050 or the iGPU potato tier. WARP may lack features our shaders use; test first.
- AgX changes every display color. All goldens and calibration bars re-bless. Keep ACES behind a flag until the look judge signs off.
- A tape torn-tail reader needs a format version bump. Only a truncated trailing record may be dropped; a bad middle record must still fail hard.
- scena's CPU rasterizer is deterministic but slow. Do not make it the default for the arena.
- pfx's 4.4 ms and 34.7 MB figures come from its own machine and build. Not reproduced here.
- Engine pitch claims (bit-exact, 4K in 4 ms, 10,000 cubes at 2,000 FPS) are README claims only. None was run.

## Not read

- Bodies not opened: kooch, telos, Minerust, IgorKonovalov/voxel-engine, myth, ludenea ADR bodies, quench3d physics src, Aurora-Viewer, AstraWeave, noob_tube, rounds-port NOTES.md, bevy_atmos wet and sky shaders, scena src/, forge3d beyond the scheduler head.
- minecraft-2: AgX WGSL body not located; renderer.rs and svgf.rs not read.
- pfx: warm.rs, taa.rs, impostor.rs not read.
- Established names (Fyrox, kiss3d, Stardust XR, Veloren, Ambient) were name-checked by metadata only, no directory read.
- Search quota was not hit. Dust3D and Blue Engine were not found by name; a search for miniquad and Vx gave no hit.
