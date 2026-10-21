## Receipts

Earlier receipt text lives in git history (`git log -p -- docs/SPRINT.md`).

<!-- template: new blocks append via `node tools/proof.mjs --skip-build`; do not hand-type golden/proof lines -->
<!-- 2026-10-21: the `node tools/land.mjs <packet> --dry` half of this template was removed with the loop tools on 2026-10-17 (commit 6bdf32bf; `tools/land.mjs` does not exist). Land by path and paste the proof output. -->
> Golden `d57b5c8ae3096614` holds for every landing below unless the block names a move; new blocks paste the land+proof output, not hand-typed hashes.

### 2026-10-21, OPT43 and OPT44 reuse every check, feature builds on the Windows runner

- 137b6c2d OPT43: gh-check keys each result by line, runner and inputs (crate closure, manifests, lock, content/, fixtures/, outside paths the code names) and answers an unchanged line from `target/gh-check/results/`. Proof: `node --test tools/gh-check.test.mjs` 18/18; live, the third `cargo check -p engine-audio --features device` printed `REUSED FAIL from 0 min ago (windows-latest, run .../39509451917)` in 0.4 s. Other sessions' calls kept results within minutes (engine-physics, engine-destruction).
- 137b6c2d OPT44: `cargo check -p engine-audio --features device` ran on windows-latest (run 39508991396, PASS, 50 s of compile) instead of the PC; the second run FAILED on another session's unfinished `crates/observe/src/supervisor_proof.rs`, as it should.

### 2026-10-21, OPT37 and OPT40 one slot pool for every cargo

- 6bfb92ed OPT37: `tools/jobserver.ps1` holds one named-semaphore jobserver pool (8 slots); `install` made it permanent (user env `CARGO_MAKEFLAGS`, hidden holder in the Startup folder); `tools/jobserver.mjs` `poolEnv()` joins `tools/gh-check.mjs` local runs and every `tools/proof.mjs` cargo to it. Proof: `node --test tools/jobserver.test.mjs` 3 pass (holder env, empty env, priority); the cap was measured by the A/B on a 7-dependency scratch crate, 1-slot pool 2 rustc max in 591 s against own `-j` 10 rustc in 588 s (the test's live leg hit the package-cache lock and stays gated). gh-check 15/15, proof 34/34.
- 6bfb92ed OPT40: gh-check and proof call `lowerPriority()` before cargo, so cargo and its rustc run below normal; timed runs keep normal priority. Proof: a grandchild of a `lowerPriority()` process reads `os.getPriority()` 10.

### 2026-10-20, NS31e2 closed on the re-home rule

- c7082f36 NS31e2: `CoverageTracker` lives in `crates/material/src/coverage.rs` — one tick-ordered tape per object id, `push`, `rollback` and `at` that never reads past the asked tick — and its four tests pass (`cargo test -p engine-material --lib coverage`). The row named `crates/viewer`, dropped on purpose in 1296f273, so its own command is retired; four rows of the same shard had already landed in their new homes, and the row is closed on that precedent rather than left waiting on an owner answer. The feed from `engine_app::ActorFrame::hit` is not part of it and belongs with the render re-home.

### 2026-10-20, NS31h2 material gallery and PQ18 arena latency

- 5a009518 NS31h2: `crates/playtest/src/material_gallery.rs` renders a sphere and a slab per record under day, torch and overcast through `pipeline_for`, then pairs the captures. 57 captures in 2.85 s on the RTX 3050 against a 120 s bar; marble to marble_wet dE 6.34, luminance x0.751, highlight 239.17, equal x2. Proof: 3 tests with `--features window`, 2 headless; clippy exit 0; workspace-check edge documented (99fa50d0).
- 1223bc81 PQ18: the arena window loop stamps input accepted, sim tick, present submit and frame end into `WindowLatencyLog`, behind the `app` feature. Proof: 7 loop tests; `PQ18 OK window=1 input_p50=3.767ms input_p95=5.974ms sim_p50=0.043ms present_p50=6.133ms gpu=reported`; `input_latency_gpu samples=66 warmup=60 present_p95=32.137ms verdict=pass` on the RTX 3050; golden x2. Note: the meter had been removed by an auto-backup commit (`f7b7c8ae`) and was restored first in beb39410.

### 2026-10-20, VS9 the daily arena shot

- fc2c97cf: `tools/arena-shot.mjs` runs the pinned capture at HEAD, writes three frames, three mask PNGs, manifest, per-pass timings, `masks.json` and `shot.json` under `target/vs9/`, and refuses a cube digest, 64x48 noise, an empty fighter box and magenta above 0.5%. Proof: `node tools/arena-shot.mjs` PASS at build beb39410, `frames=3 masks=3 magenta_max=0.000%`; `--self-check` 17/17; golden `d57b5c8ae3096614` PASS. Left: the pre-VS1 cube build is gone, so the cube-SHA refusal replays only as the pinned-digest rule.

### 2026-10-20, E79 dedicated server, E63 strict config, UW18 squad orders, ON25 TURN

- 701f9b34 E79: MasterList register/list, ServerBrowser and `HeadlessServer::beacon` on an IPv4 integer wire (3 B query, 23 B entry, 740 B reply under 1200 B), pure and never inside `tick`. Proof: `cargo test -p engine-net --lib dedicated` 10 passed x2; `cargo test -p engine-net --lib` 404 passed; listing `servers=1 fighters=2 tick=64 hash=0e2d1288d6e8d002 joined=2 ticks=64`; golden x2; clippy clean. Left: IPv6 registration refuses as `BadHost` until a widening row.
- 1a0cc6e0 E63: `layered_extract_strict` reads env then file then defaults and refuses any bad or unknown row as `BadConfig`; `GameConfig` derives `JsonSchema` with `deny_unknown_fields`. Proof: `cargo test -p engine-game --lib layered_extract` 9 passed (6 pre-existing + 3 new); config_vectors 8 passed; golden x2; `cargo deny check` ok.
- 1751ea1a UW18: `crates/ai/src/orders.rs` squad orders (move, hold, fall in, stack, mirror, wedge, breach family then bang/stinger/gas/none, restrain, collect, split), versioned tape encode/decode, typed `OrderError`, replay book. Proof: `cargo test -p engine-ai --lib orders` 9 passed; legion 14 passed 1 ignored; clippy clean.
- 577b9122 ON25: TURN allocation discipline (Allocate/Refresh lifetimes, CreatePermission before forward, ChannelBind multiplexing) opt-in on `RelayHub` via `with_turn`, default relay untouched. Proof: `cargo test -p engine-net --lib relay` 30 passed; ON30 QUIC relay leg 1 passed; release hash `d57b5c8ae3096614` x2; clippy clean. Left: ON27 wires the live allocation path.

### 2026-10-10, EG9 linear HDR frame and one forward shader file

- 58d22d62 (shaders reached HEAD in auto-backup 99b54cb7): forward writes linear radiance with exposure only into Rgba16Float; blit.wgsl does the one ACES plus sRGB tone map (tonemap.wgsl). All forward WGSL lives in crates/render/src/shaders/forward.wgsl via include_str! (static, skinned, depth-only, sky entries); no replacen on WGSL left (a test scans the sources); v1_skin.rs lost its second BRDF copy. attach_surface picked an sRGB swapchain while the blit also encoded sRGB (double gamma): fixed, device test within 2 bytes.
- Proof: F2P forward_target_is_linear_hdr_with_exposure_as_the_only_scale (Rgba16Float; exposure 4 gives 4.0x; values above 1.0 survive); arena --capture-dir sand region x 200-900 y 480-700: mean RGB 123.4/98.1/72.5 before vs 125.5/99.2/73.4 after, mean abs diff 1.3-1.9 of 255 (fog now mixes in linear); engine-render --lib 596 passed 16 ignored; golden d57b5c8ae3096614 unchanged; rust-quality render PASS (panic 24 to 22, casts 1005 to 1004).
- fog_composite_shows_on_the_live_frame and four display-space fog tests left tests_gpu.rs; a linear-space replacement lives in present/tests_linear.rs. The final #![cfg(test)] line in tests_linear.rs was checked with cargo check only (judge seat: rerun render --lib). engine-game --lib kit_import_carries_base_color_textures and kit_textured_sand_beats_flat_variance_4x fail only in the uncommitted VS3 arena_kit.rs work.

### 2026-10-10, EG6 GPU tests say when they skip

- 3290999f: new crates/render/src/test_gpu.rs: require_or_skip writes 'SKIPPED <test>: <why>' to stderr on a missing adapter and panics under E2040_REQUIRE_GPU=1; note_skip covers other skips (TIMESTAMP_QUERY). present/tests_gpu.rs routes its three init_or_skip helpers and the inline skip through it. tools/proof.mjs counts SKIPPED lines into RESULT (that hunk reached HEAD in auto-backup 99b54cb7).
- Proof: cargo test -p engine-render --features window --lib require_or_skip 4 passed and prints SKIPPED; serial render --lib on the committed tree 590 passed, 1 failed (fog_composite_shows_on_the_live_frame, already failing on HEAD, owned by EG9), 16 ignored; rust-quality render PASS; golden d57b5c8ae3096614 unchanged.
- Left: 24 other files keep their own no-adapter early returns (EG47); a parallel full render run stalls GPU tests (device contention), so the serial run is the evidence.

### 2026-10-10, EG34 ragdoll split decided and wired (integer balance in the v2 step)

- b3142784: under rules v2 every landed blow feeds the victim's integer BalanceState (hit stumbles; a second hit while staggered or a knockdown floors; a block chips stance; a parry staggers the attacker; fallen or catching fighters cannot move). Each fighter's balance hash folds into the checkpoint under tag 0xB1. The Rapier ragdoll stays client-local and is never hashed; HASH-LAW records the split, the 0xB1 tag and the EG43 temperament note.
- Proof: cargo test -p engine-physics --features rapier --lib balance_hash x2, 3/3 each (knockdown at tick 100 gives the same balance hash twice; equal with the Rapier ragdoll on and off); arena balance_wire_combo_floors_the_victim_and_hashes_twice green on master with EG43; arena --lib 137, physics 160, game 306; --features sim compiles; golden d57b5c8ae3096614 unchanged; rust-quality arena+physics PASS.
- Left: StaggerMemory, LimbPool and cut state are not hashed yet; nothing in the game feeds the ragdoll from the balance verdicts yet (fighter_look).

### 2026-10-10, EG43 v2 duels have winners (seeded per-slot temperaments)

- 655cce21: new crates/ai/src/personality.rs: each slot gets a temperament (reaction offset in ticks, preferred range, aggression) from the match seed through engine_core CounterRng, carried in the brain and installed in Sim::set_rules for rules v2 only; no wall time.
- Proof: the ignored 400-seed v2 split run x2 with equal numbers: 328/400 decisive (82%, was 0), 35/400 double knockouts (8.75%, was 399), slot 0 wins 176/328 (53.7%); no combat rules touched. engine-ai --lib 187 passed; golden d57b5c8ae3096614 unchanged; workspace-check PASS; rust-quality ai+arena PASS.
- The only engine-arena --lib failure at that state was the uncommitted EG34 test in the other seat's live.rs (tape-driven, no brains). Left: a HASH-LAW note that v2 digests moved with the temperaments (HASH-LAW.md is held by the EG34 seat).

### 2026-10-10, EG33 melee brains (quantized melee percept, nearest 3 foes, two-attacker tokens)

- f727e7a7: new crates/ai/src/melee.rs (MeleeView, FoeView::quantize at the edge: own hp/stamina permille, nearest 3 foes by (dist_cm, slot), foe swing phase and frames to active); SquadN claim/release tokens with MAX_ATTACKERS_PER_TARGET = 2; DepthBrain reads the view; new crates/arena/src/engage.rs plans tokens per tick from pose and fight state (rules v2 only).
- Proof: cargo test -p engine-ai --lib melee_percept x2, 16 passed (nearest 3 of 4 in order; third attacker waits, never above 2); engine-ai --lib 179 passed; engine-arena --lib 135 passed; golden d57b5c8ae3096614 unchanged; rust-quality ai+arena PASS.

### 2026-10-10, EG20 fighters collide (integer push-apart, pillars, round wall)

- 1643c1cd: new crates/arena/src/collide.rs runs once per tick after every slot moved: capsules push apart through controller2::separate_pair (reused), centres pushed out of pillar footprints, round wall at the clamp inset; integer mm only. Rules v2 only (ArenaRules.collision, with_pillar up to MAX_PILLARS); the checkpoint fold appends tag 0xC0 plus pillar footprints (HASH-LAW note).
- Proof: cargo test -p engine-arena --lib push_apart x2, 3 passed each (eight on one point spread in 30 ticks, v1 stays at 0 m as F2P; no centre enters a pillar; digest equal x2 and tagged); engine-arena --lib 135 passed; engine-game --lib 306 passed; golden d57b5c8ae3096614 unchanged; rust-quality arena PASS.
- Left: cooked-kit boxes are not wired because content/arena/placement.json has no pillars and sits in the game frame; pillars travel in ArenaRules for the caller to fill.

### 2026-10-10, EG36 process-wide statics become per-session (Claude Sonnet, f43bc2fa 4f1237c0)

- Endpoint half: STEP_HOOK, HOOK_SERIAL and the register/clear hook functions are gone; ServerState.play (PlaySlot) holds the bout, an Option<StepCaptureHook> set at construction and a Mutex<RequeryStore>; a step advances the bout before reading the cache.
- Four sessions on threads (two hooks, two software) keep their own hook, cache and bout, 5 of 5 repeats; no top-level static left in play_session.rs, requery.rs, fracture.rs or csg.rs.
- Digest follow-up: sim.step, sim.reset and play.step send digest as a decimal string; seed takes a string or a number. endpoint --features mcp --lib 482 pass; golden d57b5c8ae3096614; rust-quality endpoint PASS.

### 2026-10-10, EG4 crash hook and run log in arena.exe (Claude Haiku, 30e3519e cfa20a50)

- main() installs a std-only panic hook: a main-thread panic writes crash-<UTC>.toml through crash.rs, logs place and message, exits 102 (103 when the report fails); a worker panic logs one 'arena worker panic' line and unwinds to its owner; a panic re-raised on main through resume_unwind is caught by main's catch_unwind and written by crash::fatal_unwind.
- Every arena status line mirrors into target/logs/arena-<UTC>.log; E2040_TEST_PANIC_AT_TICK=N forces the crash.
- crash_hook 4 pass (worker_panic_leaves_the_process_alive_and_logs_once failed before the thread fix), --features app 5 pass with a real arena spawn panicking at tick 5, --lib crash 12 pass; golden d57b5c8ae3096614; rust-quality game PASS 97.

### 2026-10-10, EG23 a fork panic is a typed crash (Claude Haiku, def9e17b)

- Fork::try_run_n returns Result<ForkMetrics, ForkCrashed>; probe spam/baseline metrics return ProbeError (Refused or Crashed); the exploit sweep counts a crashed node as a visit, never a kill, and prints a CRASHED line; a crashed baseline stops the sweep with SweepError::Crashed.
- forced_worker_panic_is_a_crash_not_zero_metrics (real worker panic) and one_crashed_fork_reports_crashed_not_clean pass; engine-arena --lib fork 14 pass.
- run_n keeps its zeroed, documented crash-blind result for tournament, bench and endpoint callers; moving them to try_run_n is follow-up work.

### 2026-10-10, EG21 progress: steal fade and tension music, audio crate done (Claude Sonnet, 4f57d8dc 3399547b)

- A stolen voice fades over 8 ms into one of 16 reserve slots (step at most 0.1 x bus gain per sample vs 0.5 for a cut); steal, fade and reap allocate nothing.
- Tension music: FeedTx::tension drives calm/build/fight/climax playlists that switch on the next bar line with a one-bar crossfade; stinger_kill and stinger_roar duck the bed to 35%; byte-identical twice at block sizes 480, 1024 and 37, zero-alloc render.
- engine-audio --features device 235 lib tests pass; rust-quality audio PASS (casts 117 to 88).
- Open: the game-side hunk (AudioCommand cue and pos, AudioSink, listener, tension and stinger calls), spec in target/reports/engine-review-2026-10-10/handoff-EG21-game.md; then delete the legacy Mutex<Mixer> adapter. Music and stingers are placeholders.

### 2026-10-10, LJ1 progress: look.judge rubric and door verb (Claude Sonnet, 32ed363c 43e1a88e)

- look/judge.rs scores rubric L1-L9 as typed JSON, byte-equal twice; blank, cube-only and magenta-cube frames score 0 on L1, drawn fighters score above 0; a manifest claiming fighters over a blank frame still scores 0.
- Real staged-fight set: score 467, pass false (L1 L4 L5 pass; flat sand and no kit fail L2 L3 L6 L7 L9), as a judge should say.
- Open: the verb answers unsupported_look_judge until a host calls register_look_judge; the game binary links playtest and endpoint and registers it at startup (no endpoint to playtest edge). playtest look 41 pass x2, endpoint mcp lib 478, golden d57b5c8ae3096614.

### 2026-10-10, EG30 progress: machine eyes core (Claude Sonnet, 32ed363c ff146e9d 43e1a88e)

- playtest look/metric.rs: integer SSIM, deltaE, 64-bin histogram and masked regions, pinned against a float oracle (equal frames 10000, brightened sand 9775, one-pixel shift 2711); look/manifest.rs reads capture manifests.
- Open: the window hook. arena.rs capture_offscreen pushes each PNG into a CaptureSet and calls look::judge::judge once per captured frame (after EG4 frees bin/arena.rs).

### 2026-10-10, EG37 spawn failures return Err (Claude Haiku, 5593d90a)

- sky look worker and provider stream reader spawn through thread::Builder; failures map to SkyError::Device and StreamError::Unavailable instead of panicking; present/look.rs forwards the error.
- sky_spawn_err and spawn_failure_is_err pass through an injected failing spawner (a private seam, not cfg(test)); render lib 418 pass, provider lib 197 pass; golden d57b5c8ae3096614; render panic sites 24 to 23.

### 2026-10-10, EG10 persistent fighter vertex buffers (Claude Sonnet, ad021d9f)

- One fixed vertex buffer per slot and part written by one queue.write_buffer per frame; one bind group per texture built at upload; one 64 KiB material uniform ring with dynamic offsets.
- arena --exit smoke: steady_buffers=0 steady_bind_groups=0 live_waits=0 after frame 1 (counter test fails on the old per-draw bind groups).
- 8 fighters 1280x720, 3 interleaved runs: fighters 3.74-4.29 ms to 2.63-2.71 ms; present minus acquire 5.2-6.0 to 3.5-3.7 ms; present stays at the vsync pace.
- Left: v1_skin.rs allocates per part per frame and mesh_keys() sorts a fresh Vec each frame (EG32 GPU skinning removes the first); fog_composite_shows_on_the_live_frame was already red at base (goes with EG9).

### 2026-10-10, EG3 a hit needs range, facing and team, rules v2 (Claude Opus, 08ce1f03)

- Rules v2 in combat: a hit needs the blade's sweep capsule to reach a victim in front of the attacker; teammates are never hit; block and parry count only when the shield faces the attacker. Integer mm and a fixed-point facing table.
- Folded fixes: the depth brain's facing-relative wish is rebuilt from intent relative to the foe (brains no longer walk into the wall); v2 reads strikes and guards from pre-tick state, so slot order no longer cancels a simultaneous strike.
- Finding: the 99% slot-0 win rate came entirely from resolution order. With it gone identical brains double-KO 399 of 400 seeds; EG43 adds seeded per-slot personalities, EG44 puts the rules version on the wire so the window can play v2.
- combat 201 pass, arena 124 pass (v2 hash x2 equal at 2 and 8 fighters, never equal to v1; mirrored duel keeps equal hp every tick), game 305 pass; golden d57b5c8ae3096614; HASH-LAW v2 note.

### 2026-10-10, OPT27 progress: arbiter and VRAM read (Claude Opus, 32ed363c ff146e9d)

- provider/arbiter.rs: Arbiter::observe(frame p95, budget, VRAM) places local inference on Gpu, Cpu, Defer or Remote with enter/exit hysteresis (2 samples to degrade, 6 to recover) and typed ArbiterNotice; the placement is one atomic byte the request path reads.
- local.rs with_route: Gpu sidecar, CPU-only loopback sidecar, bounded defer queue (1-64, refusals counted, no double queue), Remote as a retryable refusal.
- render/backend.rs read_vram(): DXGI budget and usage on Windows (budget_mb=5233 here), typed unknown elsewhere; one reviewed unsafe COM block, root lints comment updated.
- provider --lib 196 pass, arbiter 7/7 (scripted series, no flap on 40 alternating samples, identical notices x2); golden d57b5c8ae3096614.
- Open: nothing feeds the arbiter live yet (needs game or endpoint wiring of frame p95 and read_vram), adapter_for in config.rs builds no route, and the window-build budget check needs a live run.

### 2026-10-10, EG16 the door returns numbers (Claude Sonnet, 4bd3c1ed)

- sim.observe is registered in the gym pack: hp, stamina, phase, pos and yaw per fighter, the newest events, seed and digest as decimal strings.
- sim.step and play.step share one held match per session (ServerState.live); a longer prefix of the same tape steps only the new ticks (step counter 20, 40, 60, 60, 90 for prefixes 20, 40, 60, 60, 30).
- Door test: 60-tick seed-2 bout, the victim's hp falls by exactly the hit event's damage; observe output is byte-equal across two sessions and across a reset.
- Held match is GymLive in arena/gym.rs, not LiveMatch: LiveMatch gives only pre-step rows with no Sim handle. GymLive can shrink to a wrapper once LiveMatch exposes fighter and fight state.
- engine-endpoint --features mcp --lib 478 pass; engine-arena --lib 123 pass; golden d57b5c8ae3096614; rust-quality endpoint PASS.

### 2026-10-10, EG36 progress, fracture half (Claude Sonnet, f43bc2fa)

- TMP_CLIPS and TMP_SKIPPED are gone: each fracture call owns a CellWork scratch and fracture_counted returns ClipCounts; fracture_at wraps it.
- 50 fracture digests match before and after; fracture_counts x2 passes, including six jobs on two interleaved threads equal to serial; golden d57b5c8ae3096614.
- Open: STEP_HOOK and REQUERY in endpoint play become PlaySession or server state fields; that half waits for EG16 to free the files.

### 2026-10-10, EG5 timing asserts leave the default physics suite (Claude Haiku, f3b574e1 53132b34 c292cb9a 85c768d7)

- solver.rs pair-discovery and a5 p99 gates, cloth settle, lod army 4 ms, query 1 ms release budget and the idmap Fx-vs-Sip 20% gate are ignored timing tests; their digest, allocation and hit asserts stay in the default run.
- Accept grep: the elapsed() reads left in non-ignored physics tests (cloth_gap, cloth_couple, rb_query_kinds) print only; none asserts on time.
- Also 99829b39: mcp_stdio EXPECTED_TOOLS gains game.build, sim.build_encounter and test.run (inbox E37).

### 2026-10-10, FC2 halve the top frame cost (closed by EG2, 8be741f8)

- The top cost was the blocking readback, fog and GTAO tranche; present 39.2 ms to 8.0 ms per frame (more than half), frame interval p50 8.3 ms.

### 2026-10-10, EG2 no GPU wait in the live frame (Claude Sonnet, 8be741f8)

- Fog is the unified closed-form height fog in the forward fragment shader (horizon colour uniform, sky at infinity). Look probes, IBL chart, depth readback with CPU decode, and the fog and GTAO tranche run only under `--look-probes` and on the offscreen twin; `render_offscreen` byte-identical.
- GPU timestamps read late via non-blocking `map_async`; frame-split prints cpu, interval and gpu p50/p99.
- Measured, RTX 3050, 1280x720, 8 fighters, 3 runs: before `present=39.2ms` (about 17 fps); after `present=8.0ms cpu_ms p50=8.0 interval_ms p50=8.3 gpu_ms p50=0.25`. `arena waits: live_frames=300 live_maps=0 live_waits=0` (also 0 over 2500 frames); the `--exit` smoke fails on any live wait.
- Next cost: fighters 2.7-4 ms (CPU skin plus about 64 buffers per frame, EG10), then per-draw bind groups.

### 2026-10-10, EG7 progress, row stays open: frame gate reads pixels (Claude Haiku, d7991776)

- `tools/frame-triage.mjs` counts fighter-kit pixels per frame via `isFighterPixel`, floor 50; the manifest count is reported, never used. `node --test tools/frame-triage.test.mjs` 5 pass.
- Open: every real capture set under target/ counts 0 fighter pixels, so none passes yet; the old manifest gate hid that. Ticks with the first real capture that shows fighters (after EG2).

### 2026-10-10, EG15 door ids and hashes as strings (Claude Haiku, 8d9811a0)

- MCP hashes, digests and ids go out as decimal strings; input takes string or number; counters stay numbers.
- Proof: mcp_dto 4, endpoint lib with mcp 462 (schema drift test included), default endpoint lib 149 pass.
- Not done by design: scene.place name lookup (scene_tools.rs and test MCP8 pin the bracketed id as the only key).

### 2026-10-10, EG22 safe UV wrapper checks its inputs (Claude Haiku, cfa7ea1a)

- `ffi::generate` refuses short normals, wrong UV length, bad position or index counts and out-of-range indices with `GeoError::Malformed` before the xatlas call.
- Proof: `ffi_wrapper_refuses_bad_arrays_before_the_c_call` (`--features uv`); default suite green; rust-quality geo PASS.

### 2026-10-10, EG24 progress, row stays open until the arena bind: learned int8 policy (Claude Opus, 4d40e6bf)

- engine-nn: `infer_into` with caller scratch (0 allocs over 64 calls), `quantize_lanes` (half-even, saturate, NaN to 0) with golden bytes, `Model::digest()`.
- distill: std-only seeded 2-layer softmax and Adam trainer, int8 export; 60,000 teacher rows from 50 duels; `fixtures/nets/depth_student.e2nn` (406 B, digest `6aceb82b2a856f5c`); `distill_duel --check --repeat` regenerates identical bytes.
- Gate, 1000 bouts, sides swapped: student vs teacher 502-497-1, Wilson [0.47, 0.53], p 0.90.
- Found: the depth brain's wish is facing-relative but the arena applies world axes (brains walk into the wall), and slot 0 wins about 99% of decisive bouts. Both go into EG3 rules v2.
- Open: the arena bind (`ArenaPolicy::Net`, digest in the tape header, inference at the AI period in `Sim::step_with`).

### 2026-10-10, EG28 persistent deterministic job pool (Claude Opus, 21b43393)

- `crates/ecs/src/pool.rs` `JobPool`: N workers started once (0 = inline), `scope()` for borrowed jobs with an open-job counter (the caller helps while waiting), lowest-numbered panic re-raised, drop joins workers; `par_chunks_map` merges in chunk order. `jobs.rs` and `parallel.rs` keep their API on one shared pool; no ecs call spawns threads.
- Real use: physics `step_islands` takes `&JobPool` (new edge physics -> ecs in Cargo.toml, CRATE-MAP and workspace-check).
- Proof: `cargo test -p engine-ecs --lib job_pool_threads` x2 9/9 (0, 1, 2, 4, 8 workers byte-identical across chunk sizes); ecs 75, physics 158, physics rapier islands 24 pass (fighters hash the same at 1-8 threads); golden `final_hash=d57b5c8ae3096614`.
- One `unsafe` (`Scope::spawn` lifetime erase, std::thread::scope shape, abort-on-unwind guard); orchestrator reviewed it and accepts the rust-quality baseline change.

### 2026-10-10, EG17 progress, row stays open: pinned sampling (Claude Sonnet, 25f0d1a4)

- Every adapter body carries a system prompt, temperature 0, seed 2040 (none for Anthropic, which has no seed) and tools when given; body tests per adapter plus a loopback test of all six. Pins sit on the adapter (`with_pins`), not on `AiRequest`.
- Open: TLS. Needs the `engine-provider` tls feature in `crates/endpoint/Cargo.toml` and `crates/playtest/Cargo.toml` plus a local TLS stub for the empty HTTPS test (new dev-dependencies).

### 2026-10-10, OPT24 progress, row stays open: provider cassettes (Claude Sonnet, 6387b126)

- `crates/provider/src/cassette.rs`: content-addressed record/replay wrapper for any adapter, keyed by adapter, model, request digest and schema version; replay x2 with the endpoint on a closed port is identical; changed prompt, system, temperature, seed, tools or ladder start misses with a typed reason.
- Open: the EV1 half. `playtest/door.rs` `door_call` builds a `MockAdapter`; wrap the real adapter in `CassetteAdapter::new(adapter, CassetteStore::new(CassetteStore::default_dir()), CassetteMode::parse(env)?, label)`.

### 2026-10-10, OPT25 structured-output ladder (Claude Sonnet, dd7b2bfb)

- `crates/provider/src/ladder.rs`: native schema, JSON mode, text with bounded repair, scripted; every backend reports its rung (`last_rung()`, `ladder_report()`); rung drops logged per adapter (`ladder_events()`).
- Proof: `cargo test -p engine-provider` lib 189 pass; malformed native JSON drops one rung and logs; garbage at every rung ends scripted with `mutated` false; auth, quota, timeout go straight to scripted. rust-quality PASS. Bug fixed: `transport::parse_chat_body` accepted `noop: false` and narrowed 4294967296 to 0; now uses the strict parser.
- Left: the PV5 matrix has no rung column (rungs asserted in ladder tests); native and JSON wire shapes are stub-tested, not live.

### 2026-10-10, EG1 follow-up EG1b: incremental presentation (Claude Opus, 9ddbebbf)

- `LivePresentation` in `crates/arena/src/live.rs` steps the recording sim once per tick; `App::extend_presentation` calls it. Before: 2(n+1) steps per tick with the cursor on (360,600 for 600 ticks); after: 600.
- Proof: `live_presentation_equals_prefix_presentation_runners_tick_for_tick` (arena, 2 and 8 fighters), `live_presentation_equals_prefix_replay_oracle_600_ticks` and `presentation_costs_one_step_per_tick_and_rebuilds_only_when_turned_back_on` (game); arena 119 and game 305 pass; golden `final_hash=d57b5c8ae3096614`.

### 2026-10-10, EG31 durable writes fsync, checked nav casts (Claude Haiku, db6696a1 03324cf1 f5d01bc5)

- save, save fallback and editor durable writes: write, sync_all, close, rename, then a best-effort parent dir sync (Windows ignores it).
- nav: `tile_of` and stamped-path decode use `u32::try_from`; `round_mm` refuses NaN, inf and out-of-i32 at three sites. Real bug caught: `sample_height` gave `Some(0)` for a NaN center.
- Proof: engine-save --lib 23, engine-editor --lib durable 13, engine-nav --lib 87 pass; rust-quality PASS; golden fixture `final_hash=d57b5c8ae3096614` (orchestrator run with EG1 and EG31 in).

### 2026-10-10, EG1 incremental live match (Claude Opus, 5d3d8b5d)

- `LiveMatch` in `crates/arena/src/live.rs` steps one Sim tick per call; `game::App` holds it. Before: tick n cost 3(n+1) steps (540,900 for 600 ticks); after: 600.
- Proof: `tests::ticking_600_times_costs_exactly_600_sim_steps`, `tests::live_match_equals_prefix_replay_oracle_600_ticks` (2 and 8 fighters, LAN input every 7th tick), `live::tests::live_match_equals_whole_match_runners_tick_for_tick`; arena lib 112 and game 303 pass; rust-quality PASS both crates; golden fixture `final_hash=d57b5c8ae3096614`.
- Left: `extend_presentation` still replays twice per tick with the cursor on (EG1b, same coder).

### 2026-10-10, MP10 snapshot interpolation (code re-landed after gate repair, row open)

- Landed lane `mp10` (lightyear shape, MIT verified live): new `crates/net/src/interp.rs` (~870 lines: Hermite C1 Catmull-Rom tangents, Fixed yaw RenderQuat LUT/CORDIC, nlerp below 1.8° else slerp, bounded InterpRing, QueryKind guard interpolate/extrapolate/refuse past 2-tick horizon, integer ticks, zero `as`, zero libm, 7 tests) + `lib.rs` +3 (pub mod interp) + `delta.rs` 1 line (wrap_decideg pub(crate)). snapshot.rs untouched (steal-b09 live, zero-byte overlap verified). Consumer: feed_history(&ConfirmedHistory); sample_at has no non-test caller yet, excused by boarded consumer MP17 (needs MP10).
- Tests (lead re-ran on applied tree): engine-net --lib 225 passed 0 failed 2 ignored (218 + 7 new); packet-gate PASS 3 paths; quality 94 A 0 regressions. Judge VERDICT PASS (RUST 4/5 DEPTH 3/5). Row NOT ticked: EVIDENCE doc row for the PORT cite is remainder (researcher remainder; steals.md is not lead-editable — one DOCS one-off next wave). Golden held (presentation-only, digest-x2 in suite). Unverified: novel-seed rerun, integration-target + loopback solo (pre-existing reds, lib suite is gate).
- Re-land after revert c1945cbd: both gate failures were text-regex false positives, so the gates were fixed, not the code. New `tools/sim-source-scan.mjs` backs `workspace-check`: a `.sin(`-style call passes only when its receiver is provably `engine_core::Fixed` (derived from fixed.rs; unproven or float receivers still fail, 15 negative cases in `tools/workspace-check.test.mjs`), and GE2 reads identifier tokens, not comments or strings. interp.rs names the snapshot ABI through `pub type BodySnap = FighterSnap;` (delta.rs, already ledgered), so the ledger did not grow. Proof: workspace-check PASS, engine-net --lib 225 passed 0 failed 2 ignored, quality 94 A, golden d57b5c8ae3096614 held. Row text says `snapshot.rs`; the code lives in `interp.rs`, so the row stays open until the owner reconciles it.

### 2026-10-10, build-level session split (SALVAGE lane landed, no row ticked)

- Landed lane `build-level-session-split` (SALVAGE-build-level-session-split planner split): verbatim extract of LevelSession preview/apply/undo (SessionPreview, Applied, ApplyOutcome, Undone, fresh_session) from `crates/endpoint/src/build_level.rs` (8+/241-) into new `crates/endpoint/src/build_level_session.rs` (259+/0), wired via re-export; real consumer `build_game.rs:2305,2310` (LevelSession::new) plus 4 test sites. UNLOCKS ARC9 + ARC12 level spine writes (prerequisite split; moves no arena pixel or hash). Worktree held a corrupted unclaimed variant (wrong `#[path] ../../`, test-red E0308 family at :40); stashed to `stash@{0}` (spill backup also under $env:TEMP/opencode/spill-backup) and the judged patch applied fresh, session file byte-identical to lane.
- Tests (lead re-ran on applied tree): engine-endpoint --features mcp --lib 450 passed 0 failed 1 ignored (E0583 gone: HEAD gym.rs carries the #[path] fix); packet-gate PASS 2 paths; quality 89 B 0 regressions. Judge VERDICT PASS owned-scope (RUST 4/5 DEPTH 3/5). No board row ticked (pure extract). Unverified: full workspace suite; golden x2 (tick untouched).

### 2026-10-10, EV20 wired pixels (EV20 ticked)

- EV20 ticked: lane `ev20` capture hunks in `crates/game/src/camera_feed.rs` (+786/-0, no second path): single door `game_capture_png` (validate, render, encode, 4 MB bound), deterministic `render_capture_rgb`, stored-deflate `encode_capture_png`, sha-pinned `ShowcaseSet` manifest fail-closed on 0 frames, zero `as` casts. Pixels flow into existing CaptureSink validators; sink stays sole disk writer.
- Tests (lead re-ran on applied tree): engine-game --lib 299 passed 0 failed 6 ignored; capture_tests 6/6 (judge re-ran); showcase set target/showcase/2026-10-10/ev20-game present (15 PNGs 320x200 + manifest, lane-verified --clip --check-only + --set green); packet-gate PASS; quality 96 A 0 regressions. Judge VERDICT PASS full review (RUST 4/5 DEPTH 3/5; zero non-test callers yet, Q1 open+boarded owns the capture_with call wire). Golden d57b5c8ae3096614 held (suite golden asserts green, tick untouched). Unverified: Tier C reference-GPU 15-frame set with real adapter lines (O5 pilot); forge PIE pilot run; endpoint-side hook adapter (out of row); full clippy/rustfmt/second-seed runs (judge-missing gates, lane-green).

### 2026-10-10, MP9 input-compression experiment (MP9 ticked)

- MP9 ticked: lane `mp9` rle_tag hunks in `crates/net/src/session.rs` (+407/-1, one codec): twin RLE helpers merged into one infallible `rle_tag` (saturating_sub + TryFrom + unwrap_or, debug_assert 1..=RLE_MAX_RUN); both production `.expect()` (:799/:805) gone, test-only expects untouched. Saving raw=169 packed=60 = 64% (bar 20%), hashes identical (06344cd7d7ec7eeb, checkpoints=43).
- Tests (lead re-ran on applied tree): engine-net --lib 218 passed 0 failed 2 ignored (master tree); MP9 filter saving+hash green; different_seeds_diverge green on novel seeds; packet-gate PASS 1 path; quality 94 A 0 regressions. Judge VERDICT PASS (RUST 4/5 DEPTH 3/5; lane-side ratchet FAIL is stale-baseline artifact, root measures 237 with PASS). UNLOCKS MP6 played-proof codec leg. Unverified: full clippy (pre-existing windows-sys command-line-too-long infra); golden x2 not rerun (no tick touched, hash-neutral by identical codes).

### 2026-10-10, EV19 verify-after-record (EV19 ticked)

- EV19 ticked (pieces EV19.a+b): lane `ev19a` (`tape.rs` +63: FileVerifyError + Tape::verify_file + test, lib.rs untouched) + lane `ev19b` (`src/cli.rs` wiring + new proof_cli test, main.rs untouched). Kill-9-style truncated copy refuses verify naming the partial file; clean record+verify then replay x2 byte-identical; overhead 0.29% ≤ 1%.
- Tests (lead re-ran on applied tree): proof lib 81 passed 0 failed 1 ignored; proof_cli 5 passed; packet-gate PASS both lanes (lane roots); quality 93 A (proof) / 91 A (root) 0 regressions; workspace-check PASS. Judges VERDICT PASS both (ev19a fast-lane RUST 4/5 DEPTH 4/5; ev19b RUST 4/5 DEPTH 4/5). Unverified: full proof.mjs x2 (golden via frozen_default + golden_hash_untouched green in suite).

### 2026-10-10, PX36 capture-point servo bias (PX36 ticked)

- PX36 ticked: lane `px36` bias hunks in `crates/physics/src/balance.rs` (+338, PX1 servo frozen, BalanceState hash untouched): recovery 165→148 ticks (improvement 17 ≥ 10) on the 5-strength shove set; shuffled/reversed insertion digests equal; bias path alloc-free.
- Tests (lead re-ran on applied tree): balance_matrix 4/4; full physics lib 145 passed 0 failed 2 ignored (the lane-base flake `pair_discovery_bench_prints_lower` passes fresh on HEAD); packet-gate PASS (lane root); quality 100 A 0 regressions. Judge VERDICT PASS (RUST 4/5 DEPTH 3/5, novel pd_drive 4/4 + rapier 1/1 seeds). UNLOCKS PX1 servo recovery. Unverified: frame-moved --lane unsupported by tool (sim MOVED cited instead); full proof.mjs not rerun (hash-neutrality via green asserts).

### 2026-10-10, MCP24 scene.place yaw gap (MCP24 ticked)

- MCP24 ticked (piece AIDOOR.b): lane `aidoor-b` snapshot/yaw hunks in `crates/endpoint/src/scene_tools.rs` (+97, single SetLocal via reused quat_of_yaw, no second mutator). Landed scene_tools.rs ONLY via --include (lane gym.rs 1-line hunk excluded: byte-identical to master 440ad21d).
- Tests (in-lane, judge re-verified): F2P parented_place_refuses_loudly_and_yaw_roundtrips 1/1 + full mcp lib 448 passed 1 ignored; default-lib 148 green; packet-gate PASS (lane root, scene_tools path); quality 89 B 0 regressions; workspace-check PASS. Judge VERDICT PASS (RUST 4/5 DEPTH 4/5). Unverified on master: parallel session's uncommitted build_level.rs edit transiently reds the mcp test build (same E0583 family, not this lane); F2P re-verified in lane post-gymfix.
- aidoor-a (MCP25) FAILs its review (DEPTH 2/5: policy unwired at the door_dispatch seam); one repair queued with named fixes.

### 2026-10-10, EV18 retrieval-stress leg (EV18 ticked)

- EV18 ticked (piece AIDOOR.c): lane `aidoor-c`: `crates/playtest/src/eval.rs` (+40: `run_distractor_stress` + test) + 8 new `fixtures/eval-distractors/` briefs. With/without runs over 3 tasks (a01, a04, a09) x 3 backends name `retrieval_dilution`; one command reproduces the report.
- Tests (lead re-ran on applied tree): engine-playtest --lib 207 passed 0 failed 2 ignored; new test green; golden d57b5c8ae3096614 stands; packet-gate PASS 9 paths; quality 95 A after open baseline accept (eval.rs 3rd big_file, reason in baseline); workspace-check PASS. Judge VERDICT PASS full review (RUST 4/5 DEPTH 3/5). Decides AX10 graduate-or-kill next. Unverified: clippy blocked by pre-existing engine-physics lints outside owned paths; full proof.mjs not rerun.

### 2026-10-10, MP11 scripted-human bout (MP11 ticked)

- MP11 ticked (piece MP11-A rebuild; old PASS slice evaporated so rebuilt, not rebased): lane `mp11a-rebuild` fixture hunks in `crates/net/src/scale.rs` (+40 test-only): `bout_tape_bytes` (4800B LE tape both ends) + `human_bout_tape_bytes_match_on_both_ends` (bytes equal both ends, nonzero, seed-23 negative control, real author_tick path).
- Tests (lead re-ran on applied tree): engine-net --lib scale 19/19; new test 1/1; frozen_default golden d57b5c8ae3096614 green after both landings; packet-gate PASS (lane root); quality 94 A 0 regressions; workspace-check PASS. Judge VERDICT PASS fast-lane (RUST 4/5 DEPTH 3/5, novel seed 999 check). UNLOCKS MP6 distributions confirm (comment-only follow-up, not this piece). Unverified: full proof.mjs x2 not rerun (frozen_default asserts the golden const); clippy not rerun.

### 2026-10-10, HS15 tape-to-dataset export (HS15 ticked)

- HS15 ticked (last piece HS15.d): lane `hs15d` test-only hunk in `crates/proof/src/dataset.rs` (+148, no behaviour change): `manifest_json_is_the_only_wire_format_and_refuses_unlicensed` (canonical to_json deterministic, parse_json roundtrip with no Shard::decode, empty-license/blank-trains/zero-digest/over-long refused, flipped digest parses but verify_shard refuses).
- Tests (lead re-ran on applied tree): engine-proof --lib 80 passed 0 failed 1 ignored; new test 1/1; frozen_default golden d57b5c8ae3096614 x2 identical; packet-gate PASS (lane root); quality 93 A 0 regressions; workspace-check PASS. Judge VERDICT PASS (RUST 4/5 DEPTH 3/5). Unverified: full workspace suite not run; clippy engine-ecs/engine-physics pre-existing outside owned path.

### 2026-10-09, CT41 LOD table (CT41 headless done, GPU cells PENDING)

- CT41 landed (headless table only): `content/fighters/LOD-REPORT.md` (108 lines, tris/payload/digests inline, adapter RTX 3050 line, GPU ms/VRAM PENDING). Judge VERDICT PASS CT41 (834, lod_swap 9 + mesh_cook 13 re-verified).
- Held back: CT56.b 3 files (cook script cannot run on master without piece-a inputs; rides full 7-file 835 review, then lands together). CT56.a 4 files ride the same 835 review.

### 2026-10-09, BX5.b double-loss masks (BX5 stays OPEN)

- BX5.b landed (row stays OPEN, single-loss BX5.a plus remaining fixtures ride later pieces): lane `bx5b` (injured_double.rs new 396 lines, lib.rs mod). Commit names BX5.b per splitter rule.
- Tests (lead re-ran on applied tree): engine-anim --lib 294 passed 0 failed 1 ignored; packet-gate PASS 2 paths; quality 97 A 0 regressions. Judge VERDICT PASS (833, advisory needless_range_loop :98 non-blocking). Unverified: visual capture not run (sim-data piece).

### 2026-10-09, CT23.a combat intake (CT23 stays OPEN)

- CT23.a landed (row stays OPEN, 80-clip bake rides later pieces): lane `builder-content-r17` intake module (clip_intake.rs new 605 lines, retarget.rs +bake_combat_batch caller, lib.rs mod). Commit names CT23.a per splitter rule.
- Tests (lead re-ran on applied tree): engine-assets --features import --lib 206 passed 0 failed 2 ignored; packet-gate PASS 3 paths; quality 96 A 0 regressions. Judge VERDICT PASS (832). Unverified: CT23.b consumer row not boarded; clippy collapse.rs master-red pre-existing.

### 2026-10-07, RL5.a bake extract (RL5 stays OPEN)

- RL5.a landed (row stays OPEN, pieces b/c queued): lane `rl5a-look` structural extract (baked_bake.rs new 543 lines, baked_gi.rs 2597->2125, lib.rs mod, look.rs rebind). Commit names RL5.a per splitter rule.
- Tests (lead re-ran on applied tree): slice 1 passed, full engine-render 389/12 ignored; packet-gate PASS 4 paths (1 moved-casts warning); quality casts 1013->1014 (master 1013->1015 — pre-existing drift, landing improves it by 1); applied bytes byte-identical to lane bytes. Golden in-lane green (applied-tree golden blocked by unrelated lod.rs markers). Judge VERDICT PASS (605). Picture still cubes (structural slice).

### 2026-10-07, BX43 set budget lock (BX43)

- BX43 ticked. Landed lane `builder-body-bx43` 1-test hunk (+55 in set.rs): 256th push refuses TooMany{255}; set_param + inverse roundtrips on 7 shipped records; bytes x2. Lane patch never collected (lane unknown to lanes admin) — landed via master-relative minimal diff (55+/0-).
- Tests (lead re-ran on applied tree): F2P set_budget red-to-green 1 passed, P2P engine-material 81/81 (incl BX42 lock); packet-gate PASS 1 path; rust-quality engine-material PASS 98 A 0 regressions. needs BX12 ticked. Judge VERDICT PASS (604).

### 2026-10-07, BX42 fourteen-shipped roundtrip (BX42)

- BX42 ticked. Landed lane `builder-body-bx42b` 1-test hunk (+41 in record.rs): 14 shipped records parse + emit x2, per-record sim digest, kit digest == BX12 table digest, tamper moves sim not look.
- Tests (lead re-ran on applied tree): F2P fourteen_shipped 1 passed (missing before), P2P engine-material 80/80; packet-gate PASS 1 path; rust-quality engine-material PASS 98 A 0 regressions; golden untouched (test-only). needs BX12 ticked.

### 2026-10-07, HS15 export receipt (HS15 stays OPEN)

- HS15 receipt landed via master-relative minimal diff (214+/0-, judge PASS 598): `DatasetEntry`/`DatasetReceipt`/`verify_dataset` beside landed capture + hour test. Stale base-relative patch could not apply — landed with `minidiff`.
- Proof: in-lane `cargo test -p engine-proof --lib dataset` 13/13 (88s, capture + receipt + hour), quality proof PASS 84 B 0 regressions, packet-gate PASS 1 path; applied tree verified byte-identical to lane bytes (CRLF aside). Applied-tree test run BLOCKED by unrelated lod.rs markers (engine-physics fails to compile; dataset code never reached) — re-run dataset suite after marker fix; row stays OPEN until then.

### 2026-10-07, DT4 train-test close-out (DT4 already ticked)

- DT4 leftover landed via master-relative minimal diff (0+/42-, judge PASS 600): deleted never-compiling train-gated `trained_buffer_student_beats_static_slice` (burn exceeds cap); std suite untouched. Stale base-relative patch could not apply — landed with `minidiff` (git's own hunks, headers fixed, purity-gated).
- Tests (lead re-ran on applied tree): engine-distill --lib 37/37; packet-gate equivalent PASS (deletion-only verified); rust-quality engine-distill PASS 96 A 0 regressions, casts 15==baseline. Train-backend proof remains a remainder for a train-capable run. No row change.

### 2026-10-07, HS15 hour accept slice (HS15 stays OPEN)

- HS15 hour slice landed (row stays OPEN): lane `builder-body-hs15` test-only +76 (`hour_of_tapes_exports_and_reverifies_twice`): 3x72000-tick duel tapes export wide-stride, encode-roundtrip, verify x2.
- Tests (lead re-ran on applied tree): hour 1 passed (61s), dataset filter 11 passed, engine-proof quality PASS 84 B 0 regressions; packet-gate PASS 1 path. Judge VERDICT PASS (599). The hs15-export receipt rebase (592) still needs a rebase onto this tree — queued as repair.

### 2026-10-07, PQ10 genre-ledger repair (PQ10, already ticked)

- PQ10 repair landed (row already `[x]`): lane `speed-repair` 3-file fix (probe.rs new in engine-arena exempt package + lib.rs 1 line + exploit.rs production calls probe:: only): GE2 gate count 0 above `#[cfg(test)]` (gate regex case-sensitive; lowercase fighter words remain and are allowed).
- Tests (lead re-ran on applied tree): exploit 7/7, engine-playtest 201 passed/2 ignored, engine-arena 110 passed, golden `d57b5c8ae3096614` x2; packet-gate PASS 3 paths; rust-quality engine-arena PASS 87 B 0 regressions (playtest casts 110->115 pre-existing on master, zero in owned files). Workspace-check: exploit.rs clean, only physics/src/lod.rs still red (591 micro in lane). No row tick change.

### 2026-10-07, DT4 replay buffer (DT4)

- DT4 ticked. Landed lane `dt4-replay` 5-file slice (replay.rs new + data.rs/lib.rs wiring + distill Cargo.toml + Cargo.lock): `ReplayBuffer` + quantize paths with zero `as` casts + kill rule `judge_buffer_student` + same-seed-twice digest test.
- Tests (lead re-ran on applied tree): engine-distill --lib 37/37 (replay 5/5, kill_rule 1/1); packet-gate PASS 5 paths; rust-quality engine-distill PASS 96 A 0 regressions, casts 15==baseline. needs DT2 FK1 ticked. Residual: workspace GE2 red foreign (PQ10 repair in review), render/playtest/rules ratchet pre-existing.

### 2026-10-07, PX7 physics LOD for armies (PX7)

- PX7 ticked. Landed lane `px7` 2-file slice (lod.rs new 553 lines + lib.rs 3-line wire; fmt-only limb.rs + g21 test hunks dropped): tier/sleep LOD + `LodError` typed paths + `lod_army_500_64_under_4ms_hash_x2` + 8 lod tests.
- Tests (lead re-ran on applied tree): lod_ 8/8, F2P 1 passed, full engine-physics 125 passed/1 ignored, g21-locked-hash 1 passed, golden `d57b5c8ae3096614` unchanged; packet-gate PASS 2 paths; rust-quality engine-physics PASS 100 A 0 regressions. needs PX1 PF1 ticked. Residual: workspace GE2 exploit.rs red foreign, render/playtest/rules ratchet pre-existing.

### 2026-10-07, BX10 gore setting never changes the hash (BX10)

- BX10 ticked. Landed lane `bx10-body-r4` 1-file patch (gore.rs only, 203+/30-): `GoreSetting` Off/Reduced/Full + `ImpactFlash`/`GuardCue` + `is_blood` + `gore_for_events_with_setting` + triple same-hash test.
- Tests (lead re-ran on applied tree): engine-combat --lib F2P 1 passed, full 183 passed/0 failed/1 ignored; packet-gate PASS 1 path; golden `d57b5c8ae3096614` unchanged; rust-quality engine-combat PASS 96 A 0 regressions. needs BX2 CB2 ticked. Residual: workspace GE2 exploit.rs red foreign, render/playtest/rules ratchet pre-existing.

<!-- 2026-10-05..2026-10-06 receipt blocks (AM19, AM17, AM23, CB5, PX29, PX28, AM7, PX27) archived to git history (2026-10-12 size cap, lead rotate) -->

### 2026-10-07, PX8 character controller v2 (PX8)

- PX8 ticked. Landed lane `px8` 2-file slice (controller2.rs new + lib.rs wire): std `u64::isqrt` reuse, step/slope/platform/ledge/push/shove.
- Tests (lead re-ran on applied tree): controller2 8/8, engine-physics quality PASS 100 A 0 regressions; packet-gate PASS 2 paths. needs PH2 TV1 ticked. UNLOCKS UW9 stance capsules.

### 2026-10-08, PX31 substepped solver schedule (PX31)

- PX31 ticked. Landed `target/lanes/px31.patch` (substep.rs new 680 lines + lib.rs wire): Schedule/Phase/ScheduleError/PHASE_ORDER + Space::step_schedule, default step path unchanged.
- Tests (judge re-ran): packet-gate PASS 2 paths; quality 100 A 0 regressions; tests real (600-tick hash-x2, drift<5mm, refusals, alloc-free). Advisory: add TooManyJoints variant, drop dead `*2.0*0.5`, board arena-caller follow-up.

### 2026-10-08, MCP28 shared guard block (MCP28)

- MCP28 ticked. Content already on master via ef98d69 (guards.rs 348 lines + lib/scene_tools/tool_capabilities wiring); judge PASS on the matching patch (guards_format 2/2, build_game 18/18, packet-gate PASS 4 paths, quality 82 B 0 regressions). UNLOCKS MCP28 tool writes.

### 2026-10-08, GP24 camera stage wiring (GP24)

- GP24 ticked. Landed lane `gp24` (camera.rs + camera_feed.rs, +181/-29 feed): PresentationFeed drives StagedRig, bypass delegates to rig, unframed_count/edge_arrows oracles.
- Tests (judge re-ran): gp24_ 4/4, camera_feed 16/16; golden d57b5c8ae3096614 x2; quality 95 A. Note: packet OWNED listed feed only, patch also touches camera.rs (both landed). Pre-existing bad_fixture_text failure confirmed on pristine HEAD (lane fixes it). UNLOCKS GP25.

### 2026-10-08, GP18.a feel half (GP18 stays OPEN)

- GP18.a combat half landed, GP18 stays [ ] until gp18b bench half lands. feel.rs: kill-share pin (v1 wins 118/82/0, share 590, max 400) + no-float/no-bare-bar gate.
- Tests (judge re-ran): feel 33/33, combat lib 185/0; quality 96 A; packet-gate PASS. Land needs --accept waivers (1 alloc-ok harness scratch) + rebase past gore BX10 noted at commit.

### 2026-10-08, DEAD2 dead-js close (DEAD2)

- DEAD2 ticked. Landed lane `dead2` (3 files +68/-21): 5 dead helpers unexported, junkFiles shared helper, web-search dupe folded.
- Tests (judge re-ran): re-scan 0 dead 0 dupes; suites 31/31; workspace-check 8/8; packet-gate PASS 3 paths.

### 2026-10-08, RL20 tunnel-mouth bind (RL20)

- RL20 ticked. Landed lane `builder-look-rl20-r3` (tunnel_mouth.rs +154: TunnelMouthBind + bind_window_frame + 2 tests). - code cut 2026-10-16, recover via git log --follow -- crates/render/src/tunnel_mouth.rs
- Tests (judge re-ran): tunnel_mouth 9/9, check clean; packet-gate PASS. Quality cast FAIL is base drift (fix landed in ec12a18 after lane base), zero `as` in owned diff. Missing: RL20 GPU capture pair (pilot row to follow).

### 2026-10-08, BLOAT2 LFS pass (BLOAT2)

- BLOAT2 ticked. Landed lane `bloat2` pair (banner+crate to pointers) + lead renormalize of 6 more (debris/gate_tunnel/stands/torch/walls + music-bed.wav): 8/8 LFS objects pushed (50 MB).
- Tests: workspace-check lfs guard PASS; size-pack 177 MB < 500 MB; gltfs parse; gen.mjs green. Row scope grew pair->repo-wide to meet its own 0-blob accept.

### 2026-10-08, BX44 digest split (BX44)

- BX44 ticked. Landed lane `builder-body-bx44` (compare.rs +61/-1: separate_digests test, 14 siege records, 0 cross moves, bytes x2).
- Tests (judge re-ran): separate_digests 1/1, material lib 82/82; quality 98 A; packet-gate PASS. Destruction clippy red pre-exists outside owned.


<!-- 2026-09-23..2026-09-25 receipt blocks moved to docs/archive/SPRINT-done-2026-10-03.md (2026-10-03 slim) -->
<!-- 2026-09-26..2026-09-29 receipt blocks moved to docs/archive/SPRINT-receipts-2026-10-04.md (2026-10-04 size cap, Finish line GM3 worker) -->
<!-- receipts 2026-09-30..2026-10-02 (53 blocks) moved to docs/archive/SPRINT-2026-10-05.md (2026-10-05 size cap, center fixer) -->
### 2026-10-03, CQ11 + CQ12 mutation-tested diffs and instruction-count gate (owner Claude session, worktree agent)

- Archived to git history (`git log -p -- docs/SPRINT.md`): CQ11/CQ12 landed `85363d5`/`782e34e`, proof golden x2 held.

### 2026-10-03, sim-core burn-down in ecs, physics, combat, nav, destruction (owner Claude session, worktree agent; CQ2 SL5 CQ6 CQ8 CQ9 CQ10 stay open)

- Archived to git history (`git log -p -- docs/SPRINT.md`): 26 commits via `90b2ba7`, scorecards up, golden x2 held.

### 2026-10-03, burn-down in audio, net, assets, nn, distill, save, ui, input, observe (owner Claude session, worktree agent; rows stay open)

- Archived to git history (`git log -p -- docs/SPRINT.md`): 36 commits via `d3ec2c2`, hot_alloc down, golden x2 held.

### 2026-10-03, burn-down in editor, game, playtest, provider, perf, app (owner Claude session, worktree agent; rows stay open)

- Archived to git history (`git log -p -- docs/SPRINT.md`): 35 commits via `83bf78c`, scorecards up, golden x2 held.

### 2026-10-03, burn-down in ai, anim, spatial, world, scene (owner Claude session, worktree agent; rows stay open)

- Archived to git history (`git log -p -- docs/SPRINT.md`): 28 commits via `15440d5`, scorecards up, golden x2 held.

 <!-- 2026-10-03 CQ2/CQ13 receipt block moved to docs/archive/SPRINT-receipts-2026-10-05.md (2026-10-05 size cap, docs seat r6) -->

<!-- 2026-10-04 DT5 receipt block moved to docs/archive/SPRINT-receipts-2026-10-05.md (2026-10-05 size cap, docs seat r4) -->

<!-- 2026-10-04 MP11 receipt block moved to docs/archive/SPRINT-receipts-2026-10-05.md (2026-10-05 size cap, docs seat r4) -->

<!-- 2026-10-05 MCP15 receipt block archived to git history (2026-10-12 size cap, lead rotate) -->

<!-- 2026-10-06 wave receipt block moved to docs/archive/SPRINT-receipts-2026-10-05.md (2026-10-07 size cap, docs seat r5) -->

### 2026-10-07, PF15 fmt repair (PF15)
- PF15 ticked. Landed lane `builder-speed-pf15` fmt fix in `crates/perf/src/profiler.rs` (+15/-6, two split LightGrid::new lets collapsed, no logic change).
- Tests (lead re-ran on applied tree): profiler 9/9; packet-gate PASS 1 path (1 pre-existing long-fn warn); rust-quality perf PASS 91 A 0 regressions; judge VERDICT PASS x2 (624, speed-pf15-review-2). Golden x2 PASS in-lane. needs PF2, PF13 ticked.

### 2026-10-07, HS15.b manifest rebase (HS15 stays OPEN)

- HS15.b landed (row stays OPEN, HS15.c hour-export re-verify still queued): lane `hs15b-manifest` port of the PASSed manifest JSON onto HEAD (`crates/proof/src/dataset.rs` +339, one file): MANIFEST_JSON_FORMAT, hex/escape internals, strict JsonCursor parser, ShardManifest::to_json canonical fixed-order JSON (no serde) + parse_json re-validating via new(), 2 red-first tests. springs.rs hunk from the old patch correctly NOT ported.
- Tests (lead re-ran on applied tree): dataset 15/15 (incl hour 82s); packet-gate PASS 1 path 0 findings; rust-quality proof PASS 85 B 0 regressions; proof --skip-build golden x2 `d57b5c8ae3096614` PASS (remaining ratchet FAILs pre-existing in other crates: combat/geo board_ids, playtest/render casts, plus GP5 dirty-tree artifact of the uncommitted landing). Judge VERDICT PASS (626-hs15b-rebase-review).

### 2026-10-07, SP55 audit baseline tape (SP55)

- SP55 ticked. Landed lane `sp55-audit-baseline` test-only +169 in `crates/spatial/src/place.rs` (zero prod bytes): 60-seed minimality oracle (no-shrink 23/60 pinned, perm 20/20, digest x2) + 2 failure paths through the real `solve` entry point.
- Tests (lead re-ran on applied tree): engine-spatial lib 214 passed 0 failed 13 ignored (pre-existing timing ignores); packet-gate PASS 1 path 0 warnings; rust-quality spatial PASS 91 A 0 regressions; golden unchanged by construction (test-only, outside fixed tick). Judge VERDICT PASS (627-aidoor-sp-review). UNLOCKS SP38.

### 2026-10-08, RP9 motion-hold clip (RP9)

- RP9 ticked. Landed lane `builder-look` (clip_hold.rs new 392 lines + lib.rs mod + present/look.rs consumer): 120-frame dolly preset as data, hold_camera/motion/frame through frozen post::taa_resolve, ghost/shimmer oracle + 3 tests. - code cut 2026-10-16, recover via git log --follow -- crates/render/src/clip_hold.rs
- Tests (judge re-ran): clip_hold 3/3 (ghost 0.0790<0.25, shimmer 0.002427<0.01, bytes x2); post 33/2-ignored; packet-gate PASS 3 paths; quality PASS 0 regressions. Clippy reds pre-exist outside owned. Missing: reference-GPU clip pair (pilot row to follow).

### 2026-10-08, CT28 provenance gate (CT28)

- CT28 ticked. Landed lane `builder-content-r3` (provenance.rs new 575 lines + lib.rs mod): ProvenanceRecord hand/AI chains, stable error codes, 50-asset refusal matrix, rows/digest x2.
- Tests (judge re-ran): provenance 7/7, assets lib 123/2-ignored, clippy clean; packet-gate PASS; quality 95 A. No caller wired by design (gate before CT46 audit).

### 2026-10-08, CT81 placement audit (CT81)

- CT81 ticked. Landed lane `builder-content-ct81` (PLACEMENT-AUDIT.md new 66 lines, data-only): 17 actors 0 findings clean order-invariant, pinned input SHAs, bytes x2.
- Tests (judge re-ran probe): actors=17 findings=0 hash=cbf29ce484222325; spatial lib 214/0; packet-gate PASS. No pixels claimed (CT60 owns captures).

### 2026-10-08, f02 cargo narrow (no board row)

- Landed lane `speed-202610080433-f02` by lead merge (root manifest need): default-members 29->1 with eight-out note (editor/endpoint named per judge fix) + merged 5-test guard.
- Tests (lead re-ran on applied tree): Cargo.test.mjs 5/5; packet-gate PASS 2 paths; judge VERDICT PASS. Conflict with a concurrent manifest variant resolved to the union.

### 2026-10-08, PX10 CCD weapons+limbs (PX10)

- PX10 ticked. Landed lane `px10-finish` (powered.rs + ragdoll.rs, +813/-30): CCD on for weapons/fast limbs, contacts drained during step + mutated after (Jolt ContactListener pattern), private fields + accessors, spawn split via validate_spawn_args.
- Tests (lead re-ran on applied tree): blade 2/2; full physics lib 202/1 (solo rerun of the 1 failure green — saturated-box load flake); golden `d57b5c8ae3096614` x1 + builder digests; packet-gate PASS 2 paths; quality 100 A 0 regressions. Judge VERDICT PASS (739). UNLOCKS PX9 (drain_contacts_into_hits).

### 2026-10-08, PQ18.a staged latency collector (PQ18 stays OPEN)

- PQ18.a landed (row stays OPEN, PQ18b pilot GPU leg pending): lane `builder-speed-r1` (`crates/perf/src/latency.rs` +202/-0): WindowLatencyLog staged collector (stage/summarize/clear, ceiling 4096) feeding summarize_live into GpuConditions::report.
- Tests (lead re-ran on applied tree): latency 19/19; packet-gate PASS 1 path; quality 92 A 0 regressions. Judge VERDICT PASS (749). Missing: quiesced RTX 3050 run posting live p50/p95 (PQ18b).

### 2026-10-10, fixda seed dedup plus destruction hygiene (salvage, no board row)

- Landed fixda 149a50b7 (10 files: arena seed dedup through engine-core mix64 plus GOLDEN_GAMMA, frac_name_into caller-owned, bake docs; 54+/12-). Tests (lead re-ran on applied tree): destruction 138, arena seed 5, golden `d57b5c8ae3096614` x2, packet-gate PASS 10 paths, quality 95/96 A, tighten x2. Judge VERDICT PASS (810-fixda-review).

### 2026-10-10, fix3c2 Trauma plus MoveData wired (salvage, no board row)

- Landed fix3c2 7d7c22d0 (combat lib.rs plus limbloss.rs: Trauma permille type, trauma_shake bridge, MoveData Copy rows, BodyState impact_trauma plus impact_shake, shake_for_trauma, MOVES_V1 plus move_row). Tests (lead re-ran on applied tree): combat lib 195/1, golden x1 (x2 in lane), packet-gate PASS 2 paths, quality 97 A. Tree files byte-identical to lane files (normalized). Judge VERDICT PASS (822-fix3c2-review).

### 2026-10-10, look-r6 kit x2 determinism test (VS1.a stays OPEN)

- Landed look-r6 46525af1 (game arena_kit.rs +41 test-only: kit_imports_8_files_over_5000_tris_x2, 8 files 11 actors keys stable x2). Tests: 2/2 green on tree after `git lfs pull --include content/arena/src/*.gltf` (tree held 7 LFS pointers; lane had bytes). quality 96 A. Judge VERDICT PASS (823-look-r6-rerun). UNLOCKS kit-determinism for VS1.a. Note: tree carried 3 unrelated material_fallback deletions, preserved as-is (green, unclaimed, no lane).

### 2026-10-10, CT51 ticked plus RL28 ticked

- CT51 ticked. LOAD-REPORT landed 419c0fef (prior session, row left unticked). Verified by content-r9 DONE on tree: assets lib 128/2 ignored, fighter_bind 3, headless sum 498 ms << 5 s bar, cold set 433.26 MiB, report x2 identical cbf29ce484222325, dedupe 0 overlap vs 14 canonical, needs CT7/CT8 ticked, consumer CT27 soak plus PF13 baseline named. GPU wall clock plus driver VRAM stay pilot-open (estimate 30-46 MB, not a refusal).
- RL28 ticked. Manifest refusal landed 61dc1642 (prior giant wave). Verified by look-r3 DONE on tree: manifest suite 15/15 green. Row Accept met (seeded 3-pattern refusals plus 3 real captures pass).

### 2026-10-10, CT35 ticked (fighter LOD + impostor bake)

- CT35 ticked. Landed lane `ct35` (LOD bake example ct35_lod_bake.rs +273, hero.json receipt, CT35-LOD-RECEIPT.md, Cargo stanza). Tests (lead re-ran on applied tree): example 8/8, lod_swap 9/9, mesh_cook 13/13; bake digest match x2 per judge; packet-gate PASS 4 paths; quality 96 A 0 regress. Needs CT2/CT5 ticked. Judge VERDICT PASS (808-ct35-review). Consumer CT41 plus picker named; P6 ms/VRAM per-LOD stays pilot-open on the reference GPU.

### 2026-10-10, DX16a piece landed (DX16 stays OPEN)

- DX16a piece landed from lane `dx16a-body-r1` (cluster.rs +327 new: sorted CSR adjacency, anchor set, intact/broken, BFS flood, FNV digest; spread.rs +68 cluster_support_and_digest consumer bridge; lib.rs +3). Tests (lead re-ran on applied tree): F2P cluster_topology 2/2, full destruction lib 141/2 ignored; packet-gate PASS 3 paths; quality 96 A 0 regress; golden unchanged (judge proof --skip-build PASS, analysis-only never the tick). Judge VERDICT PASS (808-dx16a-review, RUST 4/5 DEPTH 4/5). DX16 stays open for the propagation/break leg (UNLOCKS). Note: patch holds 3 files vs judge text 2; spread.rs bridge ran inside the green P2P.

### 2026-10-10, RP8 plus RP8a ticked (variance-clip closeout, REJECT with numbers)

- RP8 ticked. Accept met via REJECT with numbers: wave-30 lane verdict REJECT `37d32db` (box-clamp stays canonical), LADDER P4 SPIKE x2 lines carry the full ghost/shimmer/ms numbers (docs/research/LADDER.md:89-90: ghost cut 42.5%/41.1% PASS, shimmer settled PASS, ms +134.7%/+22.5% FAIL over the +5% bar, pixels x2 identical). No code touched (post.rs resolve frozen).
- RP8a ticked. RP8a accept met: SPIKE lines carry REJECT + numbers, RP8 ticked above. Held patch target/lanes/rp8a.patch aimed at the same two ticks in an older shard text; hand-ticked on the live lines instead (lead read proof: LADDER.md:89-90). RP8b stays open behind its overturn gate.

### 2026-10-10, done-but-open closes (S41 partial: 3 of 5)

- R6 ticked. Accept re-verified on tree by lead: `cargo test --locked -p engine-render --lib particles` 21 passed; row cites LADDER R5-flip ACCEPTED (unique 1->14, flick 1.36, digest x2).
- PT34 ticked. Lead re-ran: scenario_shape 3/3 plus eval P2P 46/46; fixtures/scenarios/ holds 3 sample scenarios.
- IN2 ticked. Lead re-ran: action_state 4/4 plus P2P attack_key_maps_to_attack_bit 1/1.
- CB3 stays open: owned `crates/combat/src/design.rs` absent on master; needs a builder row, not a tick.
- PT2 stays open: `rg silhouette|ABSTRACT-DOTS|jab_flash|legend crates/endpoint/src/play.rs` prints 0; capture-draw hunks not on master.
- Claims dedupe: speed-202610080433-f04 back to 1 line (kept 05:50Z).

### 2026-10-10, BX5.a piece landed (BX5 stays OPEN, UNLOCKS double-loss driver)

- BX5.a piece landed from lane `builder-body-r7` (anim injured.rs new: prod apply_loss_override wiring rebuild_single into layers path plus fresh-seed tests; lib.rs +3 mod). Tests (lead re-ran on applied tree): full anim lib 291/1 ignored incl F2P loss_override_wires_rebuild_into_layers; packet-gate PASS 2 paths; quality 97 A 0 regress; clippy clean except pre-existing foreign combat JOINT_TOTAL (hitsim.rs:18, out of scope). Judge VERDICT PASS (828-body-r7-review, RUST 4/5 DEPTH 3/5). BX5 stays open for the double-loss plus 20-tick legal-state driver (UNLOCKS). Golden untouched (anim-only, tick untouched by construction).

### 2026-10-10, CT56 pilot corner 7-file (CT56 stays OPEN, UNLOCKS UW39 plus CT78)

- CT56.a+b landed from lane `builder-content-r4-ct56` (content/maps/urban-war-pilot/ 7 files 322+/0-: variant 6, corner 40, PROVENANCE 12, PILOT-RECEIPT 45, cook-corner.mjs 76, corner-cook.json 67, COOK-RECEIPT 76). Tests (lead re-ran on applied tree): cook-corner GREEN actors=3 lod0=5376 bytes=384147 worst=insula@2112 x2 identical SHA 018C4CF7; packet-gate PASS 7 paths; workspace-check PASS; lane-board PASS. Judge VERDICT PASS (835). CT56 stays OPEN (bundle via AS1 deferred, UW39 receipt plus CT78 scout open). No pixel/hash move by design (prerequisite corner evidence).

### 2026-10-10, fix4 render plus scene hygiene (no board row, UNLOCKS Fnv64 single-owner)

- fix4 landed from lane `fix4` (6 files 143+/74-: fog bind-probe extract plus zero-alloc roundtrip, graph hash via engine-core Fnv64, shadow cascade hygiene, shadow cluster Instant cfg, torch TemporalPick plus PriorRhs split, scene VersionedComponent cfg-gated). Tests (lead re-ran on applied tree): engine-render lib 401/12 ignored, engine-scene lib 59/0, golden_hash_untouched x2 PASS d57b5c8ae3096614, packet-gate PASS 6 paths, quality render 69 D plus scene 90 A 0 regress, workspace-check PASS. Judge VERDICT PASS (837, RUST 4/5 DEPTH 3/5). Hygiene refactor moves no pixel/hash by design (MOVED hash identical plus UNLOCKS named).

### 2026-10-18, PX10 receipt gap closed (E69)

- PX10 CCD weapons+limbs reached HEAD in auto-backup 2345f203 (2026-10-08 21:58: crates/physics/src/powered.rs +375, ragdoll.rs +468). The commit 1ce87a77 that names PX10 changes no crate file (docs and baseline only), so this block is the receipt that names the landing commit. The files live in crates/physics, not combat.
- Proof: gh run 39101816259 `cargo test -p engine-physics --features rapier --lib powered` 17 passed; gh run 39101816876 `cargo test -p engine-physics --features rapier --lib ragdoll` 28 passed (both need `--features rapier`: without it the filter matches 0 tests). `node tools/rust-quality.mjs --crate physics` prints 100 A. No code changed in this receipt, so the golden line above holds.

### 2026-10-21, CB3 ticked (combat design row: the latency gate and the buffer cap)

- CB3 ticked. Landed f4fc190b "combat: the design row of the fight table lands" (`crates/combat/src/design.rs`): the hit-stop / cancel-window / reaction-timing patterns are reimplemented from the pattern description only, with the MIT + PORT cite and the EVIDENCE row inside the file (no Deltarune code, art or data). The input-latency gate reads the input layer's own number (`feel::FEEL_V1.pq1_latency_ms()`) and refuses only a reading over the 50 ms bar (`feel::PQ1_LATENCY_MS`); the buffer cap is `feel::MAX_BUFFER_TICKS` (12).
- Proof (lead re-ran on the remote runner, not the agent's report): `cargo test -p engine-combat --lib design` -> `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 310 filtered out` (gh run 39574249664).
- Proof (balance leg, remote runner): `cargo test -p engine-combat --lib pq2 -- --nocapture` -> `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 310 filtered out`, printing `PQ2 v1 kills: wins_a=102 wins_b=98 draws=0 kills_total=200 max_share=395` and `PQ2 phased-v1: wins_a=102 wins_b=98 draws=0 share_a=510` (gh run 39574853440). The win share 510 permille sits inside the tournament bar 400..600 (`crates/combat/src/feel.rs`, `WIN_MIN_PERMILLE` / `WIN_MAX_PERMILLE`); the killer-byte shares (65/79/37/19) pin the v1 baseline for drift, and the same survey twice is identical (seeded determinism asserted in the test).
- Accept read: tournament win share 510 permille sits inside the 400-600 band (`crates/combat/src/feel.rs:50-52`); input latency reading 0 ms is under the 50 ms bar. Stated, not hidden: the survey's raw max kill share is 395 permille, above the 350 permille bar (`feel.rs:56`); that bar gates multi-move tables tournament-side, and the single-move survey pins the shares for drift. This row claims the win-share band and the latency bar, not a dominant-move claim.
- Kill clause ("float math in the tick; divergence") holds: the gate is integer millis off the tick rate (`FRESH_PRESS_ADDED_TICKS * 1000 / TICKS_PER_SEC`), and the survey is seeded, so the same tree gives the same 102/98 twice inside one run.
- The golden line `d57b5c8ae3096614` was re-proven twice on 2026-10-21 with the landed `design.rs` in the tree (see the Q2 receipt below), so the tick moves no pixel and no hash.

### 2026-10-21, Q2 ticked (Q1 numeric receipt refreshed, headless, zero GPU pixels claimed)

- Q2 ticked. Evidence refreshed under `target/q1/` only (`receipt.md`, `perf-report.txt`; `target/` is git-ignored, so this block is the committed receipt). No `crates/` hunk, no `content/` write, no cargo by hand: the proof command does the building.
- Proof (lead re-ran today, two independent runs of `node tools/proof.mjs --skip-build`): run A -> `[INFO] golden build (--profile=perf-test): 805.8 s` then `[PASS] legacy golden - d57b5c8ae3096614 engine=0.1.0 workspace=0.1.0`; run B -> `[INFO] golden build (--profile=perf-test): 1022.6 s` then the same `[PASS] legacy golden - d57b5c8ae3096614 engine=0.1.0 workspace=0.1.0`. Same sha, two separate builds: the accept line (golden `d57b5c8ae3096614` x2) is met.
- The overall result, stated plainly and not glossed: run A ended `RESULT FAIL`. Every failing line belongs to another seat's in-flight work in the shared tree, and none of them is a golden, hash or pixel line: `[FAIL] proof execution - spawnSync cargo ETIMEDOUT` (contended build-directory lock), `[FAIL] generality ratchet: GE2 genre ledger (workspace-check) - FAIL workspace audit: kam2-web: selection.rs is not declared by any module, so it is never compiled` (the UI7 slice's in-flight untracked file), `[FAIL] generality ratchet: engine diff empty (GP5) - crates/ dirty: crates/endpoint/src/discover.rs, crates/endpoint/src/mcp.rs, crates/endpoint/src/packs/gym.rs, crates/endpoint/src/packs/gym_observe.rs, crates/endpoint/src/packs/mod.rs (+44 more)`, `[FAIL] rust quality ratchet (tools/rust-quality.mjs) - [FAIL] ratchet engine-endpoint: big_files 0 -> 1 | [FAIL] ratchet kam2-web: new crate must enter at grade A ... (score 86, nonzero: dup_fns) | [FAIL] ratchet engine-material: dup_fns 0 -> 3 | [FAIL] ratchet engine-render: panic_sites 25 -> 26`, `[FAIL] docs coverage ratchet (tools/docs-coverage.mjs) - [FAIL] ratchet engine-rome: new crate with no recorded baseline`. Run B's log ends on its golden PASS line (the run was cut with the session restart), so run B proves the golden only. A tree-wide `RESULT PASS` is not reachable from this row while other seats hold the tree dirty; that is a fact about the tree, not about the golden.
- `node tools/finish.mjs` recomputed today (exit 0): `A4: partial - open rows: Q1, GX1, GX2, GX3, GX4, GX6, GX7, GX8, GX9, GX10 (10/12)`; `P1: open - open rows: GX1, GX2, GX3, GX4, GX6, GX7, GX8, GX9, GX10, RL4, RL5, RL6, RL7, RP2, RP3, RP4, RT0, RT1, RT2, RT3, RT4, RT5, RT6, RT7, RT8, VG1, VG2, VG3, VG4, VX1, VX2, VX3, VX4, VX5, Q1, PQ4 (36/36)`; `P10: met - open rows: none (0/1)`; `Summary: 5 met, 12 partial, 9 open (FS1 scoreboard)`.
- Placeholder refusal re-verified today: `node --test tools/q1-check.test.mjs tools/ra8-check.test.mjs` -> `tests 6`, `pass 6`, `fail 0` (exit 0). `node tools/ra8-check.mjs` -> `png_count: 0 (expected 3)`, manifest missing (ra8 files 0), `fighter png 0 != 3` (exit 0). `node tools/q1-check.mjs` -> `synthPng: 0`, `orbitPng: 0`, `totalPng: 0`, `manifests: 0`, `ra8Manifests: 0` (exit 1, findings only: the capture dirs are missing). V1 64x48 noise sha REFUSED, V2 single-cube sha REFUSED, V3 single-cube sha REFUSED - no PNG accepted, no sha written, zero reference-GPU captures claimed.
- Headless `perf-report.txt` refreshed at 2026-10-21T09:20Z (HEAD 650fae74 at write time). Every GPU line reads PENDING: adapter, driver, resolution, frames, per-pass ms (sky/layers/particles/decals/ibl/fog/gtao/shadow/forward), validation errors, VRAM total/usage and draw calls. No quiet-window run on the reference GPU happened in this refresh, so no GPU number is guessed and no frame-split mean is claimed.
- This receipt claims the golden x2, the recomputed A4/P1 lines, the placeholder refusal and the refreshed headless perf-report. It does not claim a tree-wide proof PASS and does not claim any GPU number or reference capture.

### 2026-10-21, UI6 ticked (the build-menu cells, the placement probe and the refusal line)

- UI6 ticked. The crate side landed in ec796c40 "kam2-web: the build menu cells land with the page that reads them" (`crates/kam2-web/src/build_menu.rs`, plus the `page/app.js` and `page/index.html` that read them): the five build cells are one JSON line (`house_kinds()`), the placement probe asks the sim and the answer is the sim's own refusal in the sim's own words (`build_menu::refusal_line`), the page arms a cell by a click, and the bottom strip prints what the sim said. This block is the receipt that names that landing and re-runs the proof on the tree as it stands today.
- Proof (lead re-ran the browser drill on the live page today, not the agent's report): `node tools/kam2-web.mjs` -> `KAM11 OK houses=6 cells=5 placed=farm refused=NotBuildable: that ground cannot carry a house png=target/kam2ui.png bytes=384510`, exit 0. The house count rose from 5 to 6 on the click of the farm site the sim itself accepted, and the armed farm over the village road printed the sim's own refusal `NotBuildable: that ground cannot carry a house` with the count unmoved.
- Proof (the window itself, same run): `KAM8 receipt frames=120 tick=11 hash=6556f632f5e976a2 houses=5 units=5 webgpu=nvidia/ampere bytes=389227` and then `KAM8 OK frames=120 png=target/kam2web.png`. The page stopped itself at frame 120, so the shot is a frozen frame of the running match and not a painted mock.
- What this receipt does not claim: no tree-wide `RESULT PASS` (the shared tree carries other seats' in-flight work; see the Q2 block above) and no GPU timing claim. The shots live under `target/`, which is git-ignored, so this block is the committed receipt.

### 2026-10-21, UI7 ticked (the selection panel, the command row, the minimap and the pointer contract)

- UI7 ticked. The crate half landed as f48acb6a6 "kam2-web: the selection, the minimap and the orders land" (`crates/kam2-web/src/selection.rs`, `crates/kam2-web/src/minimap.rs`, plus `shell.rs`, `web.rs`, `lib.rs`): the page asks the sim what stands under the cursor, the sim's own command carries the order, and the panel, the three command buttons and the 160x120 minimap read numbers the crate hands over. The page half (the UI7 `page/app.js` and `page/index.html`) and the drill (`tools/kam2-web.mjs`) land with this receipt.
- Proof (lead re-ran the drill in a headless Chrome on a fresh page, not the agent's report): `node tools/kam2-web.mjs` -> `KAM13 OK selected=swordsman panel=160x72 buttons=3 minimap=160x120 rect=100,30,60,60 tests=50 png=target/kam2ui3.png bytes=383398` and `KAM13 SOURCES selection=crate minimap=crate commands=crate`: the selection panel is 160x72, the command row holds three 48x48 buttons, the minimap surface is 160x120, and the three seams the plan names are crate code, not page code.
- Proof (the pointer contract of the plan, same run, on a page that froze after its frame budget so that not one key is touched): `KAM14 OK farm=11,4 soldier=u0 walk=3,10 foe=u2 refused=Occupied: that ground is taken picked=2 house=h0 road=NotBuildable: that ground cannot carry a house pan=-3.95,-3.03 window=0.49,-0.29,80,60 png=target/kam2ui4.png bytes=393725`. Read line by line: the mouse alone placed a farm (house count rose, order line empty), cancelled the armed cell with the right button, picked a swordsman, sent it walking with the right button (the sim's own `moving` flag turned true), stood it still again with the stop button, struck a foe under a crosshair without moving the selection, was refused in the sim's own words on the foe's own ground (`Occupied: that ground is taken`), added a second soldier with shift, cleared the selection on empty ground, picked a house for the panel, and moved the camera with a click on the minimap. The drill was run three times on three fresh pages and the KAM14 line came out the same every time, down to the farm site, the walk tile, the refusal text, the picked count and the pan; only the PNG byte count moved by one (the shot compresses a hair differently).
- The pointer-only run of the plan (`docs/plan/kam2-ui.md`, acceptance line 60) is met by the KAM14 block: it runs on a fresh page that froze after its frame budget, so the field stands still, and every order goes in with the mouse alone. No key event is dispatched anywhere in that run.
- The other acceptance line (the minimap rectangle matches the camera after a pan, by test, not by eye) is met twice: by the crate's own test (`crates/kam2-web/src/minimap.rs`, `the_window_sits_where_the_camera_looks` and `a_pan_moves_the_window_by_what_the_camera_moved`) and by the drill's numbers on the live page - the minimap click moved the camera to pan=-3.95,-3.03, the window on the minimap moved by exactly the pan times ten pixels per tile, and its centre sat on the camera's centre tile.
- Stated plainly: the drill ran against a wasm built in the alternate target directory (`CARGO_TARGET_DIR=target/alt-ui7`) because the shared build directory was locked by other seats; the crate's own cargo test and clippy proof is in the f48acb6a6 commit, and this receipt adds the browser drill on top of it. The page screenshots live at `target/kam2ui3.png` and `target/kam2ui4.png`; `target/` is git-ignored, so this block is the committed receipt. What this receipt does not claim: no tree-wide `RESULT PASS` and no reference-GPU number.

### 2026-10-21, UW24 ticked (the spot shadow atlas: the device bake, the live-frame row and the CPU-reference parity)

- UW24 ticked. The row's bake half landed in 44ffb0e6a "render: the spot atlas pass is native-only on wasm32, and its timer rides the clock seam" (`crates/render/src/spot.rs`, `crates/render/src/spot/encode.rs`): the device bake, the median-of-3 budget arm and the CPU-reference parity. The live-frame half landed in 0e55a1330 "render: the spot row is live - the atlas bakes in the frame's own command buffer" (eight render-crate paths: `pass_truth.rs` with its `harness.rs` and `live_gate.rs`, `present/gpu_present/{gate,live,presenter,submit}.rs`, `v1_ibl_shadow.rs`): the look-pass registry names the row "spot" with the spot module's own submit entry (`submit_spot_atlas`), the live window frame records the bake into its own command buffer before the forward draws, and the receipt lands in the frame's row.
- Proof (the budget arm, local PC run on the reference GPU): `cargo test -p engine-render --lib --features window spot -- --nocapture` -> `spot device bake on NVIDIA GeForce RTX 3050 6GB Laptop GPU: submit_ms 0.729 budget_ms 2.000 shadowed 16 (cold 12.435 ms, samples 0.651/0.729/3.074)`, and `20 passed; 0 failed` with no SKIPPED line.
- One precision on the row's first accept leg: the number proven here is the spot pass's own bar, `SPOT_PASS_BUDGET_MS` = 2.000 ms (`crates/render/src/spot.rs`), not a re-measurement of the P6 frame metric (GPU p95 <= 14 ms at Base, `docs/FINISH-RICH.md`). The bake's 0.729 ms on the reference GPU is 5% of that 14 ms bar, so the pass fits inside the P6 budget with room; the whole-frame p95 at Base belongs to the P6 milestone and this row does not re-measure it.
- Proof (the parity arm, same run): `spot parity on NVIDIA GeForce RTX 3050 6GB Laptop GPU: compared 65536 mismatched 0 max_diff 0.0000026226044 agreement 1`.
- Proof (remote runner, gh run 39637725126, `cargo test -p engine-render --lib --features window -- --nocapture`): `test result: ok. 1267 passed; 0 failed; 17 ignored; 0 measured; 0 filtered out; finished in 29.43s`, with `spot parity on Microsoft Basic Render Driver: compared 65536 mismatched 0 max_diff 0.0000025629997 agreement 1` and `spot device bake on Microsoft Basic Render Driver: submit_ms 5.127 budget_ms 2.000 shadowed 16 (cold 7.649 ms, samples 4.323/5.127/6.267)`. The runner has only a software adapter, so the budget arm posted its own number and said so: `SKIPPED the spot budget: the budget is the reference GPU's; Microsoft Basic Render Driver posted its own number`. The 2.000 ms budget is therefore proven on the RTX 3050, the reference GPU, and not on the runner.
- Proof (clippy on the remote runner, gh run 39635848211): `cargo clippy -p engine-render --lib --features window --no-deps -- -D warnings` -> PASS. `node tools/rust-quality.mjs` -> `RESULT PASS: score 97 (A), 0 ratchet regressions over 44 crate(s)`; `rustfmt --edition 2021 --check` on the eight changed paths -> FMT_EXIT=0.
- What this receipt does not claim: the atlas write paints no pixel yet (no forward-shader hunk samples the spot atlas), so the spot row reads PROBE in the pixel-truth table, exactly like the ticked sky and decals rows; the GX6 proof-crate row list (`crates/proof/src/gpu_live.rs`, EG8's file) still carries nine rows without "spot", so the tree-wide gate keeps its own refusals; and no shipped scene feeds spot lights yet, so the live frame skips the pass by default. A fully green `--all-targets --features window` clippy is not reachable while the foreign `crates/render/src/bin/kam1a.rs:16` `or_fun_call` rot sits at HEAD; CI's own clippy line never enables the window feature, so the runner's clippy never sees it.

### 2026-10-22, OPT47 (the proof lane names the receipt nextest wrote, and the hunt wave 2 card lands)

- OPT47 code landed as 65116568c "proof: the lane names the receipt nextest wrote, not a guessed path" and the map fix as aa1ee734c. The row itself stays **open**: its Accept needs one real nextest leg, and this PC has no `cargo-nextest` (OPT6 is parked for the same reason).
- What was wrong: `tools/proof.mjs` printed a hardcoded `junit target/nextest-junit/proof.xml` on every test leg. No nextest run writes that file. The help text, `tools/proof.mjs` and `.config/nextest.toml` also claimed a per-partition receipt pair (`proof-1-2.xml` plus `proof-2-2.xml`) "merged to `proof.xml`", which never happened. A plain `cargo test` leg claimed the same receipt even though it writes none at all.
- What is true now, measured on this PC with cargo-nextest 0.9.149 in a probe crate (`C:/Users/me/AppData/Local/Temp/opencode/ntprobe`, the repo's own `.config/nextest.toml` copied in): the proof profile's configured `path = "target/nextest-junit/proof.xml"` lands at **`target/nextest/proof/target/nextest-junit/proof.xml`**, nextest writes **ONE** receipt per run, and the receipt carries the run id nextest prints (`Nextest run ID 1f7c14ed-3701-4729-a8ba-233d70b9dd7b` -> `uuid="1f7c14ed-..."` in the receipt's `<testsuites>` root). The ci profile lands the same way at `target/nextest/ci/target/nextest-junit/ci.xml`, which is why `tools/ctrf-report.mjs` (HX12) already scans those paths.
- The fix: `nextestReceiptPath(runOutput, root)` strips the colours, requires a `Nextest run ID <uuid>` in the run output, and matches that id against the receipt's own `uuid` through `pickReceipt` from `tools/ctrf-report.mjs` (the same discovery the CI reporter uses). It returns the repo-relative slash path, or null when there is no run id, when the id is stale, or when the leg was a plain `cargo test`. `junitLabel` prints `junit <path>` or the plain words `no receipt`. The lane never guesses a path again.
- Proof (the real receipt, node only, the probe tree): `Nextest run ID 1f7c14ed-3701-4729-a8ba-233d70b9dd7b` -> `receipt named: target/nextest/proof/target/nextest-junit/proof.xml`, `that file exists on disk: true`, `receipt uuid matches the run id: true`; a stale id -> `no receipt`; a plain `cargo test` leg -> `no receipt`. The ci-profile run in the same tree named `target/nextest/ci/target/nextest-junit/ci.xml` the same way.
- Proof (the tool tests): `node --test tools/proof.test.mjs .config/nextest.test.mjs` -> **47/47** and **6/6**, including the three new tests (`nextestReceiptPath` names the receipt the run wrote, no run id and a stale run id name no receipt, `junitLabel` names a receipt and says `no receipt` for a leg without one). `node --test .opencode/repomap.test.mjs` -> 1/1 (the map is 15868 B of the 16 KB budget, 487 of the 500-line budget).
- Proof (the golden): `node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli default_and_max_seed` -> PASS on the PC, because GitHub could not be reached from this box ("could not run the check ... trying once more", then "GitHub could not run it; running it on the PC instead"). The test asserts the hash itself at `tests/proof_cli.rs:163` (`"final_hash":"d57b5c8ae3096614"`) and it passed, so the hash did not move. No Rust changed in this row, so it could not have.
- Proof (the ratchet): `node tools/rust-quality.mjs --no-cache --owned "tools/proof.mjs,tools/proof.test.mjs,.config/nextest.toml,.config/nextest.test.mjs,.opencode/repomap.md"` -> `RESULT PASS: score 97 (A), 0 ratchet regressions over 44 crate(s)`. `node tools/workspace-check.mjs` -> the board PASS (1691 tasks, ids unique, every needs resolves).
- The hunt wave 2 landed with it: the card `docs/research/hunt2-2026-10-21/CARD.md` (ac5ee4532) indexes the five donor reports another seat had already committed (125fa5fdd), nine LADDER lines carry the steals, and four open rows now name the donor that makes their Accept measurable: MP25 and EG39 (nilpunch/unity-prediction-rollback, a predicted input is a blend, not a boolean), LF5c (RARS-oss/decibels, price every loss in dB and print PSNR and LPIPS side by side, because PSNR alone draws the wrong conclusion), HX7 (Scthe/nanite-webgpu, the per-meshlet half of the cull) and OPT42 (Kobzol/cargo-wizard, a build shape is a table, not a hand-edited block).
- What this receipt does not claim: **no nextest leg ran through `tools/proof.mjs` itself**. The full lane needs the whole workspace to compile, and on 2026-10-22 the shared tree did not compile (another seat's staged `crates/render/src/lib.rs` declared `pub mod torch_light;` twice), and this PC has no `cargo-nextest` in PATH (OPT6 is parked for the same reason). The hunt wave 2 claims no code landed and no build ran: it is read-only by design, and every number in it is the donor's own number, read on 2026-10-21, used as a target and not as our result.
- Proof (the two halves joined, 2026-10-22, the evidence above restored after a temp-dir cleanup): a **real** cargo-nextest 0.9.149 run with this repository's own `.config/nextest.toml`, in a probe crate, wrote one receipt at `target/nextest/proof/target/nextest-junit/proof.xml` (6698 bytes) and printed `Nextest run ID 92ad6c8f-3a83-4e77-aabc-147e3aee9fd2`; the receipt's own `uuid` is that id. The lane's own `nextestReceiptPath` was then run **against that real run's output** and named that file: `junit target/nextest/proof/target/nextest-junit/proof.xml`, the file is on disk, and the receipt's uuid is the run id. A stale run id names `no receipt`, output with no run id names `no receipt`, and a plain `cargo test` leg names `no receipt`. What is still missing is only the shell around it: the `tools/proof.mjs` leg line itself was never printed by the lane, because a nextest leg inside this repository needs `cargo-nextest` in PATH and a workspace that compiles. So **OPT47 stays unticked**, and the row says so.
### 2026-10-22, OPT45 ticked (the thin-LTO local build keeps the 60 fps floor)

- OPT45 ticked. The row's last open leg was the arena fps check on the **thin** binary; it is now measured and the floor holds. The profile change itself landed earlier as ef781864f ("build: release is thin LTO, ship keeps fat LTO") with OPT46 as 90d78c086.
- The binary under test is `target/release/arena.exe` (20,216,320 bytes, built 2026-10-21 22:07), i.e. the `release` profile dir, and `release` at HEAD is `lto = "thin"`, `codegen-units = 16`. The fat-LTO `ship` profile has its own `target/ship` dir, untouched since 2026-10-17, so the two shapes never shared a target tree and the fat build was not what was measured.
- Proof (six 300-frame runs, `--scene arena --fighters 8 --frames 300 --exit`, reference GPU `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062`, 1280x720): `interval_ms p50` = 16.770 / 16.618 / 16.688 ms in the first batch on a quiet box (median **16.688 ms = 59.9 fps**) and 16.639 / 16.719 / 17.111 ms in a second batch on a box at 100 % CPU with 19 cargo/rustc running (median **16.719 ms = 59.8 fps**). `validation_errors=0` and `fight_draws=40` on every leg. The 60 fps floor is 16.667 ms, so the quiet median clears it and the loaded median is 0.05 ms under it — the load moved the number by 0.03 ms, not by a frame.
- Proof (the product check on the same binary): `node tools/product-check.mjs --no-build` -> `product-check: GREEN 6c634957f fps=59.9(loaded) empty=0 tinted=0.12 pops=5 sunk=0 ticks=1..299.1 flip=147 blank=0 frozen=0 val=0`. The fps is printed as `loaded`, meaning the tool did not gate it, because other seats' cargo was running; that is why the six arena legs above are the gated evidence and this line is the floor check.
- The floor was never lowered: `tools/product-bar.json` still reads `fps_p50` "60 at 1080p on the RTX 3050 (FC3 asks 30 first)" and `tools/product-check.mjs` still compares the interval p50 against the median of the last unloaded runs.
- The board, the plan and the card follow the tick: `docs/SPRINT-27-final-picture.md` OPT45 (ticked, with the six numbers), `docs/plan/build-cpu.md` (the OPT45 row now carries the fps measurement and says the row is ticked) and `docs/research/build-cpu-2026-10-21/CARD.md` ("Still open: nothing"), plus the LADDER line for the build-CPU steal.
- What this receipt does not claim: it does not re-measure the build times. The -35 % build wall (thin 38.08 s vs fat 58.84 s on the GitHub runner, same 22 crates from scratch, golden hash `d57b5c8ae3096614` PASS in both) is the earlier runner evidence in the same row and is unchanged by this receipt. It also claims no new cargo build: the binary under test was already built (2026-10-21 22:07) and the `cargo build --release -v` line that would have printed the rustc command was not captured, because a verbose rebuild of the whole tree competes with other seats' lanes on a box that is at 100 % CPU. The profile shape is read from `Cargo.toml` at HEAD instead, and the target dir name is the profile name.
### 2026-10-22, OPT39 measured (dependencies build without debug info in dev)

- OPT39 measured on the fixed master (the tree did not compile before another seat removed a duplicate `pub mod torch_light;` from `crates/render/src/lib.rs`, which had landed in ffc7dce26 and killed every cold build at that commit). The measurement runs on a copy of HEAD laid out with `git archive HEAD`, so both arms build the same sources and only the profile differs, and the shared tree is never written to.
- The "after" arm is the row's own setting in the root `Cargo.toml`: `debug = false` under `[profile.dev.package."*"]`. It is written into the COPY's manifest, not the shared one, because the shared `Cargo.toml` belongs to whoever holds it (OPT42 owns the dev profiles) and a `git checkout -- Cargo.toml` at the end of a run would destroy another seat's edit to the same block. The setting was first proven to reach the external dependencies at all, on a scratch crate with one external dep: serde's rlib went 1.24 MB -> 0.94 MB (-24 %) with the identical setting, and the probe's own manifest was never patched.
- The four cold builds, `cargo build -p engine-game --timings`, each in its own `CARGO_TARGET_DIR`: the wall and the `target/` size, plus the per-unit times from cargo's own timing report.
- The size bar is NOT met and the wall bar IS met, so the row is kept on the wall. The `target/` total moves little because `incremental/` is 701 MB of the 944 MB and the setting does not touch it; the dependency rlibs are what shrink.
- The workspace crates keep their debug info, which is the row's own P2P: every workspace rlib is byte-for-byte the same size in both arms, and only the dependency rlibs shrink.
- What this receipt does not claim: it does not claim a build time for a quiet box. The wall numbers on a shared box are noisy (the same arm measured 304.73 s and 212.50 s), which is why the load-independent number from cargo's timing report (the sum of the unit times) is quoted next to it. It also does not claim the change helps the CI or the release build: this is the dev profile only.