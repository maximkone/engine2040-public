### Powerups: the wave's finds as board rows (owner 2026-10-21)

Boarded 2026-10-21 from the powerup wave of 2026-10-20 (`docs/research/powerups-2026-10-20/`).
The lead's index over 15 lanes lists 50 finds. This file turns the ones a builder can land into
board rows. Every row is new (`PW01` up), so no live id moves; the row a find closes is named in
the body ("closes LF5"). The old rows stay as they are until the lead corrects their text.

The wave's rules are kept here: the tree is checked before a find is called missing, no donor
bytes are vendored (we port the method and write our own code), free models only, and the golden
`d57b5c8ae3096614` never moves. No row here is ticked and no row claims a test that was not run.
Crate notes are as the lanes read them on 2026-10-20: `crates/render` was held by the GS44 landing
agent, `crates/assets` and `crates/game` by WEB6, `crates/anim` by AM24, `crates/provider` by OPT27,
`crates/observe` by the auto-heal lane and `crates/nav` by the layers lane. A row that needs one of
those crates says so.

Two lanes have no proposed-rows file (08 and 13): PW30 is built from the index row alone and keeps
the mark, and lane 08's finds stay in the index with no row here. Lanes 02, 04 and 14 wrote theirs
late on 2026-10-21 (00:53, 00:55, 00:57) and those files are still uncommitted, so the rows they feed
keep the mark "from a lane still landing - the Accept may change"; the test filters in those rows do
follow the landed files. Lane 15 wrote its file at 00:45, so its four rows (PW20, PW52, PW53, PW54)
carry that lane's own commands and no lane mark.

### The lit frame, five cuts (owner 2026-10-21)

Gap B7 of `docs/plan/finish-line.md` asks for "a depth and normal prepass, GTAO on ambient only,
the post chain (bloom, grade, TAA) as a real pass, particles and decals drawn in the scene, and
cull, sort and instancing in the live draw loop". These five rows are the cuts, in order. The
order is the lane's, not mine: cut 2 is the gate for cuts 3 and 5, and cut 4 needs cut 1.


### The look around the lit frame (owner 2026-10-21)


### Bodies, cloth and the float world (owner 2026-10-21)


### The door and the agent (owner 2026-10-21)

Rows in this section come from lanes 04, 08 and 13, whose proposed-rows files are not written yet.
Each one says so; the Accept may change when the lane lands.


### The anim and the game (owner 2026-10-21)

- [x] PW37 web: the page draws the real cooked fight, not stand-ins (closes A1, WEB16, WEB17 and WEB5) — the browser page must show the real fight: the cooked arena, the parry and block cues and a frame gate that compares the page against the reference capture. Today the page has the shell and the receipt but not the real scene, so gap A1 is open. Donor: our own web lane; the receipt shape is `crates/web/src/report.rs` and the view is `crates/web/src/arena_view.rs`. Diff: regular. Tier A. Size M. Owns `crates/web/src/arena_view.rs`, `crates/web/src/arena.rs`, `crates/web/src/report.rs`, `crates/assets/`. Crate note: `crates/web` was held by the G5 lane and `crates/assets` by WEB6 on 2026-10-20. Landed 6c767d8. The page draws the cooked arena: the receipt names the scene it drew (scene=arena-kit, the match line shows kit=11/11 draws and bodies=2/2) and the fight cues are printed once each (cue: PARRY and cue: BLOCK in one 120 s headless Chrome run on nvidia/ampere). The frame gate compared the page's own capture with the stored reference capture and printed the measured SSIM 0.9992 (bar 0.95, verdict pass, scene=arena-kit); the stand-in ring measures 0.452362, under the bar. cargo test -p engine-web --lib arena green (14 tests; the helper runs that line on the PC because crates/web/src/arena.rs holds GPU code); clippy -p engine-web --all-targets -D warnings green on the runner (run 39565641494); golden d57b5c8ae3096614 green (run 39564307544, and the page prints the same hash in the wasm); the web budget gate green (wasm 4150422 of 4250000). crates/web/src/fight.rs is outside this row's Owns list and took two accessors (the match's cues and its kit draws) and the arena half-extent fix: the page's ?fight died on the game crate's 5.0 m floor, so no fight was drawn at all.
  Accept: `node tools/gh-check.mjs cargo test -p engine-web --lib arena` green: the page shows the cooked arena, a parry and a block each print their cue, and the frame gate compares the page capture with the reference capture and prints the SSIM value it got; the receipt names the scene it drew.
  F2P: the page draws stand-ins and the parry and block cues are missing.
  P2P: golden d57b5c8ae3096614 x2; the web budget gate stays green.
  needs: none.
  Progress 2026-10-21 (the live page, fixtures re-taken): the stored gate fixtures went stale - the live page's own capture measured ssim=0.8601 against the old reference (bar 0.95, verdict fail) because the browser frame moved after that capture (the spot atlas pass is native-only on wasm32 now, and the frame bus is a seqlock). The page is still reproducible: two fresh runs of the live page (headless Edge 155 on nvidia/ampere, the RTX 3050) are byte-identical (capture hash 20fa9d4cdce30a25, frame 31, 1256x708, mean luma 197.50), so the fixtures are re-taken from those runs and the bar stays 0.95 - reference against the page run 1.0000, against the stand-in ring 0.5350 (under the bar, so the gate still tells the cooked arena from the placeholder). Proof: `cargo test -p engine-web --lib` 150 passed, 0 failed (run 39640183565). Re-taken because the old reference was stale, not re-pinned to hide a failure.

### The web, the tools and the workspace (owner 2026-10-21)

- [x] PW43 tools: one crate per agent, a packet under 6,000 tokens (closes OPT29) — an agent's entry set is about 14,703 tokens today (`llms.txt` 856, `AGENTS.md` 1,420, `docs/book/for-ai.md` 1,512, `docs/CRATE-MAP.md` 2,009, `.opencode/repomap.md` 3,966, `docs/plan/README.md` 4,940), and `.opencode/modmap.md` adds 36,154 more if it is read whole (144,615 bytes, 1,397 lines, 42 crate sections). One crate's slice is 300 to 4,068 tokens, so a packet of one crate's map slice plus its `docs/CRATE-MAP.md` row, its plan-index line and its board row fits under 6,000. The row builds the packet. Donor: yamadashy/repomix (MIT, read live 2026-10-20) for the pack shape, Aider-AI/aider `aider/repomap.py` (Apache-2.0, read live 2026-10-20) for the ranked map, and tree-sitter plus tree-sitter-rust (MIT) for the parse; the tool is ours, no new dependency. Diff: regular. Tier B. Size S. Owns `tools/modmap.mjs`, `.opencode/modmap.md`, `docs/INDEX.md`, `llms.txt`. Landed 11b9f969. `node tools/modmap.mjs --check` prints `PASS .opencode/modmap.md`; `node tools/modmap.mjs --pack crates/render` measures the packet at 5,754 tokens (23,014 bytes, budget 6,000) with this row ticked, and all 43 crates fit under the budget (worst is crates/render 5,754, best engine-chaos 220). `node --test tools/modmap.test.mjs` 15 pass, `node --test tools/for-ai.test.mjs` 6 pass, `node tools/workspace-check.mjs` PASS (maps: 43 crates, 1,431 paths match the tree), `node tools/rust-quality.mjs` PASS (score 97 (A), 0 ratchet regressions over 44 crates); golden d57b5c8ae3096614 x2 on the runner: `node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli default_and_max_seed` run 39683813396 and run 39684146089, 1 passed each. The packet carries the map slice, the `docs/CRATE-MAP.md` row, the `docs/plan/README.md` plan line and the open board rows, and it measures itself, so the token number in the packet is the number the tool computed.
  Accept: `node tools/modmap.mjs --check` prints `PASS .opencode/modmap.md`, and a packet for `crates/render` (map slice, CRATE-MAP row, plan line, board row) measures under 6,000 tokens; `node --test tools/for-ai.test.mjs` passes.
  F2P: an agent reads 36,154 tokens of modmap to work on one crate.
  P2P: `node tools/workspace-check.mjs` PASS; golden d57b5c8ae3096614 x2.
  needs: none.
- [x] PW47 assets: content by fetch, the AssetSource seam and the LoadQueue (closes A2 and WEB6) — the content is cooked from disk only, so the web bundle and the game load the same way and a big world cannot stream. The row adds an `AssetSource` seam (disk, memory, or a fetched range), a `LoadQueue` with a priority and a byte budget, and a generational handle with an unload, so the page and the game ask for the same content by name. Donor: our own assets lane; the shape is the existing `crates/assets/src/cooked.rs` handle and `crates/assets/src/ktx2_cook.rs` output; no donor code. Diff: regular. Tier A. Size M. Owns `crates/assets/src/source.rs`, `crates/assets/src/load_queue.rs`, `crates/assets/src/cooked.rs`, `crates/game/src/`. Crate note: `crates/assets` and `crates/game` are held by the WEB6 landing agent. Landed f8eb3c9eb (the commit carries the WEB6 label; PW47 is the owning row). WEB6 in docs/SPRINT-22-web-realism.md keeps its own row and its own remaining list, so it stays open there; PW47 delivers the seam and the queue it was waiting on. The seam landed as `crates/assets/src/asset_source.rs` with the fetched-range source in `crates/assets/src/asset_source/fetch.rs`, not as `crates/assets/src/source.rs` in the Owns list; the LoadQueue is `crates/assets/src/load_queue.rs`, the generational handle is `crates/assets/src/handle.rs` and the game consumer is `crates/game/src/kit_assets.rs` with `crates/game/src/arena_kit.rs`. Accept green: `node tools/gh-check.mjs cargo test -p engine-assets --lib load_queue` 10 passed / 0 failed (run 39596950970), with the four named behaviours in it: a request over the byte budget is refused by name (`queue_refuses_a_request_over_the_byte_budget`), the high-priority name loads first (`queue_reads_the_highest_priority_first`), an unload frees the bytes and the stale handle is refused (`queue_unload_gives_the_bytes_back_and_stales_the_handle`), and two loads of the same name give the same bytes (`queue_keeps_one_handle_per_path`). P2P green: golden d57b5c8ae3096614 twice on a clean HEAD (runs 39598950063 and 39599440658; the live page prints the same hash) and the cook tests stay green (29 passed / 0 failed, run 39598332625). Progress 2026-10-22 (the rig cook): the CT7 fighter rig cooks into one bundle - `fighter_bundle` (crates/assets/src/fighter_bundle.rs plus the `fighter_bundle` bin) packs the rig tree (the manifest, the four bodies, the baked clips) into one `E2BUNDLE` in three tiers: tier 0 the manifest the bind reads, tier 1 the clips and the hero body (the first rig a page can draw), tier 2 the other bodies. The bytes of the tiers `0..=p` are one prefix of the file, so the page opens the first rig with the prefix open and the rest later. The cook refuses a rig without the manifest, the hero body or the clips, by name; the same files give the same bytes; the bin reads the written file back through `BundleSource` and every rig file comes back byte for byte. The receipt on the real content: entries=19 bytes=7895521 tiers=0:24643 1:2923236 2:4946412 first_prefix=2949109 bar=4000000 ok. Proof: `cargo test -p engine-assets --lib fighter_bundle` 5 passed (run 39664985931); `cargo test -p engine-assets --features import --test it fighter_rig` 4 passed (run 39664975796); `cargo clippy -p engine-assets --all-targets -- -D warnings` passed (run 39664962318); `cargo test -p engine-assets --lib` 226 passed on the PC. Not in git: `content/fighters/fighters.e2bundle` (7.9 MB raw, 6.1 MB brotli) - the page's first-fetch budget is 2040000 brotli bytes and the arena bundle alone is 735831, so shipping the rig as one file does not fit that budget; the rig must be cooked smaller (the compact glTF lane) or fetched in tiers after the first frame. Left: the browser load of that bundle, the fight rig on the page, a refreshed `dist/web`. The rig loads out of a fetched bundle with the native bytes: the game crate's `FighterLook::load_from` reads the manifest, the bodies and the clips out of a cooked `.e2bundle` and gives the same rig summaries as the native directory load (run 39698918656, `cargo test -p engine-game --features app --lib fighter_look`, green). The bundle proof compares the same file set: every bundle entry is compared byte for byte against the directory file, never the whole tree (the tree also holds the atlas PNGs and the previews, so a whole-tree digest could never equal the bundle digest). Measured 2026-10-22 (the rig's size on the page): the cooked rig does not fit the page's budgets as it stands. Brotli, per file: the manifest 3,322 B, the hero body 1,809,331 B, the rival bodies 1,364,207 + 967,275 + 1,908,936 B, the fourteen clips 186,550 B; the whole rig is 6,239,621 B brotli and 7,894,291 B raw. The site's `first_bundle` budget is 2,040,000 B and the page already spends 1,621,894 B of it, so 418,106 B are left: the hero body alone is 4.3x the room, and the whole rig is 3.1x the whole budget. Memory is the second wall: the hero body's four atlas quadrants are 1024x1024 PNGs that decode to 16,777,216 B of RGBA (4,194,304 B each), and the page's `mem` budget is 19,988,480 B, so one body's textures alone are 84 % of the page's whole linear memory, before the mesh, the clips and the wasm module. The lever is the same one the arena kit already uses (`crates/assets/src/ktx2_cook.rs`, `crates/game/src/kit_pbr.rs`): a cooked lane with KTX2 maps and a compact glTF, read through `decode_level_rgba` instead of `TextureUpload::from_encoded`. That is new work in the assets crate (a rig cook that keeps the skin) and it changes the body bytes, so the CT7 body pins (`EXPECTED_BODY_SHA256` in `crates/game/src/fighter_bind.rs`) and the manifest move with it. Until that lands, the rig can be fetched and read in the browser but not shipped on the site. The rig bundle stays out of git for the same reason: 7,894,291 B of binary in the tree is not a reviewable diff.
  Accept: `node tools/gh-check.mjs cargo test -p engine-assets --lib load_queue` green: a request over the byte budget is refused by name, a priority queue loads the high-priority name first, an unload frees the handle and a stale handle is refused, and two loads of the same name give the same bytes.
  F2P: every asset is read from disk at load and there is no queue and no byte budget.
  P2P: golden d57b5c8ae3096614 x2; the cook tests stay green.
  needs: none.

### The dead code, the layers and the world (owner 2026-10-21)

- [x] PW49 crates: the seven dead feature flags come out (closes AD5) — seven crate features are asked for by no other crate and read by no `cfg(feature = ...)` in their own crate: `endpoint/tls`, `geo/default`, `geo/kit`, `geo/all`, `kam2-game/window`, `playtest/tls` and `scene/math`. Seven more are weak (one `cfg` use, no external request): `core/test-util`, `geo/bake`, `geo/fbx`, `proof/arena`, `proof/spine`, `proof/ragdoll`, `rules/arena`. The row removes the seven dead flags and records the seven weak ones in the crate map. Donor: cargo-unused-features 0.2.0 (https://crates.io/crates/cargo-unused-features, source https://github.com/TimonPost/cargo-unused-features, path `main/LICENSE`), licence MIT (read live 2026-10-20); our own scan at HEAD `77789c2a` produced the same 7 dead and 7 weak names. Diff: good. Tier B. Size S. Owns `crates/endpoint/Cargo.toml`, `crates/geo/Cargo.toml`, `crates/kam2-game/Cargo.toml`, `crates/playtest/Cargo.toml`, `crates/scene/Cargo.toml`, `docs/CRATE-MAP.md`. Landed 8c923d0. The scan is `tools/feature-scan.mjs` (plus `tools/feature-scan.test.mjs`): it reads the manifests and the Rust files, and a feature is dead when no other crate asks for it and no `cfg(feature = ...)` in its own crate reaches it, and weak when exactly one place in its own `src/` reads it. `node tools/feature-scan.mjs` prints `dead features: 0` and `weak features: 6`; `node tools/crate-map.mjs --check` prints `PASS docs/CRATE-MAP.md`. Delta versus the row's list of seven, re-derived on current master: `scene/math` was already gone upstream (718684d11) and `endpoint/tls`, `geo/default`, `playtest/tls` came out here, plus `geo/kit` and `geo/all`; `kam2-game/window` stays because the crate's own `kam2play` target reads it (`required-features = ["window"]`), and the scan counts a `required-features` line as a read in its own crate, so that flag is alive. The geo kit test moved to a dev-dependency on `engine-assets`, so all ten kit tests keep running, and the `all` umbrella went with the single tools it named. Proofs: `cargo check -p engine-geo --all-targets` PASS run 39698918656; `cargo check -p engine-endpoint --all-targets` PASS run 39699276613; `cargo check -p engine-playtest --all-targets` PASS run 39699171499; `cargo check -p engine-scene --all-targets` PASS run 39699424886; `cargo check -p kam2-game --all-targets` PASS run 39699508578; `cargo clippy -p engine-geo --all-targets -- -D warnings` PASS run 39702621579; `cargo test -p engine-geo --features boolean,uv,lod,collision,bake,fbx --test kit --no-fail-fast` 10 passed 0 failed run 39702842099; `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` PASS runs 39699934679, 39700170892, 39702147460 (the test asserts `final_hash=d57b5c8ae3096614` with `golden_checked` and `verified` true, so the golden holds); `node --test tools/feature-scan.test.mjs` 3 pass 0 fail; `node --test tools/crate-map.test.mjs` 3 pass 0 fail; `node tools/workspace-check.mjs` PASS; `node tools/rust-quality.mjs` PASS score 97 (A) 0 ratchet regressions over 44 crates; `rustfmt --edition 2021 --check crates/geo/tests/kit.rs` clean.
  Accept: a feature scan over `crates/*/Cargo.toml` prints `dead features: 0` (today it prints `dead features: 7`), and `node tools/crate-map.mjs --check` prints `PASS docs/CRATE-MAP.md`.
  F2P: the scan prints 7 dead features.
  P2P: `cargo run --locked -- --verify` prints `final_hash=d57b5c8ae3096614` twice; `node tools/workspace-check.mjs` PASS.
  needs: none.

### Parked, read later (owner 2026-10-21)

Research-only finds. Read them when a row needs them; none of them is a row today.

- Radiance cache for light with no bake, and the SVGF denoiser - https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK (`Kits/FidelityFX/radiancecache/include/ffx_radiancecache.h`) and https://github.com/NVIDIAGameWorks/Falcor (`Source/RenderPasses/SVGFPass/`), MIT and BSD-3-Clause - parked: RT4 is a LATER row and RT8 is frozen by owner decision D7, so there is no bar to land against.
- Hair and strand shading with LTC - https://github.com/selfshadow/ltc_code, BSD-3-Clause - parked: VX3 is Tier C and the proof game has no hair cards.
- Volumetric clouds - https://github.com/AmanSachan1/Meteoros (`src/CloudScapes/shaders/cloudRayMarch.comp`), MIT - parked: VX2 needs GX7, GX8, RL4 and RL6 first.
- Frame generation - https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK (`Kits/FidelityFX/framegeneration/fsr3/`), MIT - parked: there is no free optical-flow provider in the tree, so the row dies on arrival.
- Probe-grid GI with ray splitting - https://github.com/entropylost/chandra and https://github.com/wroughtinator/split-radiance-cascades-webgpu, NO LICENSE in either repo (read 2026-10-20), paper arXiv 2607.20384 (free to read) - parked: no grant, ideas only.
- A unit-variance noise field - https://github.com/facebookresearch/Tabula-Rasa, CC-BY-NC-4.0 - parked: non-commercial and copyleft, so we vendor zero bytes.
- JPEG as a texture format - https://github.com/elias1518693/jpeg_textures, MIT but with a CC BY-SA 4.0 island in `src/kernels/utils.cuh` - parked: the copyleft island, and the KTX2 cook already landed.
- Fire and smoke, two shapes - https://github.com/olliep24/firesim (`src/advect_scalars.wgsl` and the rest of the solver) and https://github.com/MadeInShineA/forest-fire-simulation (`forest_fire_simulation_model.md`), no LICENSE file in either repo - parked: no grant, read only.
- A fixed-point rigid body engine in Rust - https://github.com/richardhozak/fppe (`src/lib.rs`, the `I32F32` unit type), MIT OR Apache-2.0 - parked: the donor's own README says the library is incomplete, so it can never be the oracle.
- Safe Rust bindings to Jolt - https://github.com/SecondHalfGames/jolt-rust (`crates/rolt/src/body_interface.rs`), MIT OR Apache-2.0 - parked: RZ10 dies if Rapier holds the bar, and JoltC needs a C++ toolchain on the runner.
- Cloth on the GPU, two shapes - https://github.com/vitalight/Velvet (`Velvet/SpatialHashGPU.cu`), MIT but CUDA only, and https://github.com/nakjun/Cloth-Simulation-WebGPU, no LICENSE file - parked: no web target, no grant, and the CPU verlet holds the 0.5 ms bar.
- The crowd as one tick, and the crowd sharing one animation texture - https://github.com/DigiLogicLabs/dll-mass-units, Apache-2.0 but the host is Unreal Engine 5, and https://github.com/natepac/crowdbake, licence UNKNOWN (no LICENSE file, `package.json` says private) - parked: no grant, and our own `crates/anim/src/lod.rs` and `crowd_scale.rs` stay the only paths.
- A live motion tracker, and a small grounded VLM - https://github.com/google-ai-edge/mediapipe, Apache-2.0, and https://github.com/microsoft/Florence-2-base, MIT - parked: a model download at runtime, and our own relation graph answers the same questions exactly.
- Project memory as typed notes, and an offline locomotion teacher - https://github.com/mem0ai/mem0, Apache-2.0, and https://github.com/google-deepmind/mujoco, Apache-2.0 (leggedrobotics/legged_gym is BSD-3-Clause but needs a closed SDK) - parked: a network, an embedding model or a Python environment the build does not have.
- The costmap as layers - https://github.com/ros-navigation/navigation2 (`nav2_costmap_2d/plugins/obstacle_layer.cpp`), file header BSD-3-Clause but the repo licence is mixed SPDX with LGPL-2.1-or-later - parked: copyleft risk, ideas only.
- The three refused mesh sources - https://github.com/wgsxm/PartCrafter (MIT repo, but one file carries the Tencent Hunyuan Community terms, no EU), https://github.com/Tencent-Hunyuan/Hunyuan3D-2 (Tencent Hunyuan Community licence, no EU, UK or South Korea) and https://github.com/Stability-AI/stable-fast-3d (licence `NOASSERTION`, gated card) - parked: research only, the gate refuses them (PW56).

### Already in our tree - do not re-mine (owner 2026-10-21)

Each line is a correction with the file and line that proves it. Nobody mines these again.

- Motion matching: `crates/anim/src/motion_match.rs:1`, `crates/anim/src/motiondb.rs`, `crates/anim/src/motion_style.rs`, `crates/anim/src/motion_net.rs`.
- Spring bones and secondary motion: `crates/anim/src/springs.rs:1-2`.
- Inertialization and inertial blending: `crates/anim/src/inertial.rs:1`.
- Two-bone IK, FABRIK and foot planting: `crates/anim/src/ik.rs`, `crates/anim/src/fbik.rs:7-9`, `crates/anim/src/feet.rs`.
- The quaternion pack lanes: `crates/anim/src/pack.rs:7` (smallest-three u16) and `:24` (`PACK_VERSION_ROT`).
- The clip edit verbs (trim, retime, cleanup, strip, mirror): `crates/anim/src/edit.rs:77, :111, :141, :197, :217`.
- The offline lip-sync bake and the viseme shapes: `crates/anim/src/lipsync.rs:1-2`, `crates/anim/src/viseme.rs` - only the live curve source is missing (PW35).
- Crowd scaling and the LOD tier table: `crates/anim/src/crowd_scale.rs:1-8`, `crates/anim/src/lod.rs:1-10`.
- The JSON anim graph verbs: `crates/endpoint/src/packs/anim.rs:34-40` (`anim.graph.set|undo|get`, `anim.preview`).
- The strict WebGPU CI leg: `crates/render/src/backend.rs:437-461`, `.github/workflows/ci.yml:111`.
- The browser netcode: `crates/net/src/browser/wt.rs`, `crates/net/src/browser/ws.rs`.
- Cross-origin isolation: `crates/web/coi-serviceworker.js`, `crates/web/src/shell.rs:76-79`, `crates/web/src/report.rs:116-119`.
- The wasm size tools: `.github/workflows/web-build.yml:65-80`, `tools/web-budget.mjs:137`, `tools/wasm-info.mjs`.
- The quality ladder and the potato gate: `crates/render/src/tier.rs`, `crates/web/src/tier_view.rs`, `tools/potato-check.mjs`.
- The keypoint verifier shape: `crates/playtest/src/keypoint.rs` (`KeypointSpec`, `run_suite`), `crates/playtest/src/keypoint/spec_parse.rs`.
- The look judge and its nine-item rubric: `crates/playtest/src/look/judge.rs:81` (L8 is "SSIM x10000", the full-frame SSIM against the paired Q1 reference frame), `crates/playtest/src/look/metric.rs`, `crates/playtest/src/look/manifest.rs` - the judge is in the tree, the Q1 reference set is not, so only the set and the second metric are open (PW14).
- The auto-exposure CPU half and the 64-bin histogram kernel: `crates/render/src/post/auto_exposure.rs`, `crates/render/src/shaders/auto_exposure_hist.wgsl` - only the dispatch and the EV feed are missing (PW10).
- The retarget cook, the CC0 intake and the provenance rows: `crates/assets/src/retarget.rs`, `content/arena/sources/PROVENANCE.md`, `crates/assets/src/provenance.rs:355`.
- The KTX2 cook: `crates/assets/src/ktx2_cook.rs` (five CC0 ambientCG sets, 15 KTX2 files, 9.79 MB) - only the load-time transcode is missing (PW40).
- The BT load-time validator: `crates/ai/src/bt.rs:449` (`BtTree::decode`, 11 `BtError` variants) - GA11 needs a consumer, not a decoder.
- The rule tooling in workspace-check: `tools/workspace-check.mjs` (the `allowedEdges` map at `:19-102`, the std-only check at `:822`).
- The cloth and the layer table: `crates/physics/src/cloth.rs`, `crates/physics/src/layers.rs` - the wave's own lane built and proved CLX1 to CLX7, so only the open legs become rows (PW16, PW17, PW18).
- The colour metric and the one ACES: `crates/material/src/compare.rs:74` (`ciede2000`), `crates/render/src/lib.rs:494` (`aces_one_source_gate`) - the open half is the tonemap and the dither gate (PW12).
- `import_repair` is wired: `crates/render/src/v1.rs:256` and `crates/render/src/present/gpu_present/presenter.rs:2197` (the path `present/gpu_present/v1.rs` does not exist).
- `engine-playtest` has a consumer: `crates/endpoint/Cargo.toml:16,32,79`, `crates/endpoint/src/packs/verify.rs:34-37`.
- `nav::crowd` does not exist: the file is `crates/nav/src/orca.rs` (971 lines); the live cue type is `crates/audio/src/crowd.rs:512`.
- `ai::utility` is live but internal: `crates/ai/src/gladiator.rs:144,168,179`.
- EG24's `.e2nn` is on disk: `fixtures/nets/depth_student.e2nn` - the row is open only on the arena bind and the gate.
- `content/reference/` does not exist; only `crates/endpoint/src/packs/view.rs:1700` names it, so the Q1 reference set is empty.
- The ORCA production caller is already a board row: TC4 in `docs/SPRINT-27-final-picture.md` (it owns `crates/nav/src/orca.rs`), so the wave's ORCA find is not a new row.
- The part-set geometry: `crates/geo/src/parts.rs` - landed by the wave, only the intake half is open (PW57).
- The centre runner and the licence gate: `C:\empire\center\make.mjs` - landed by the wave, only the engine door half is open (PW56).
- The provider half of the arbiter: `crates/provider/src/pressure.rs`, `crates/provider/src/arbiter.rs` (the `VramHeadroom` seam) - landed 2026-10-21, only the caller is open (PW28).
- The wave's own tools: `tools/unwired-code-gate.mjs`, `tools/compile-cache-spike.mjs`, `tools/dep-rebuild-count.mjs`, `tools/nextest-spike.mjs`, and the workspace double-scan in `tools/workspace-check.mjs`.
- The draw count in the web receipt: `crates/web/src/arena.rs` and `crates/web/src/arena_view.rs` already carry `receipt.draws` and `last_cpu_ms` - PW39 adds the window over them.
- The one ACES gate and the colour metric pair: `crates/render/src/shaders/tonemap.wgsl:2` (`fn aces_fitted`) and `:125` (`fn dither4`); the grade lives in `crates/render/src/shaders/blit.wgsl:118` (`fn blit_grade`), not in `present/look.rs`.

### Order: the ten rows that come first (owner 2026-10-21)

1. PW01 - the velocity target: one target unlocks TAA and the motion blur after it, and its need LF3 is already ticked.
2. PW02 - the bins and the transform: the live loop still draws one item at a time, and RL41 and RL35 both wait on it.
3. PW03 - the depth and normal prepass: GTAO still derives a normal from depth and the AO still touches the sun.
4. PW04 - TAA as one pass: it needs the velocity target from PW01, and three TAA copies sit in the tree with no live resolve.
5. PW05 - the GPU cull and the indirect draw: it rides on PW02 and carries the DX12 `first_instance` risk, so it lands last of the lit frame.
6. PW10 - the auto-exposure histogram: GS44 has the CPU half and the kernel, and the night arena clips without the dispatch and the EV feed.
7. PW06 - the sun shadow: the arena map is redrawn into a 256 map every frame, and the row closes RL35 and QA4 together.
8. PW07 - the pass report: a pass can be skipped and the frame looks merely wrong, and the landed GS45 has no caller.
9. PW22 - the verifier in every build verb: D19 is the gate most of the agent rows wait on, and the rule is "no check is not a pass".
10. PW30 - the heals that prove themselves: one row closes AD14, E75, A9 and G1, and today a heal that does nothing is recorded as applied.

### The duel brain (owner decision 2026-10-21, lead D1)

