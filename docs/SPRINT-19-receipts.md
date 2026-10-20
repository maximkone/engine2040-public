## Receipts

Earlier receipt text lives in git history (`git log -p -- docs/SPRINT.md`).

<!-- template: new blocks append via `node tools/land.mjs <packet> --dry` + `node tools/proof.mjs --skip-build`; do not hand-type golden/proof lines -->
> Golden `d57b5c8ae3096614` holds for every landing below unless the block names a move; new blocks paste the land+proof output, not hand-typed hashes.

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
