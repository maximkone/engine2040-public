# Book gaps 3: engine2040 vs the books (2026-10-17)

Method: I read the crate map, finish-line, status-2026-10-17, web-and-realism, AI-NATIVE (skim), ARCHITECTURE (skim) and the two earlier gap reports. Then I walked the book subsystems (geaen / gearus chapters 5-16, found2 ch 5-10, rustwasm, testing, dushkin) and grepped `crates/` for each concept. Every status below cites a file I opened or a grep with 0 or N hits. "Not re-checked" means I did not look today.

Book note: `books.mjs toc` prints only the first 200 headings, so I used chapter names from the TOC plus `search` hits for quotes. Quotes are short. Russian quotes are from dushkin.

## 1. Verdict in ten lines

1. The engine is a deep library set. The shipped game is still one arena duel.
2. Since 2026-10-12 a lot closed: sun shadow is sampled, MSAA targets exist, GTAO runs on the device, the HP overlay draws, kit walls reach the match, combat tables V2 and poise are wired, release has overflow checks, a free camera exists, physfx (ragdoll, debris, blast) exists, sphere cast and ray cast are door verbs, there is an AssetSource seam and a browser shell.
3. Still open from the old reports: frustum cull and draw sort in the live loop, post chain (bloom, TAA) as a real pass, nav in the arena, job pool in the sim, loader thread, in-game debug draw / console / profiler, reference captures, MCP resources and prompts.
4. The web build today draws a procedural arena with 8 skinned fighters. It does not run the arena sim, load the cooked kit, or join a match (`crates/web/src/arena.rs:9`, `crates/web/Cargo.toml` has no `engine-arena` or `engine-game`).
5. Physics "hard stuff" is now two layers (authority integers + physfx floats). The window hooks (RZ2 ragdoll, RZ3 limbs, RZ4 break draw) are the open part. The door has no verb that spawns a ragdoll or a limb.
6. There is no game-logic layer a non-programmer or an AI can fill with data: no event bus, no trigger volumes, no objective graph. The Rhai `rules` crate is only reachable from the MCP door (`crates/endpoint/Cargo.toml:27,48`); `engine-game` does not depend on it.
7. The AI door is strong (15 packs, undo, budgets, receipts, web verbs). It lacks MCP resources and prompts, progress for long calls, and a visual score the AI can trust (the look judge was removed; zero reference PNGs).
8. Agent patterns from dushkin (compensating action, idempotent action, budget-aware, trace) are mostly HAVE in the door. Shared memory across agent sessions and a "task recipe" layer are not.
9. Multithreading is the biggest silent hole: `JobPool` exists, has no sim caller, and `fork.rs:314` still spawns one OS thread per branch.
10. Status docs disagree with code in places (finish-line G11 says "not started" while `physfx`, `break_look.rs`, `ragdoll_look.rs` exist). Trust the code lines below.

## 2. Re-check of the earlier gaps

Legend: CLOSED, STILL OPEN, WORSE, PARTIAL (moved but not done), N/R (not re-checked).

### 2.1 BOOK-GAPS-2026-10-12 (W rows and section 4)

| Row | Then | Today | Evidence |
|---|---|---|---|
| W1 sun depth never sampled | open | CLOSED (4-tap compare). Fixed 4 taps, no cascade overlap check done | `crates/render/src/shaders/forward.wgsl:173-176` `textureSampleCompare`; `present/gpu_present/submit.rs:667` |
| W2 GTAO thrown away | open | PARTIAL. `encode_gtao` writes `ao_tex` on the device, but the CPU feed is empty `&[]` and forward uses `sun_contact_factor`, not AO | `present/gpu_present/live.rs:1245-1262`, `forward.wgsl:836` |
| W3 bloom / post chain no caller | open | STILL OPEN. A one-float flag lifts a halo in blit; `encode_post` has 0 hits in `render/src/present`; `post.rs run_frame` has no live caller | `live.rs:1282-1296`; grep `encode_post` 0 hits |
| W4 TAA/SMAA, MSAA | open | MSAA CLOSED (targets + resolve). TAA STILL OPEN in the live frame | `submit.rs:592-620`, `present/aa.rs:205` only |
| W5 particles, decals only probes | open | PARTIAL. Particle pool steps and submits every frame; a flipbook draw builder exists; pass-truth tests still pin decals as PROBE | `submit.rs:306-310`, `render/src/particle_draw_gpu.rs`, `pass_truth.rs:823-824` |
| W6 captures without sky | open | CLOSED for the live path | `live.rs:1236` `ForwardLook::live(.., sky_draw ..)`, `submit.rs:838` |
| W7 frustum cull and LOD no caller | open | STILL OPEN | `eg38_frustum_planes` only at `submit.rs:1635` and a test `:2275`; `game/src` has 0 calls to `cull_lod` / `frustum_planes` |
| W8 CSM texel snap | open | N/R | |
| W9 torch lights | open | CLOSED (8-slot uniform landed) | `docs/plan/render-look.md:235` cites `forward.wgsl:45` |
| W10 P2P rollback no caller | open | STILL OPEN (decision D2 not taken) | `P2pSession` 0 hits in `crates/game/src` |
| W11 stream queue / LOD swap | open | PARTIAL. `LodSwapper` is live in the window dressing stage; no loader thread; kit read now goes through an AssetSource seam | `game/src/windowed_dressing.rs:11,326`; `assets/src/stream.rs` has no `spawn` |
| W12 nav not in arena | open | WORSE. `crowd.rs` and `neighbors.rs` were deleted as zero-consumer; `crates/arena/Cargo.toml` has no `engine-nav` | `docs/plan/status-2026-10-17.md:36`, grep 0 hits |
| W13 projectile CCD | open | N/A, file gone | `physics/src/lod.rs` deleted (3d6ee4a1) |
| W14 shipped app has no Rapier bodies | open | CLOSED by design. Two layers: authority stays kinematic; physfx paints on top | `game/Cargo.toml:14-21`, `game/src/ragdoll_look.rs:33`, `break_look.rs:22` |
| W15 frame arenas | open | N/R (`core/src/arena.rs`, `ecs/src/arena.rs`, `render/src/frame_alloc.rs` exist) | |
| W16 profiler no HUD | open | STILL OPEN | grep `puffin|Profiler` in `game/src` 0 hits |
| W17 gate tools unwired | open | N/R | |
| 4.1 Asset handles with generation | MISSING | PARTIAL. Built, no caller | `assets/src/handle.rs:3` "planned, no caller yet" |
| 4.1 Loader thread, async IO | MISSING | STILL OPEN | `stream.rs` no thread |
| 4.1 Cell streaming drives the game | PARTIAL | STILL OPEN. `modes_world.rs` imports only `CellCoord`, `MAX_COORD` | `game/src/modes_world.rs:29` |
| 4.1 SIMD | PARTIAL | STILL OPEN | grep `std::arch|simd128|wide::` in `anim/src` 0 hits |
| 4.1 One config file | MISSING | STILL OPEN. `core/src/config.rs` is sim config only | `core/src/config.rs:1` |
| 4.1 Memory by subsystem | PARTIAL | STILL OPEN. Counting `#[global_allocator]` lives in test builds | `ai/src/lib.rs:13`, `game/src/lib.rs:1760` |
| 4.2 Multicore sim | PARTIAL | STILL OPEN. Pool callers are bake/verify only | `ecs/src/jobs.rs:1`; no `JobPool` in `arena` or `game` |
| 4.2 Debug draw, console, pause/step | MISSING | STILL OPEN. `debug_draw.rs` has tests only; no console | `render/src/debug_draw.rs` (only test hits), grep `console` in game only finds log text |
| 4.2 Free debug camera | MISSING | CLOSED | `game/src/camera.rs:372 free_cam_step`, `bin/arena.rs:37` |
| 4.2 Input feel: rumble, disconnect | MISSING | STILL OPEN | grep `rumble` 0 hits |
| 4.3 Attach points | MISSING | STILL OPEN | grep `attach|socket` in `anim/src` finds none that matter |
| 4.3 Rotation keys in packed clips | PARTIAL | STILL OPEN | grep `quat|rotation` in `anim/src/pack.rs` 0 hits |
| 4.4 Shape casts | MISSING | CLOSED (sphere cast + door verbs) | `physics/src/query.rs:318`, `endpoint/src/packs/physics.rs:411,416` |
| 4.4 Collision layers, materials | MISSING | CLOSED | `physics/src/layers.rs`, `physics/src/material.rs`, RZ8 landed 3b01824 |
| 4.4 Joints beyond ragdoll | MISSING | STILL OPEN | |
| 4.5 Event bus | PARTIAL | STILL OPEN. `observe/event_queue.rs` has no game caller | grep `event_queue` in `game/src` 0 hits |
| 4.5 Script waits, regions, objectives | MISSING | STILL OPEN | `rules` is not a game dependency |
| 4.5 Audio | PARTIAL | CLOSED for the window path (cue bank, mixer, spatial); Doppler N/R | `game/src/windowed.rs:18,831` |
| 4.6 Visual reference set | MISSING | STILL OPEN (zero PNGs; Q1 open) | `docs/plan/quality.md:26,55`; `tests/` has no png |
| 4.6 Property tests | MISSING | STILL OPEN | `proptest` 0 hits in root `Cargo.toml` |

### 2.2 BOOK-GAPS-2-2026-10-12 (N rows)

| Row | Today | Evidence |
|---|---|---|
| N1 job pool no sim caller | STILL OPEN | see 4.2 above |
| N2 fork thread per branch | STILL OPEN | `arena/src/fork.rs:314 std::thread::spawn`, `gym.rs:494 std::thread::scope` |
| N3 tick graph not drawn | N/R | |
| N4 no loom / TSan | STILL OPEN | no `loom` in root `Cargo.toml` |
| N5 HUD overlay never drawn | CLOSED | `game/src/windowed.rs:971 duel_overlay_for_app`, `live.rs:1312 hp_bars_plan` |
| N6 no sort/instancing in the live loop | STILL OPEN (N/R in detail; no grouped draw in `live.rs` / `submit.rs`) | |
| N7 baked GI never sampled | N/R | |
| N8 GpuSkinned no caller | CLOSED | `game/src/fighter_look.rs:1265`; web page says 8 GPU-skinned fighters |
| N9 cull bound must follow posed body | STILL OPEN (needs W7 first) | |
| N10 siege stepper | N/R | |
| N11 float gate skips arena | N/R | |
| N12-N15 input buffer, windup V2, commit lock, poise | PARTIAL/CLOSED for V2 and poise: `arena/src/lib.rs:1205` picks `TABLES_V2`, `:1866` `poise()`. Input buffer and dodge commit lock N/R | |
| N16 crowd neighbour scan | WORSE (crowd code deleted) | status-2026-10-17.md:36 |
| N19 kit walls never reach the match | CLOSED | `game/src/arena_kit.rs:301 MatchWorld::try_new` |
| N31 free-fly cam | CLOSED | camera.rs:372 |
| N32 release overflow checks | CLOSED | `Cargo.toml:229 overflow-checks = true` |
| N36 MCP resources and prompts | STILL OPEN | grep `resources/list|prompts/list` in `endpoint/src` 0 hits |
| N37, N38 fenced fetched text, progress | N/R | |

## 3. Subsystem scorecard

HAVE = wired in the played path (window game or door). PARTIAL = code exists but not wired, test-only, or one leg only. MISSING = nothing.

| Subsystem (book) | Score | Evidence and gap |
|---|---|---|
| Game loop, fixed step, anti-spiral (geaen 7.5; gearus 8.2-8.5) | HAVE native; MISSING on web | `core/src/time.rs:50-104` cap + banked steps; web page draws a picture, no fixed-step sim (`web/src/arena.rs`) |
| Start-up / shut-down (geaen 5.1) | PARTIAL | `app` crate; failure unwind N/R |
| Memory / allocators (5.2, 9.9) | PARTIAL | counting allocator tests only; no per-subsystem heap line in the shipped game |
| Containers, strings, ids (5.3-5.4) | HAVE (Rust std; `scene/src/ids.rs`; assets key = hash) | generational `AssetHandle` built but unused |
| Engine config (5.5) | PARTIAL | typed sim config only; `env::var` still scattered |
| Resource manager (6.2) | PARTIAL | `AssetSource` seam + `.e2bundle` landed (WEB6 bc6838a); registry; handle store unused; no unload owner |
| File system, async I/O (6.1, 7.1.3) | MISSING | no loader thread; web has no fetch loop yet |
| Hot reload (6.2.2) | PARTIAL | `game/src/reload.rs` + `bin/arena.rs:1386 poll_hot_reload`; live asset swap `live.rs` (1,389 lines) was deleted; `material/src/reload.rs` exists |
| Debug tools: log, crash (9.1) | PARTIAL | `game/src/crash.rs` tape tail |
| Debug draw, console, cheats, pause/step (9.2-9.6) | MISSING in the game | `render/src/debug_draw.rs` unused; free cam only |
| In-game profiler (9.8) | MISSING | puffin opt-in in perf only |
| HID / input (8) | PARTIAL | action map, bindings, gilrs pad, web keyboard/mouse/Gamepad (`web/src/input.rs`); no rumble, no disconnect rule |
| Render: forward PBR, GPU skin | HAVE | `forward.wgsl`, `fighter_look.rs:1265` |
| Render: sky, IBL, sun shadow, MSAA | HAVE | `live.rs:1236`, `forward.wgsl:173` |
| Render: GTAO | PARTIAL | `live.rs:1245`; feed empty; forward does not read it |
| Render: bloom, grade, TAA, SMAA | PARTIAL/MISSING | flag only; `post.rs run_frame` unused in the live frame |
| Render: particles, decals | PARTIAL | `submit.rs:306`; pass-truth PROBE |
| Render: culling, sort, instancing (found2 9.4; geaen 11.2.7) | MISSING in the live loop | `draw.rs` helpers, no caller |
| Render: LOD | PARTIAL | `LodSwapper` live in dressing; `lod_fade.rs` |
| Render: materials from data | HAVE on the door | `crates/material`, `packs/material.rs` (7 verbs) |
| Render: texture compression, mips, KTX2 | PARTIAL | `assets/src/bc.rs`, `texture_cook.rs`; VR2 KTX2 cook not done |
| Render: web (WebGPU) | PARTIAL | async device init, loss recovery, limits gate (VR3), shell prints receipt; no web tier, no pipeline warm-up |
| Animation: blend, state machine, IK, foot lock, inertialization | HAVE | `anim/src/*`, `fighter_look.rs:25,918` |
| Animation: rotation keys, compression, attachments | MISSING/PARTIAL | `pack.rs` no rotation lanes; no sockets |
| Animation: retarget | PARTIAL | `assets/src/retarget.rs` |
| Collision (geaen 12): queries | HAVE | `physics/src/query.rs`, door ray and sphere casts |
| Rigid bodies (12) | PARTIAL by design | authority is kinematic + integer; floats only in physfx (cosmetic) |
| Character controller | HAVE | `physics/src/controller2.rs`, `arena/src/controller.rs`; one test red (`loc2_wired_slope_step_air_course_x2`) |
| Ragdoll, limbs, debris | PARTIAL | physfx + `ragdoll_look.rs`, `break_look.rs`; window draw hooks and limb part-swap left (RZ2-RZ4) |
| Destruction | HAVE authority; PARTIAL picture | `destruction` crate; `break_look.rs` headless landing |
| Audio (13) | HAVE | cue bank, mixer, HRTF, occlusion in the window; web uses a ScriptProcessorNode chime only (`web/src/browser.rs:43`) |
| Gameplay: object model | PARTIAL | `scene` ids, schema, prefab; `ecs/spine.rs`; legacy world fork |
| Gameplay: events/messaging | MISSING | typed events per crate; no bus |
| Gameplay: scripting (15.8) | PARTIAL | `rules` Rhai sandbox on the door only |
| Gameplay: triggers, regions, objectives | MISSING | grep found none |
| World loading and streaming (15.4) | PARTIAL | `world/src/cells.rs` bake; `modes_world.rs` imports only coords |
| Save | HAVE | `crates/save`, `game/src/lib.rs:147` |
| AI: BT, utility, EQS, perception | HAVE | `crates/ai` |
| AI: nav / crowd | WORSE | crowd cut; flow still exists in `nav` with no arena caller |
| Multiplayer: server-auth, prediction, snapshot, heal | HAVE (loopback proof only) | `net/src/session.rs`, MP6 loopback |
| Multiplayer: P2P rollback | PARTIAL | `net/src/rollback.rs` no game caller |
| Multiplayer: web transports | PARTIAL | `net/src/webtransport.rs`, `websocket.rs`, `connect_race.rs`, `browser.rs` exist; no wasm client in `crates/web` |
| Multithreading / jobs (geaen 7.6.4) | PARTIAL | `ecs/src/pool.rs` bake only; threads per fork branch |
| Tools / editor | PARTIAL | `crates/editor` is headless-first shell; `tuning.rs` stage+replay |
| Testing: golden hash, matrix, break corpus | HAVE | `proof`, `golden-matrix.yml` (3 native + wasm tapes) |
| Testing: golden images, look score | MISSING | zero reference PNGs; look judge removed (finish-line G4 note) |
| Testing: property, loom, fuzz | PARTIAL | `proof/src/fuzz.rs`, `tape_fuzz.rs`; no proptest, no loom |
| Testing: bots, charters, playtest | HAVE | `crates/playtest` (`charter.rs`, `bot_harness.rs`, `tournament.rs`, `exploit.rs`) |
| Web: wasm threads | MISSING (LATER) | WEB14 |
| Web: asset loading | PARTIAL | AssetSource + bundle format; fetch loop not done |
| Web: audio worklet | MISSING | deprecated ScriptProcessorNode used |
| Web: WebGPU limits | HAVE | `render/src/webgpu_limits.rs`, VR3 landed 927626a |
| Web: headless proof | HAVE | `tools/web-browse.mjs`, `web-run.mjs`, `web-budget.mjs`, `web-build.yml`, `pages.yml` |
| Agents: tool use, adapter | HAVE | 15 packs, `api.call/api.load`, `packs/mod.rs:67` |
| Agents: compensating / undo | HAVE | undo on scene, anim, web verbs |
| Agents: idempotent / revisions | HAVE | scene revisions, receipts |
| Agents: budget-aware | HAVE | tool-list budget `mcp.rs`, provider `policy.rs`, `budgets.rs` |
| Agents: tracing / audit | HAVE | `receipts.rs`, `watch_ledger.rs`, `log_tools.rs` |
| Agents: planner/executor, reflection | PARTIAL | `plan_goals.rs`, `assistant.rs`, `agent_loop.rs`; critic/reflect loop not verified |
| Agents: memory across sessions | MISSING | no persistent agent memory or episodic store the door serves |
| Agents: MCP resources, prompts, progress | MISSING | grep 0 hits |

## 4. The finish line: top 20, ranked by impact over effort

Impact tags: W = Windows+web engine, A = AI-native core, E = hard stuff easy. Size: S under 1 week of agent work, M one to three, L more.

### 1. Run the real match in the browser (W, E) — L, crates `web`, `game`, `arena`
- Book: rustwasm "we can just treat the browser as a platform library that we call functions on"; geaen 7.5 / gearus 8.2 game loop; geaen 7.6.
- Have: native `bin/arena.rs` (3341 lines) owns the winit handler. `crates/web/src/arena.rs` draws `ProceduralArena` from `requestAnimationFrame`. The tape runs in wasm (golden check) but the page never steps the match.
- Build: move the app handler out of `bin/arena.rs` into a lib module both entry points share. On wasm drive `engine-arena` with a fixed-step accumulator from `performance.now()` (the clock seam exists: `engine-clock`). Feed web input records (`web/src/input.rs`) into the tape. Draw sim state with the same `fighter_look` the window uses. Print `tick`, `hash`, `tier` in the receipt line.

### 2. Finish the lit frame: AO feed, post chain, particles, decals (W) — L (split in slices), crate `render`
- Book: found2 10.5 Ambient Occlusion, 10.1 Decals, 10.7 Motion blur; geaen 11.3 / 11.4 (gearus 11.3-11.4 "Visual effects").
- Have: GTAO computed to `ao_tex` but the feed is `&[]` (`live.rs:1261-1262`); forward reads `sun_contact_factor` (`forward.wgsl:836`); bloom is a flag (`live.rs:1291`); `post.rs run_frame` has no caller; particles submit (`submit.rs:306`) but pass-truth keeps decals PROBE.
- Build: add a depth+normal prepass, run GTAO on it, multiply ambient only, remove the whole-HDR multiply in blit. Call the bright/down/up/composite chain between forward and blit. Draw particles and decals in-scene. Gate: VR1 table with every pass LIVE on native and web. Owner said "the arena looked meh": this is the visible win.

### 3. Content by fetch, with a loader (W) — M, crates `assets`, `web`, `game`
- Book: geaen 6.2.2.1 "Provides a single unified interface through which a wide variety of resource types can be managed"; geaen 7.1.3 async file I/O.
- Have: `asset_source.rs` trait (disk or `.e2bundle`), bundle format and packer (bc6838a), kit and ring loaders on it. Not done: fighter loaders, headless `import_kit`, fetch loop with partial-prefix open, bundle step in `web-build.mjs`; no loader thread natively.
- Build: finish WEB6 remaining items. Add one `LoadQueue` (priority, byte budget, cancel) used by native (thread) and web (`fetch` futures). Wire generational `AssetHandle` (`assets/src/handle.rs`) as the only way the game holds assets, with unload. Print `loaded`, `late_blob` lines.

### 4. Reference captures and a machine look score (A, W) — M, crates `proof`, `playtest`, `tools`
- Book: testing "Screenshot/video or similar" ("Attaching a screenshot or a video makes the job much easier"); testing "Quick regression".
- Have: `proof/tests/capture_ssim_windows.rs` (SSIM windowed pattern), `pass_truth.rs` (on/off pixel diff), `view.look_judge` door verb. Zero reference PNGs; the look judge and `look_score` modules were removed (`status-2026-10-17.md`, finish-line G4 note).
- Build: pick 10 camera/tick keys, store small reference PNGs plus their SSIM floors per tier (native, web, potato) in `content/reference/`. One verb `view.compare` returns SSIM + FLIP + a diff image. AI can then tune a material and see a number. Needed before any "looks good" claim.

### 5. Cull, sort and instance in the live draw loop (W) — M, crates `render`, `game`
- Book: found2 9.4 Frustum Culling, 9.3 Bounding Volumes; geaen 11.2.7 application stage ("visibility", "sort").
- Have: `draw.rs` has planes, LOD pick, sorted groups; `eg38_frustum_planes` has no live caller (`submit.rs:1635`).
- Build: in `submit.rs` loop call cull, then group by pipeline/material, back-to-front for blended. Bound from posed joints for fighters. Web needs fewer draws, so make it part of the web tier. Print `draws=<n> culled=<n>`.

### 6. A data-driven game layer: events, triggers, objectives, scripts in the played path (E, A) — L, crates `rules`, `arena`, `scene`, `game`
- Book: geaen 15.1 Components of the gameplay foundation; geaen 15.8 Scripting ("permit users to control and customize the behavior of a software application"); geaen 14-16 events/messaging.
- Have: Rhai sandbox + `set/test/explain` on the door (`rules/src/host.rs`, `packs/rules.rs`); typed per-domain events; round flow in `game/src/flow.rs`, `rounds.rs`. No bus, no trigger volumes, no objective graph, no `engine-rules` in `game/Cargo.toml`.
- Build: one `game.rules` file per mode: spawners, regions (box/sphere), triggers (enter/leave/timer/hp), objectives (graph with win/lose), and Rhai hooks. Run it inside the arena step through `Rules::tick` actions only (hash stays honest). Scene docs reference regions by id. This is what makes "a new game mode" one file.

### 7. One-call physical effects: ragdoll, limbs, break, blast (E, A) — M, crates `physfx`, `game`, `endpoint`
- Book: geaen 12.7.3 Ragdolls (gearus 12.7.3 "Тряпичные куклы"), geaen 11.7; geaen 12 collision.
- Have: `physfx` (`spawn_ragdoll`, `spawn_limb`, `spawn_debris`, `apply_blast`, caps, sleeping), `ragdoll_look.rs`, `break_look.rs`. Left: window draw hooks (RZ2 ragdoll render, RZ3 limb part-swap, RZ4 draw), dust/sound cues, cape caller. Door: `break.fracture`, `hit.simulate`, no ragdoll or limb verb.
- Build: finish RZ2-RZ4 draw. Add a `body.toml` (bone shapes, muscle strength, gore level, limb cut table). Add door verbs `fx.ragdoll`, `fx.limb`, `fx.break`, `fx.blast` that take a slot or prop id and print the `physfx:` receipt line plus a capture. One call for the human, one verb for the AI.

### 8. Make `engine2040 new / build / run` real (W, A, E) — M, root bin `engine2040`, `endpoint`
- Book: geaen 1.7 Tools and the asset pipeline; geaen 5.5 engine config.
- Have: AIS1 landing 1 (b63b771): `game.toml`, `new` for `arena-duel` and `siege`, `run --headless`, `build`. `engine-game` is not a root dependency, so the project does not run the real game.
- Build: wire the game into the root bin. `build --target windows|web` shells to cargo and `web-build.mjs`. Add door verbs `project.new`, `project.build`, `project.run` so an AI creates a game from a template and gets the receipt. One `game.toml` carries tier, gore, content roots.

### 9. Debug kit in the game and on the door (A, W) — M, crates `game`, `render`, `perf`, `endpoint`
- Book: geaen 9.2 Debug drawing facilities, 9.4 In-game console, 9.5 Debug cameras and pausing, 9.8 In-game profiling (9.8.1.1 "allow the programmer ... to indicate the hierarchical interrelationships between profiling samples").
- Have: `render/src/debug_draw.rs` (tests only), `free_cam_step`, `puffin` opt-in, `crash.rs` tape tail.
- Build: a cvar registry (name, type, range, default, undo), a console that sets cvars and runs commands, pause / step / time-scale, tick-phase profiler scopes with a per-system line. Mirror each as door verbs (`cvar.set`, `debug.draw`, `profile.frame`) so the AI uses the same tools. Web page shows the same lines.

### 10. Browser multiplayer client (W) — L, crates `net`, `web`
- Book: geaen multiplayer chapter (network architecture, replication, prediction); `docs/plan/multiplayer.md`.
- Have: `net/src/webtransport.rs` server transport + cert helper (bf65f26), `websocket.rs`, `connect_race.rs`, `browser.rs`; `crates/web` has no net code.
- Build: wasm client using `web-sys` WebTransport with a WebSocket fallback via `connect_race`; same snapshot codec and heal. Job: native server + native client + headless Chrome end on one digest. Print the blocked leg on failure.

### 11. Use the job pool in the sim and fork (W) — M, crates `ecs`, `arena`
- Book: geaen 7.6.4 Job system (gearus 8.6.4 "Система заданий"); geaen 7.6.5 Jobs.
- Have: `ecs/src/pool.rs`, `parallel.rs` fork-join (hash equal at 1/2/4/8 threads per RC36), only bake/verify callers; `fork.rs:314` spawns a thread per branch; `gym.rs:494` thread scope.
- Build: run fighter step, islands and crowd scratch through `fork_join_on`, golden unchanged at 1 and 8 workers. Run forks on the shared pool. Add a `loom` model of `JobPool::scope`. Keep web single-thread; the pool already degrades to inline.

### 12. Asset pipeline that ships: KTX2/BC/ASTC, meshopt, LOD chain, hot swap (W) — L, crate `assets`
- Book: geaen 6.2 Runtime resource management, 7.2 resource manager; found2 7.4 Texture mapping.
- Have: `bc.rs`, `texture_cook.rs`, `mesh_cook.rs`, `meshlet.rs`, five CC0 PBR sets in `content/arena/sources` (025f358f). Not done: KTX2 cook, meshopt cook, coarse-first bundle, live asset swap (`live.rs` deleted).
- Build: VR2 and VR4 as written, plus a small `asset.reload` door verb that swaps one asset by key and prints digest before and after. Gate: cooked textures 4x smaller than PNG, first frame before last blob.

### 13. MCP resources, prompts and progress (A) — S, crate `endpoint`
- Book: dushkin 19.1 "Журналирование и трассировка рассуждений" ("превращая «чёрный ящик» в систему, где можно шаг за шагом проследить путь"), 10.2 Tool adapter; `BOOK-GAPS-2` N36.
- Have: tools only; `mcp.rs` lists no resources or prompts (0 hits). Door docs exist as `DOC3/DOC4` pages (`llms.txt`).
- Build: serve the crate map, door reference, error catalog, `game.toml` schema and the pass-truth table as MCP resources. Add 5 prompts (new duel game, add a break, tune a hit, fix a red capture, make the web build). Add `notifications/progress` to `play.run`, `build.web`, `browser.capture`.

### 14. Agent memory and a task suite (A) — M, crates `endpoint`, `playtest`
- Book: dushkin 12.1-12.3 memory patterns (short/long, episodic, condensing), 14.1 Self-reflection, 16.1 Orchestrator-workers, 18.1 Blackboard; geaen's "game world as tools".
- Have: receipts and `watch_ledger.rs`; `playtest/src/tasks.rs`, `eval.rs`; GameCraft-style harness named in AI-NATIVE pillar 6.
- Build: a project memory the door serves (`memory.read/append`: decisions, failed fixes, tuned values, each with tape hash). A public task suite of 10 build tasks ("make a break", "add a mode") scored by checks (golden unchanged, pass-truth, reachability). Report pass rate per model. This is the proof that AI needs little human help.

### 15. Auto-tune loop with seed averages (A) — S/M, crates `playtest`, `editor`
- Book: geaen 15.x data-driven tuning; `BOOK-GAPS-2` N18, N26.
- Have: `editor/src/tuning.rs` stage + replay, `playtest/src/tournament.rs`, `exploit.rs` MCTS sweep.
- Build: `tune.sweep` verb: one knob, a range, 5 seeds, writes CSV and the win-rate curve; halve step when the loss rises; the result lands only through a validated change with undo.

### 16. Web quality tier and pipeline warm-up (W) — M, crate `render`, `web`
- Book: rustwasm (browser as platform library); geaen 10 rendering.
- Have: `quality.rs` tiers (potato...high), `webgpu_limits.rs`, `perf/src/vram.rs`; no `Web` tier, no warm-up (WEB8, WEB13 open).
- Build: add `Web` tier (render scale, shadow size, AO/TAA, particle and physfx caps); degrade ladder in a fixed order; compile every pipeline asynchronously before the first frame and print `pipelines=<n> ms=<t>` and `late_compile`.

### 17. Web audio on a worklet (W) — S, crates `web`, `audio`
- Book: rustwasm audio chapter (Web Audio decode and play); geaen 13 Audio.
- Have: `ScriptProcessorNode` (deprecated, main thread) with a quiet chime source (`web/src/browser.rs:43`, `web/src/audio.rs`).
- Build: an AudioWorklet that pulls from a ring buffer written by the mixer; fight cues as the source (the cue bank and mixer exist natively). Print `audio: state=running underruns=<n>`.

### 18. Netcode: choose the model and run two machines (E) — L, crates `net`, `game`
- Book: gaffer (see `gaffer-netcode` skill); geaen multiplayer; `BOOK-GAPS` section 6.
- Have: server-auth + prediction + snapshot; `P2pSession` no game caller; MP6 is loopback; ON30 internet open; lag comp and cheat checks were deleted (MP5, ON5).
- Build: owner decision D2. Then one verb `net.host` and `net.join` and one `net.lan` check with two real machines. Soak at 150 ms RTT with 9-tick resim window (`anim/src/net_pose.rs:34`).

### 19. Animation data: rotation keys, attachments, additive rotation (E) — M, crate `anim`
- Book: geaen 11.8 Compression techniques (quantization, sample dropping), 11.11.1 Attachments, 11.6.5 Additive blending; geaen 11.4.6 Metachannels ("event triggers at various time indices").
- Have: pose, blend, layers, state machine, IK, foot lock, inertialization, events; `pack.rs` has no rotation lanes; no socket type.
- Build: quaternion lanes in packed clips (smallest-three, 16-bit), a `Socket` (joint + offset) so a weapon or shield follows a hand, rotation additive. One `anim.attach` data row per prop. Test: weapon world pos follows hand; clip size drops.

### 20. Windows ship check and a runner for the product check (W) — M, crates `game`, `tools`
- Book: geaen 5.1 Start-up/shut-down; testing "Basic acceptance / smoke" (BAT).
- Have: `tools/product-check.mjs` (ownerless since the keeper was cut), `potato-check.mjs` (numbers, verdict withheld), GM3 clean-folder run never done, last product check 2026-10-13.
- Build: one `node tools/ship-check.mjs` that cleans a folder, builds `--profile ship`, runs the exe N seconds, reads the view line and receipt, compares to the reference captures (item 4), prints PASS/FAIL. Run it from a GitHub workflow on every push to the mirror. Without this "never broken" is unmeasured.

## 5. Next 8 (smaller or later)

1. Streaming world: connect `world/src/cells.rs` to `modes_world.rs` through the LoadQueue (L).
2. Engine config file and per-user settings with cvars (geaen 5.5) (S).
3. Input feel: rumble, disconnect pause, rebind conflict check (geaen 9 HID; S).
4. Property tests (`proptest`), `loom` for the pool, TSan leg (S).
5. Nav back into the arena (flow fields + crowd) or delete for good (M).
6. Localization wired in game (`ui/src/locale.rs` exists, no game caller) (S).
7. SIMD and wasm simd128 on skinning-free CPU loops; measure first (M).
8. wasm threads host (WEB14, LATER): needs COOP/COEP; Pages cannot set them.

## 6. Why this order

- Items 1-3 turn the browser tab from a picture into the game and give it content. Without them "web" is a demo.
- Items 2, 4 and 5 are what make the arena look good and prove it with numbers.
- Items 6-9 are the "hard stuff easy" layer: data in, game out. This is where an AI saves most of the human's time.
- Items 13-15 make the AI door a real build loop (docs it can read, memory, tasks, tuning).
- Items 10, 11, 18 are expensive but needed for multiplayer and speed.

## 7. Cross-checks and warnings

- Two docs say different things on the same row. Example: finish-line G11 says "not started: the playable arena has no rigid-body physics" while `crates/game/src/ragdoll_look.rs` and `break_look.rs` exist and `game/Cargo.toml:21` turns on physfx for `app`. Update the doc after the draw hook lands; do not tick rows from the doc.
- `status-2026-10-17.md` lists 41 suspect ticked rows whose code was removed. Rows I rely on above that are affected: MP5, ON5 (netcode), PX2 cloth (restored fresh as RZ7), HR2 (live swap), NV2 (crowd). Decide restore or re-open before adding callers.
- One arena test is red: `loc2_wired_slope_step_air_course_x2` (autostep after `1d01a410`). Do not stack item 11 on a red base.
- Not re-checked: CSM texel snap, cull bound on posed body, `PhaseArena` use, input buffer and dodge commit lock, Doppler, fenced fetched text, `tools/wiring-scan.mjs` wiring.
