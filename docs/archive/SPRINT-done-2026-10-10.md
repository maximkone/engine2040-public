# SPRINT done rows 2026-10-10 (moved from the docs/SPRINT-*.md shards; reversible via git log)

Done `- [x]` rows moved out of the shards, verbatim with their continuation lines, grouped by shard.
tools/board-slice.mjs reads these ids as ticked (for needs: and the ticked count only); they are never open or ready rows.

---
## SPRINT-01-rome-first.md

- [x] BV1 viewer: Bevy viewer crate, step 1 of the Bevy-body decision (inbox EB-2026-10-03-S55, center D2) —
  `crates/viewer` (package `engine-viewer`, one bin; draws, replaces nothing) runs the deterministic arena sim
  headless and draws the arena kit from `content/arena/placement.json` (11 actors over the 8 glTF files in
  `content/arena/src/`) plus the CT7 hero `content/fighters/glb/hero_body.glb` (one skin, 43 joints) in a Bevy
  window or offscreen camera, the hero at the sim's own root-table position
  (`App::presentation().root_tables`) for the shown tick. The sim, the golden hash and every existing crate
  stay untouched; rapier3d stays inside the sim (no bevy_rapier); no Bevy plugin besides what the viewer
  needs. Tier A. Size L. Owns `crates/viewer/**` (new; depends on `engine-game` and `engine-arena` by path),
  one root `Cargo.toml` workspace line `bevy` 0.19.1 (MIT OR Apache-2.0, rust-version 1.95 under our 1.97.1;
  read on crates.io 2026-10-03) with `default-features = false` plus only the features it uses,
  `dynamic_linking` only behind a viewer `dev-fast` feature if it links on Windows, `Cargo.lock`, one
  `docs/DEPENDENCIES.md` row, one `docs/ARCHITECTURE.md` edge row. The viewer is NOT in root
  `default-members`, so bare `cargo build`/`cargo test` never compile Bevy. Accept: F2P
  `cargo test --locked -p engine-viewer --lib` green (placement parse gives 11 actors over 8 kit files; the
  hero glb skin has 43 joints; the hero transform equals the sim root-table position at ticks 0, 60 and 599,
  x2 identical) and fails today (no crate); pilot capture `cargo run --locked -p engine-viewer --release --
  --scene arena --fighters 2 --frames 120 --capture target/bv1/` writes `target/bv1/frame_000060.png` with
  the kit and the hero at its simulated place and the log line `bv1 tris=<n> joints=43`, n over 5,000, the
  judge opens the PNG; P2P golden `d57b5c8ae3096614` unchanged (`node tools/proof.mjs`), the diff touches
  only the Owns paths, `cargo tree -p engine-game -e normal` shows no `bevy`, `node tools/workspace-check.mjs`
  PASS, `cargo deny check` green. Stop: the first Bevy build is heavy (one heavy slot, the lane pool dir,
  packet front matter `max_min: 90`); at the cap report what compiles and the next step, and split before
  any resend. needs: none. (Bevy viewer test: Maxim decides the next step from the BV2 picture.)
  Done 2026-10-05 (Eyes, partly): `crates/viewer` builds against Bevy 0.20.0-rc.2 (`-j 2`, dev profile, deps at opt-level 3;
  one wgpu 30.0.1 in `cargo tree -p engine-viewer -i wgpu`; `cargo tree -p engine-game -e normal` lists no bevy, 0 lines);
  `cargo test --locked -p engine-viewer --no-default-features --lib` 35 passed; `cargo deny check licenses bans sources`
  ok (advisories not run). Offscreen capture at 1920x1080 on the RTX 3050 writes `target/bv1/frame_000030.png`,
  `frame_000060.png`, `frame_000090.png` and `manifest.json`; log line `bv1 tris=20496 joints=43`. The pictures were
  opened: the textured knight hero (helmet, plate, black shield, sword) stands on a sand floor in front of a wood slab
  with sky behind. Gaps, each its own row: the capture ran in the dev profile, not `--release` (BV5); golden
  `d57b5c8ae3096614` was not re-run by Eyes, the diff touches only `crates/viewer` and docs (BV5). Tick audit 2026-10-19: the `engine-viewer` crate was dropped on purpose (1296f273, rot); this row's crate is gone.
- [x] BV2 viewer: the hero animates and the sim drives the picture — in `crates/viewer` the hero plays the CT7
  clips in `content/fighters/glb/clips/` (`human_male_idle_combat_main_hand_melee.glb` standing,
  `human_male_run.glb` while the sim root-motion speed is over 0.5 m/s, a `human_male_cast_*` clip on a sim
  hit cue), bound to the skin by joint name; the 3 opponents (`opp_*_body.glb`) draw at their sim positions;
  frames advance with the accepted tick, never the wall clock. Tier A. Size M. Owns `crates/viewer/**` only.
  Accept: F2P `cargo test --locked -p engine-viewer --lib` green (clip choice equals the sim locomotion
  state for 120 ticks x2; the hero sword-hand joint moves over 1 cm between frame 30 and frame 90; every
  clip target name resolves to a skin joint, 0 unresolved) and fails before this row; pilot capture writes
  `target/bv1/frame_000030.png` and `target/bv1/frame_000090.png` with the hero in two different poses; the
  handoff shows today's cubes capture `target/pilot-gx13/owner-arena/frame_000015.png` beside
  `target/bv1/frame_000090.png`, both opened by the judge; P2P golden `d57b5c8ae3096614` unchanged,
  `node tools/workspace-check.mjs` PASS, BV1 tests still green. Stop: M 45 min. needs: BV1.
  Done 2026-10-05 (Eyes): the hero plays the idle, run and cast clips, bound by the bone name path (`bv2 fighter 0 clip
  0 binds 43 of 43 targets`, the log line `report_binding` in `crates/viewer/src/main.rs`); the clip rig is 2.329 times
  larger than the hero body, so `rig::retarget_clip` scales translation keys by the bone-length ratio (before it the
  limbs stretched; tests `a_retargeted_clip_keeps_the_hero_at_its_own_size`, `a_node_path_starts_at_the_scene_root`).
  `target/bv1/frame_000030.png` and `frame_000090.png` (opened) show the hero in two different run poses (tick 30 clip
  Run since 1, tick 90 Run since 85, legs and arms in different phases) at x 18.33 and 18.01. The 3 opponents use the hero's
  ratio, not their own (BV6). The side-by-side with the cubes capture is left to the judge. Tick audit 2026-10-19: the `engine-viewer` crate was dropped on purpose (1296f273, rot); this row's crate is gone.
- [x] BV4 viewer: the material hook for the one material record (decision D1; the Materials Fable owns
  `crates/viewer/src/material.rs`) — Eyes creates the file with a stub trait `LookMaterial` (record name in, a Bevy
  `StandardMaterial` out), the viewer draws every kit and fighter mesh through it and ships the identity mapping now.
  Tier A. Size S. Owns the stub file and its one call site in `crates/viewer/src/main.rs`. Accept: F2P
  `cargo test --locked -p engine-viewer --no-default-features --lib material` (the stub maps a record name to the
  same Bevy-free descriptor x2; an unknown name falls back to the default, never panics); golden unchanged.
  needs: BV1.
  Progress 2026-10-04 (Materials took this row): `crates/viewer/src/material.rs` maps a record to a Bevy-free
  descriptor (`RecordLook`, ior to reflectance, coat, anisotropy, transmission and volume, emission, wet and blood
  layers through the record twin); `cargo test -p engine-viewer --no-default-features --lib material` 6 passed;
  `to_standard_material` and the call site in `main.rs` are written but NOT compiled (the Bevy build is Eyes' BV1
  blocker): tick this row when `cargo check -p engine-viewer` passes.
  Done 2026-10-05: `cargo check --locked -p engine-viewer` 0 errors (Bevy 0.20.0-rc.2, dev profile);
  `cargo test --locked -p engine-viewer --no-default-features --lib` 41 passed (material 6/6: same descriptor
  x2, unknown falls back, never panics); golden `d57b5c8ae3096614` x2; `node tools/rust-quality.mjs --crate viewer`
  PASS 97 (A); `node tools/workspace-check.mjs` PASS. The hook is wired: `LookSource(RecordLook::shipped())` in,
  `apply_look` in Update draws known records through `to_standard_material`, the rest keep their glTF material. Tick audit 2026-10-19: the `engine-viewer` crate was dropped on purpose (1296f273, rot); this row's crate is gone.
- [x] VS2 render: textured draws — `DrawItem` gains `base_color:
  Option<TextureKey>`; the forward pass multiplies the material factor by the
  sampled texture when present. Tier A. Size M. Owns the `DrawItem` struct in
  `crates/render/src/present.rs` and the forward bind in
  `crates/render/src/present/gpu_present`. Accept: F2P a 2x2 checker texture
  renders both colors in an offscreen frame (llvmpipe); P2P every draw with
  `None` gives the same bytes as before (snapshot), 0 validation errors.
  needs: RA1.
- [x] VS6 game: rolling presentation cursor — `App::presentation` stops
  re-simulating from tick 0; poses advance once per accepted tick. Tier A.
  Size S. Owns `App::presentation` and its state in `crates/game/src/lib.rs`.
  Accept: F2P presentation cost at tick 18,000 within 2 times the cost at tick
  10 (bench); P2P poses byte-equal to the full replay x2. needs: none.
  Landed 2026-10-07 (vs6-look-r2): `window_frame_poses()` window-frame entry,
  `pose()` rewired through it; cursor 4/4, 18k bench 1/1, headless 600-tick
  x2 1/1, quality 95 A. Tick-incremental arena API split to follow-up row.
- [x] WIN6a game: staged-fight receipt — run the staged 600-tick
  fight (27 hits, 3 knockdowns) and pin its receipt: hashes x2,
  CT7 SHAs pinned, per-frame manifests recorded and differing
  across frames. Tier A. Size S. Owns the receipt + manifest
  hunks only (no presenter code). Accept: receipt replays x2
  byte-identical; manifests differ across frames. needs: CT7,
  CT15, WIN1. (A7, 2026-10-02.)
  Receipt 2026-10-02: landed 3a0491c-followup (staged_fight.rs
  +185 tests-only); staged_fight bin 4/4, receipt x2 identical
  (27 hits/3 knockdowns), manifests differ; quality game 81/B 0
  regressions; judge PASS.
- [x] CT22 content: arena establishing-shot staging — sand disc, gate
  tunnel mouth and stands ring placed via SP5 with provenance as the
  art-directed composition target RA14/WIN6 present, replacing the
  faceted cube backdrop. Tier A. Size M. Owns
  `content/arena/establishing.json` + staging glue (no pass code).
  Accept: window + twin show sand, stands and gate on the reference
  GPU with 0 validation errors; capture pair before
  `target/showcase/2026-09-27-q1/fight/frame_000015.png` vs after
  scores higher by this seat on silhouette + composition. needs: CT8.
  (CT22 re-dive 2026-10-03: land `establishing.json` headless first —
  composition of existing kit, no new bytes, SP5 0 findings + cook x2;
  capture rides CT60 pilot-only, never re-shot in a headless lane;
  see `docs/plan/content.md:320`.)
  Receipt 2026-10-09: `content/arena/establishing.json` landed headless-first (17 actors, placement+dressing SHAs pinned, camera matches `windowed.rs:186-190`); engine-assets lib 116/116 green, digest x2 `e12f70b8…`, packet-gate 1 path 0 findings; capture pair rides CT60 pilot-only.
- [x] GP24 game: camera stage wiring — wire the staged `camera.rs` rig
  hunks through the WIN1 presentation feed under the GP3 director
  (presentation camera work only, never the BET-GP24 feel kernel). Tier B.
  Size S. Owns wiring hunks beside `crates/game/src/camera.rs` +
  `camera_feed.rs` (director API frozen). Accept: 600-tick 2-fighter
  orbit keeps both framed through WIN1; trauma replays x2; feed within
  0.05 ms. needs: GP3, GP16. (PLN48 orphan verdict; checker wave-48:
  zero-caller FAIL held — tick only with the WIN1 feed caller.) R3's t60
  edge-clip + t120 corner-split fixtures join this row's framing proof;
  GP25 proves the play-step follow after this row lands (PLN49).
- [x] PQ1 perf: input-latency meter, headless leg — seeded fixture to
  first sim effect and first present, p50/p95 per build with machine
  and profile conditions in the report; the P2 bar is present p95
  ≤ 50 ms on the seeded 3-frame fixture. Tier A. Size S. Owns
  `crates/perf/src/latency.rs` (frozen; docs-only close). Accept:
  `cargo test --locked -p engine-perf latency -- --nocapture` green
  with the seeded 3-frame delay detected exactly (sim +1f, present
  +3f, verdict=pass) and `node tools/proof.mjs --perf` prints
  `[PASS] PQ1 input latency`; golden unchanged. needs: WIN1, PF2.
  Deep-dive 2026-10-03 (stuck 11 helpers/72h): root cause is the
  heavyweight `--perf` release gate (20+ min all-features, flakes on
  loaded boxes) retried for a frozen-file docs-only close, plus GPU
  PENDING reached for headlessly. Different approach: close headless
  lane-only on the light proof `cargo test --locked -p engine-perf
  latency -- --nocapture` green (headless_report lines
  present_p95=50.000ms + verdict=pass + reference_gpu PENDING in the
  log); the quiesced GPU fill rides pilot-only PQ18 (PQ1 -> PQ18,
  never the reverse); never gate this row on the full `--perf` run.
  2026-10-04: CLOSED on the light proof. `cargo test --locked -p engine-perf
  latency -- --nocapture` exit 0, 16 passed (seeded 3-frame delay: sim_p95=16.667ms,
  present_p95=50.000ms, p95_budget=50.0ms, verdict=pass; the headless_report
  line carries `reference_gpu ... PENDING (no GPU on headless runner)`). Log
  `target/evidence/pq1/latency.log` (debug profile, scratch target dir).
  Golden: `node tools/proof.mjs --skip-build` prints `legacy golden x2 -
  d57b5c8ae3096614 / d57b5c8ae3096614`. The full `--perf` release run was not
  run, as this row directs; the GPU numbers stay PENDING for PQ18. TICK.
- [x] UIX2 ui: settings — bindings, volume, graphics via TU1 staging.
  Tier A. Size M. Owns `crates/ui/src/settings.rs`. Accept: settings
  apply at boundaries and replay; every action undoable where it
  edits. needs: IN1, TU1.
  2026-10-04: LANDED `crates/ui/src/settings.rs` (`SettingsPanel`: stage on
  a draft, apply at a boundary tick never in the past, `SettingsTape`
  replay with a digest, undo of a staged edit and of an applied batch by an
  appended inverse; volume, graphics, rebind with a taken-button refusal,
  deadzone). `cargo test -p engine-ui --lib settings` 12 passed, clippy
  -D warnings clean, `node tools/rust-quality.mjs --crate ui` PASS 99 (A).
  Model and tests only: the windowed settings screen is not wired to it
  yet. TICK.
- [x] CT28 content: provenance gate lock (release refuses missing
  URL+SPDX+author+SHA / AI generator chain; 50-asset refusal matrix).
  Size S. Owns gate hunks in `crates/assets`. Accept:
  `cargo test --locked -p engine-assets provenance`, bytes x2, tick
  untouched. needs: CT7, CT8. (Plan extender wave 56; plan CT27.)

## SPRINT-02-done.md

- [x] C2 provider contract: request, result, event, capabilities, errors.
- [x] C4 anim contract: skeleton, pose, root and contact tables.
- [x] A1 assets: glTF mesh and material import; reimport keeps IDs.
- [x] A2 assets: texture cook with mips and sRGB rules.
- [x] A3 assets: skinned mesh, skeleton and clip import.
- [x] HR1 assets: polling watch with debounced recook.
- [x] R1 render: offscreen V1 forward PBR with PNG capture.
- [x] R2 render: IBL and shadow map as CPU reference.
- [x] R3 render: GPU skinning with a captured receipt.
- [x] R4 render: cascades, probe GI, bloom, TAA jitter and grading as CPU
  reference.
- [x] R5 render: particles and blood decals as CPU data.
- [x] R6 render: R5 torch-dust flipbook burst-sync — per-spawn flip
  offsets beside `Flipbook::index`, spawn stores offset, update uses
  `(base+offset)%F`. Tier B. Size S. Owns flip-offset hunks beside
  `crates/render/src/particles.rs`. Accept: `cargo test -p
  engine-render --lib particles` green + burst unique>=8, flick<=2,
  digest x2; O5 per-pass ms before LAND. needs: R5. (LADDER R5-flip
  ACCEPTED 2026-09-30, base unique1->14, flick1.36, digest721b5e3f x2.)
- [x] HR2 render: live shader and material swap keeping last-good.
- [x] E1 editor: dock shell over command batches.
- [x] E2 editor: viewport picking and gizmo to batch.
- [x] E3 editor: save and reopen, child-process play, durable journal.
- [x] SU1 editor: selective undo with conflict reports.
- [x] AR1 arena: headless match with checkpoints.
- [x] AR2 arena: physics, combat, brain and root motion in the match.
- [x] LOC2 arena: locomotion, controller and combat bindings.
- [x] FK1 arena: counterfactual forks on worker threads.
- [x] DT1 arena: fork decision recorder.
- [x] PH1 physics: Rapier adapter with hash and snapshot/restore.
- [x] PH2 physics: Rapier character controller.
- [x] PH3 physics: deterministic active ragdoll.
- [x] D1 physics: integer-only vehicle math.
- [x] CB1 combat: table-driven fight step.
- [x] CB2 combat: presentation-only gore events.
- [x] HR3 combat: validated versioned move tables.
- [x] BR1 brain: integer-utility gladiator policy.
- [x] BR2 brain: net policy with scripted fallback.
- [x] HR4 brain: versioned AI curves with fallback.
- [x] AN1 anim: pose sampling, blending, root-motion extraction.
- [x] LOC1 anim: baseline locomotion state machine.
- [x] LMM1 anim: motion nets over the integer runtime.
- [x] NET1 net: UDP LAN session with prediction and resimulation.
- [x] NET2 net: snapshot deltas and interpolation.
- [x] MCP2 mcp: play.run and capture tools.
- [x] MCP3 mcp: scene.brief, scene.spatial and scene.place.
- [x] PV2 provider: local and remote adapters.
- [x] PV3 provider: announcer and taunts with scripted fallback.
- [x] SP1 spatial: staged before/after gate.
- [x] SP3 spatial: digest-stable relation graph.
- [x] SP4 spatial: bounded brief.
- [x] SP5 spatial: intent placement solver.
- [x] QA1 playtest: agent with replayable bug tapes.
- [x] VF1 playtest: reachability, win-rate and budget verifiers.
- [x] PT34 playtest: declarative scenario fixtures — seed + input/wait steps +
  assertions + tags JSON shape from flax-mcp `scenarios/*.json`
  (`determinism_test.json`, license MIT, Maxim's own code) read by a small
  Rust reader. Tier B. Size S. Owns `crates/playtest/src/scenario_shape.rs`
  + `fixtures/scenarios/`. Accept: 3 sample scenarios parse steps in order
  with seed/tags present; same seed replays x2 identical. F2P:
  `cargo test -p engine-playtest --lib scenario_shape` (fails before: no
  declarative shape, hand-rolled grammars only). P2P:
  `cargo test -p engine-playtest --lib eval` stays green. needs: QA1.
  (FLAX-MCP HARVEST EB-2026-10-02-S93;
  research/flax-mcp-harvest/forge-scripts-tools.md #1.)
- [x] DT2 distill: reproducible trainer exporting to NN1.
- [x] NN1 nn: integer `.e2nn` runtime.
- [x] NN2 nn: int8-QDQ ONNX converter.
- [x] OB1 observe: typed events, ring buffers, spans.
- [x] HL2 proof: chaos suite with heal records.
- [x] FZ1 proof: deterministic fuzz harness.
- [x] LT1 proof: format museum.
- [x] SV1 save: bounded generations and failed-write recovery.
- [x] AU1 audio: device mixer and decode.
- [x] AU2 audio: spatial mix and crowd reactions.
- [x] UI1 ui: HUD model and colorblind palettes.
- [x] UI2 ui: text layout and quads.
- [x] IN1 input: bindings, rebinding, dead zones.
- [x] IN2 input: action-state input map for deterministic replay — digital +
   analog actions as pressed/held/released state sampled once per tick,
   ported as std-only Rust from `Leafwing-Studios/leafwing-input-manager`
   action-state shape (license Apache-2.0, reimplement only, never the
   crate dep). Tier B. Size S. Owns `crates/input/src/action_state.rs`
   (bindings frozen). Accept: same input tape replays x2 byte-identical;
   held-vs-pressed never leaks across tick boundaries; 600-tick tape
   deterministic. F2P:
   `cargo test -p engine-input --lib action_state` (fails before: no
   per-tick action state). P2P:
   `cargo test -p engine-input --lib attack_key_maps_to_attack_bit` stays
   green. needs: IN1.
   (STEAL 2026-10-03 leafwing-input-manager,
   https://github.com/Leafwing-Studios/leafwing-input-manager.)
- [x] DS1 destruction: baked debris field (library; the arena does not call
  it yet).
- [x] GM1 game: arena app as a headless data path.
- [x] GM2 game: loopback LAN session with AI fill.
- [x] HR5 game: practice mode with frame swaps.

## SPRINT-03-finish-line.md

- [x] GX1 render: frame graph goes live — `submit_gpu` in `graph.rs` and
  the executor record real wgpu passes with bind groups and RA2 transient
  aliasing, driven by WIN1 and ED1 in the window build. The `LookFrame`
  bind shape is the shared contract, landed first. Tier A. Size L. Owns
  `crates/render/src/executor.rs` and the `graph.rs` submit. Accept: the
  window build submits the forward pass plus at least 3 graph passes with 0
  validation errors; the llvmpipe twin runs the same compiled graph; no
  `Unsupported` on a device. needs: RA1, RA2.
- [x] GX2 render: RL1 live — cascaded sun shadows and clustered torches
  bound into the forward pass. Tier A. Size M. Owns the
  `shadow_cluster.rs` submit. Accept: llvmpipe parity with the R4
  reference; reference-GPU capture and pass timing. needs: GX1.
- [x] GX3 render: RL2 live — prefiltered cubemap, irradiance SH and BRDF
  LUT sampled on the GPU. Tier A. Size S. Owns the `ibl.rs` submit
  (frozen: no second chart path, no re-derived PNG/gamma, no new dep).
  Accept (headless close, lane-runnable): `cargo test -p engine-render
  --lib ibl` green incl. the forward-bind assert; llvmpipe parity with
  the R2 reference within tol; 0 validation errors; `node
  tools/workspace-check.mjs` PASS plus `node tools/proof.mjs` PASS
  golden x2 plus `node tools/rust-quality.mjs` PASS. The reference-GPU
  material chart plus Q1 V1-V3 ride the GX13 pilot only
  (GX3 → GX13 → Q1, never the reverse). needs: GX1.
  Wave 49: packet LANDED (commits 6ef6421 + 835ee7d, ibl submit wired
  per-frame, render 319 + 427 all-features, quality PASS); judge PASS
  depth 3/5 RUST 4/5. Deep-dive 2026-09-30: the conflated
  headless-plus-GPU acceptance kept re-landing code while the GPU tail
  needs the pilot; gates now split so one headless writer closes GX3.
- [x] GX4 render: RP1 live — TAA/TAAU and the post chain (bloom, exposure,
  ACES, grading) as graph passes. Tier A. Size M. Owns the `post.rs`
  submit. Accept: ghost and shimmer scenes pass on the GPU; reference-GPU
  clip. needs: GX1.
- [x] GX6 proof: truth gate — `node tools/proof.mjs` runs the window build
  one frame on llvmpipe and fails when any ticked `render` row's pass still
  returns `Unsupported` on a device; `docs/plan/render-look.md` gets a "GPU
  live" column. Tier A. Size S. Owns `crates/proof` checks (orchestrator
  wires `tools/`). Accept: a seeded stub fails the gate; the current tree
  fails until GX2–GX5 and GX7–GX10 land, then passes. needs: none.
- [x] GX7 render: RL4 live — physical sky LUTs and EV100 exposure as graph
  passes feeding IBL capture and fog. Tier A. Size M. Owns the `sky.rs`
  submit. Accept: llvmpipe parity with the RL4 CPU reference; no
  `Unsupported` on a device; 0.4 ms on the reference GPU. needs: GX1.
- [x] GX8 render: RL6 live — layered materials bound in the forward pass.
  Tier A. Size M. Owns the `material_layer.rs` submit. Accept: material
  chart parity on llvmpipe; reference-GPU chart; no `Unsupported` on a
  device. needs: GX1.
- [x] GX9 render: RP2 live — GPU particle simulate and draw passes. Tier A.
  Size M. Owns the `particles_gpu.rs` submit. Accept: identical particle
  bytes x2 from a seed on one device; 1.0 ms on the reference GPU; no
  `Unsupported` on a device. needs: GX1.
- [x] GX10 render: RP3 live — deferred decals with depth reject in the
  graph. Tier A. Size S. Owns the `decals.rs` submit. Accept: depth
  fixtures land decals on sand not air on the GPU; 0.5 ms on the reference
   GPU; no `Unsupported` on a device. needs: GX1.
- [x] GX12 render: IBL forward-wiring lock — headless only, no GPU.
  Prove `FORWARD_IBL_WGSL` is bound in the forward pass, not just
  shipped: a graph/forward-buffer test asserting the chunk hash plus
  the SH uniform bytes appear in the compiled forward bind, llvmpipe
  parity with the R2 reference within posted tol, stale proof-flip
  refreshed. Tier A. Size S. Owns wiring-assert hunks beside
  `crates/render/src/ibl.rs` + `graph.rs` (no new pass code, no
  manifests). Accept: `cargo test -p engine-render --lib ibl` green
  incl. the new bind assert; shade abs within tol of the 9028bf8
  0.00002 figure; `node tools/proof.mjs` PASS golden x2; zero
  per-submit heap allocs after warmup. needs: GX1, RL2.
  (GX3 VERDICT wave 56: split the conflated acceptance; DROP 077-gx3-r2.
  Wave 58: needs GX3 dropped — GX12 is the headless wiring half, GX3's
  capture half rides GX13; GX1 + RL2 ticked.)
- [x] RA1 render: windowed presenter over a draw list — surface, swapchain,
  depth and HDR targets, multi-draw forward PBR reusing the `v1_gpu` shader,
  GPU mesh/texture cache, resize/minimize/Outdated/Lost recovery keeping
  last-good. Owns new `crates/render/src/present.rs`. Size L. Accept: the
  offscreen twin renders on llvmpipe with 0 validation errors; a
  reference-GPU capture; resize and minimize survive. needs: R1, R3.
- [x] WIN1 game: `arena.exe` — winit loop, fixed-tick accumulator with
  interpolation, keyboard/mouse bindings to `FighterInput`, cpal audio, HUD
  through RA1, `--frames N --exit` smoke. Owns a new binary target in
  `crates/game`. Size L. Accept: Xvfb smoke presents N frames; the same tape
  gives the headless checkpoints; a reference-GPU play session. needs: RA1,
  GM1, IN1, AU1.
- [x] ED1 editor: editor executable — winit, egui-winit and egui-wgpu, dock
  layout, viewport drawn by RA1 into a texture, gizmo, save/reopen, play
  launches WIN1 as a child process. Size L. Accept: Xvfb smoke; panel to
  batch tests; a reference-GPU session. needs: RA1, E2, E3.
- [x] MCP5 mcp: `e2040-mcp` binary that serves the stdio MCP door on a project
  folder, so Claude Code, OpenCode, Cursor or any MCP client connects with one
  config line (the orchestrator adds the tokio runtime features first).
  Size S. Accept: a scripted external client lists tools, previews, applies
  and undoes one edit over stdio. needs: MCP3.
- [x] AS1 assets: content bundle v1 — cooked meshes, textures, skins and
  clips packed with blake3 digests, loaded by WIN1 and ED1. Size M. Accept:
  round-trip; a corrupt bundle is refused; load time recorded. needs: A1, A2,
  A3.
- [x] AS7 assets: bundle cook command — the missing producer (EXPLORE47
  wave 46: `BundleWriter` is library-only; no CLI feeds cooked
  `content/` bytes into `arena-bundle.bin`; GM3 blocked on the stub).
  Tier B. Size S. Owns the cook entry in `crates/assets` (writer +
  cooker already there). Accept: `BundleLoader::load` returns a
  non-empty bundle from the produced file; pack-clean `BUNDLE_RESOLVE`
  accepts it; stub refused without `--allow-stub`. needs: CT7, CT8, AS1.
  Wave 47: bundle_cook bin + lib (206+2 tests); cooked 27MB/7 entries
  incl. 2 real CT7 textures with blake3 digests; BUNDLE_RESOLVE stages
  bundle:REAL; stub refused; checker PASS, judge TICK as written
  (glb-mesh cooker wiring follow-up).
- [x] RL1 render: GPU cascaded sun shadows plus clustered torch lights.
  Size L. Accept: llvmpipe parity with the R4 CPU reference within
  tolerance; reference-GPU captures and pass timings. needs: RA1, R4.
- [x] RL2 render: GPU IBL — prefiltered cubemap, irradiance SH and BRDF LUT
  from the Poly Haven HDRI. Size M. Accept: llvmpipe parity with the R2 CPU
  reference; reference-GPU material chart. needs: RA1, R2.
- [x] RP1 render: engine-owned TAA/TAAU (jitter, motion vectors,
  reprojection, neighborhood clamp) and the GPU post chain (bloom, exposure,
  ACES, grading). Size L. Accept: ghosting and shimmer test scenes;
  reference-GPU clips. needs: RA1, R4.
- [x] RL4 render: physical sky (Hillaire 2020) + EV100 exposure
  feeding IBL capture and fog color. Tier A. Size M. Owns
  `crates/render/src/sky.rs`. Accept: llvmpipe parity within tolerance;
  0.4 ms on the reference GPU (reference GPU). needs: RL2, RA2.
- [x] RL6 render: layered materials (cloth, sand, wet blood) over the
  R1 PBR core with no mirrored types. Tier A. Size M. Owns
  `crates/render/src/material_layer.rs`. Accept: material chart parity
  within tolerance on llvmpipe; reference-GPU chart (reference GPU).
  needs: RA1, AS1.
- [x] RL7 render: height fog + contact shadows for small details. Tier A.
  Size M. Owns `crates/render/src/fog.rs`. Accept: fog matches the sky
  color ramp within tolerance on llvmpipe; contact pass 0.3 ms on the
  reference GPU (reference GPU). needs: GX7, GX1, RL1, RL4.
- [x] RL12 render: boosted-clock budget re-baseline — every look bar
  re-measured on boosted release clocks with exactness asserts in
  default and wall-clock asserts quarantined to `--perf`. Tier A.
  Size S. Owns the quarantine-split hunks in `crates/render/src/`
  tests (no new pass code). Accept: bars match PF2 boosted captures
  within noise; seeded budget regression fails `--perf`; default suite
  stays green; golden unchanged. needs: PF2, GX1.
   (PLN26 render-look.md, reviewed 2026-09-25; wave 35: 6 files test-only
   quarantine hunks, render 294 green, 10 ignored wall tests, checker PASS.)
- [x] RL20 render: tunnel-mouth exposure split — one frame holds sunlit
  sand plus the dim gate-tunnel mouth using the RL4 sky bake with the
  RL15 day rig and RL7 fog ramp, as bind glue only in a new
  `crates/render/src/tunnel_mouth.rs` (sky/fog/pass code frozen;
  `arena_backdrop.rs` and `arena_air.rs` untouched). Tier A. Size S.
  Owns `crates/render/src/tunnel_mouth.rs`. Accept: capture pair before
  `target/showcase/2026-09-28-r2/editor-viewport-rtx3050.png` vs after
  on the reference GPU scores higher by this seat on light +
  atmosphere with 0 validation errors; golden unchanged. needs: RL4,
  RL7, CT8. (ART wave 85.) Tick audit 2026-10-19: closed as dead per docs/plan/2026-10-19-owner-queue.md; the named render `tunnel_mouth.rs` went with the render wip sweep f737386e.
- [x] RP2 render: GPU particles (dust, torch flame, sparks) with a fixed
  seeded pool and CPU-sim parity. Tier A. Size L. Owns
  `crates/render/src/particles_gpu.rs`. Accept: identical particle bytes
  twice from the same seed; llvmpipe parity within tolerance; 1.0 ms
  budget on the reference GPU (reference GPU). needs: RA2, R5.
- [x] RP4 render: SMAA fallback + dynamic resolution around RP1 TAA/TAAU.
  Tier B. Size M. Owns `crates/render/src/aa_scale.rs`. Accept: shimmer
  scene passes with TAA off; scale holds 60 fps under load on the
  reference GPU (reference GPU). needs: GX1, RP1, PF2.
- [x] RP5 render: TAAU scale-seam production wiring — the post/TAAU pass
  calls `compile_taa_off_for_scale` with the `ScaleController` hysteresis
  so the SMAA-first graph drives the presented size (EXPLORE wave 22:
  only aa_scale tests call it today). Tier B. Size S. Owns the scale-seam
  hunks in `crates/render/src/post.rs` + `graph.rs` (aa_scale API frozen).
  Accept: TAA-off path renders at controller scale on the window build;
  0 validation errors; scale-hold numbers posted. needs: RP4.
- [x] RP7 render: predicated SMAA edge (depth+luma) for TAA-off shimmer
  (READER-LOOK53 card 2). Tier B. Size S. Owns
   `crates/render/src/aa_scale.rs` edge hunk (API frozen). Accept:
   shimmer variance under 0.06 with 60fps scale-hold under load +
   llvmpipe parity. needs: RP4, RP5, PF2. (LOOK53.)
  Wave 54: TICK (insertions-only, API frozen; shimmer 0.0529 < 0.06,
  GPU parity 0.0, render 438/0, clippy/fmt clean, gate + quality PASS;
  note: aa_scale.rs now 1517 prod lines vs 1500 guidance — future split
  row candidate, not this row).
- [x] RP8 render: velocity-gated TAA reject + variance-clip A/B — post.rs
  already velocity-rejects TAA so this row owns the variance-clip delta
  only: Karis variance clip vs 3x3 box-clamp on ghost + shimmer fixtures.
  Tier B. Size S. Owns clip-compare hunks beside
  `crates/render/src/post.rs` (resolve frozen). Accept: ghost-trail px
  cut ≥30% with shimmer ≤0.06 and ms +≤5%, LADDER SPIKE line with chart
   delta + hue skew + per-pass ms, or REJECT with numbers. needs: GX4,
   RP7. (P4 verdict wave 57 TAKE; LADDER P4 CANDIDATE.)
  (A7 split 2026-10-02: parent stuck — wave-30 lane verdict REJECT
  `37d32db`, box stays canonical. Children land the closeout.)
  (DO NOT DISPATCH 2026-10-05: REJECT landed twice, LADDER P4 x2 lines. Take RP8a XS closeout, never RP8. RP8b only if the lead overturns with a fused bound inside the ms bar plus release-perf plus digest x2.)
- [x] RP8a render: RP8 closeout — post the wave-30 REJECT numbers
  (ghost/shimmer/ms + chart delta) to the LADDER SPIKE line, then
  tick RP8. Tier B. Size XS. Owns board + LADDER hunks only
  (`crates/render/src/post.rs` untouched — box-clamp stays
  canonical). Accept: SPIKE line carries REJECT + numbers; RP8
  ticked. needs: none. (A7, 2026-10-02.)
- [x] RP9 render: hero motion-hold clip — a fixed 120-frame slow push-in
  over the twin establishing view proves temporal hold without boiling,
  in a new `crates/render/src/clip_hold.rs` reusing the RP1 TAA resolve
  and RP5 scale seam (resolve and `post.rs` frozen). Tier A. Size S.
  Owns `crates/render/src/clip_hold.rs`. Accept: moving-camera clip
  pair before `target/quickstart-fight-b/captures/frame_000003.png`
  (still) vs after on the reference GPU scores higher by this seat on
  motion + composition with ghost/shimmer inside the RP6 bars and 0
  validation errors; golden unchanged. needs: GX1, GX4, RP5. (ART
  wave 85.)
- [x] RL28 render: look-capture placeholder refusal beside
  `crates/render/src/manifest.rs` (Q1 consumes) — seeded 3-pattern set
  (1 cube SHA, 1 64x48 noise, 1 30-frame identical run) all fail naming
  the pattern; 3 real captures pass; 0 validation errors; golden
  `d57b5c8ae3096614` unchanged. Tier A. Size S. Owns
  `crates/render/src/manifest.rs`. Accept: `cargo test -p engine-render
  --lib manifest` green. F2P: the placeholder-refusal unit fails before
  (no refusal unit on master). P2P: manifest suite green, golden held,
  `cargo tree` headless unchanged.
- [x] LM1 anim: motion matching v2 — trajectory prediction, feature schema,
  deterministic accelerated search, inertialized transitions. Size L.
  Accept: identical picks twice; search cost per character in the receipt.
  needs: AN1, NN1.
- [x] TV1 anim: traversal — obstacle probe contract, vault, hurdle, mantle,
  climb and drop chooser, motion warping onto edges; authoritative and
  deterministic. Size L. Accept: probe fixtures pick the expected action;
  warped root lands on the edge within 1 cm; replay is exact. needs: LM1, PH2.
- [x] PX1 physics: per-bone powered ragdoll tracking the animated pose,
  partial ragdoll on hit, get-up with blend back. Size M. Accept: hash x2;
  get-up returns to animation within budget ticks. needs: PH3.
- [x] DX1 destruction: pre-fractured collections with bonds, strain and
  anchors; unsupported islands fall; server-authoritative break events; the
  arena match calls it (new arena to destruction edge). Size M. Accept:
  identical break sets twice; anchored chunks stay; a sword hit in the match
  breaks a crate. needs: DS1, AR2.
- [x] DX2 destruction: CSG fracture bake — offline boolean fracture chunks
  from blade-plane cuts (brusher BSP patterns, MIT; reimplemented, never
  vendored), deterministic bytes x2, break events reuse DX1. Size M. Accept:
  identical chunk sets twice; a sword hit in the match cuts a crate along
  the blade plane. needs: DX1, CT2.
- [x] MP1 net: dedicated headless server, LAN discovery, reconnect with slot
  reclaim and AI takeover. Size L. Accept: loopback test with a dropped and
  rejoined client converges; 8-bot soak report. needs: NET2, GM2.
- [x] AX1 provider: OpenAI-compatible, Anthropic Messages, Gemini and Ollama
  adapters behind the C2 contract, each passing the PV1 suite; live tests
  opt-in by environment variable. Size M. Accept: suite green per adapter
  on recorded fixtures. needs: PV2.
- [x] EA1a mcp: assistant loop skeleton — transcript types, inspect through
  `scene.brief`, one scripted mock-provider proposal, no writes. Size S.
  Accept: transcript golden. needs: MCP3, PV2.
- [x] EA1b mcp: proposal to `tx.preview` plus spatial checks; a bad proposal
  returns its conflict set and writes nothing. Size S. needs: EA1a.
- [x] EA1c mcp: fork play-test before apply; a balance-breaking proposal is
  rejected by its fork metrics (A6 fork proof). Size M. needs: EA1b, FK1.
- [x] EA1d mcp: apply with the CAP1 approval, undo, provenance receipt.
  Size S. needs: EA1c.
- [x] EA1e mcp: the same loop through the local and remote adapters on
  recorded fixtures (A6 AI-agnostic piece). Size M. needs: EA1d, AX1.
- [x] EV1 playtest: eval harness — 20 authoring and 5 play tasks through the
  mock, local and remote providers and one external MCP client over MCP5;
  completion, rejected proposals, latency and cost per backend. Size M.
  Accept: one command reproduces the report. needs: PV2, MCP3, EA1e, MCP5.
- [x] CR1 playtest: causal replay — remove or perturb one input at a time
  over a failing tape and report the smallest cause and the first divergent
  tick (A7). Size M. Accept: a seeded bug is attributed to its input. needs:
  QA1.
- [x] AH1 observe: supervisor and fault catalog — bounded heal policies
  (retry, reset, last-good, degrade, reconnect, AI takeover, restore,
  quarantine), escalation with backoff, sim heals applied at tick boundaries
  and recorded in the tape. Owns `crates/observe`. Size M. Accept: each
  catalog fault heals or quarantines within budget in chaos tests. needs:
  none.
- [x] AH2 render: surface Outdated/Lost and device loss heal through the
  supervisor, keeping the last-good pipeline and lowering quality on
  out-of-memory. Size S. Accept: forced-loss tests recover. needs: AH1, RA1.
- [x] AH3 audio: default-device change or loss reopens with backoff through
  the supervisor while the mixer keeps running. Size S. Accept: a simulated
  device loss recovers and reports. needs: AH1.
- [x] AH4 net: disconnect reconnects with slot reclaim while AI holds the
  slot; a desync resyncs from the authoritative snapshot. Size M. Accept:
  loopback drop and desync tests converge. needs: AH1, MP1.
- [x] AH5 save and provider: a corrupt save falls back to the previous
  generation; a provider timeout or refusal falls back to the scripted path;
  both surface as notices. Size S. Accept: chaos rows per fault. needs: AH1.
- [x] PF1 perf: budget gate — deterministic release-mode workloads in
  `crates/perf` (8-fighter arena tick, physics, AI, destruction) with
  recorded baselines, plus a counting-allocator test for zero steady-state
  allocations per tick; `node tools/proof.mjs --perf` runs it. Size M.
  Accept: the gate fails on a seeded regression and a seeded allocation.
  needs: none.
- [x] PF2 perf: profiler on observe spans (puffin or tracy, license read
  first) plus GPU timestamp queries. Size M. Accept: a frame capture names
  the costliest pass and system. needs: PF1, RA1.
- [x] CT1 assets: engine-native animation cook — retarget clips between
  skeletons by bone map with bind-pose correction, extract root motion and
  foot contacts, read clip tags from a sidecar; any glTF tool works, Blender
  is optional. Size M. Accept: three CC0 Quaternius clips retarget onto a
  second skeleton within tolerance and cook identically twice. needs: A3,
  AS1.
- [x] CT2 assets: engine-native mesh cook — LOD chain by quadric
  simplification, collision hulls, `FRAC_` chunk import for DX1, budget
  checks at import. Size M. Accept: LODs meet the triangle budgets; hulls
  contain their mesh; cook is identical twice. needs: AS1.
- [x] CT3 content: locomotion set — 100STYLE (CC BY 4.0, motions only) and
  Quaternius clips cooked by CT1 onto the hero skeleton, tagged idle, walk,
  run, start, stop, pivot, strafe and turn, baked into the LM1 database.
  Size M. Accept: the bake is identical twice; attribution rows in
  `EVIDENCE.md`. needs: CT1, LM1.
- [x] CT4 content: AI modeling loop — one MCP client drives an outside
  modeling tool (Blender through `mcp-for-blender`, MIT) and the engine
  through MCP5: build a prop, export glTF, cook, spatial-check, place; every
  generated asset gets a provenance row naming generator and license.
  Size M. Accept: a recorded session replays; a generator without a readable
  license is refused. needs: CT2, MCP5.
- [x] TU1 editor: live tuning panel — change combat and AI numbers during a
  fight through the validated swaps (HR3, HR4), applied at tick boundaries
  and recorded in the tape (A10). Size M. Accept: a tuned fight replays
  exactly. needs: ED1, HR5.
- [x] TPL1 editor: templates for a new arena, prop, weapon or fighter.
  Size M. Accept: each template creates an item that imports, validates and
  plays. needs: ED1, AS1.
- [x] EV3 proof: recorded MCP run (A6) — the EV1 suite (20 authoring + 5
  play tasks) run across the mock, local and remote adapters plus one
  external MCP client, recorded with its tape. Tier A. Size S. Owns
  `target/evidence/a6/` (report + tape). Accept: all 25 tasks green on all
  four doors in one recorded run; the run's evidence line lands in the
  `FINISH-RICH.md` status table. needs: EV1, MCP5.
  Wave 44: 75/75 green (per backend 25/25), parity 23/25 (a08/a12 remote
  privacy refusals by design), external-mcp completed=true tools_listed=7
  incl scene.brief (binary rebuilt from MCP13 sources); eval 33/33;
  checker PASS, judge parity-excuse HOLDS.
- [x] EV4 proof: recorded seeded-bug run (A7) — one seeded gameplay bug,
  its replaying tape and the causal attribution (which system, which tick).
  Tier A. Size S. Owns `target/evidence/a7/` (report + tape). Accept: the
  seeded bug fails exactly its keypoint and nothing else; attribution names
  the system and tick; the run's evidence line lands in the
  `FINISH-RICH.md` status table. needs: QA1, BR2.
  Wave 44: divert-8-1 fails exactly kp06-parry-riposte, 29/30 siblings
  green, cause tick 8 slot 1, tape replays; seeded_bug 3/3; checker PASS,
  judge code-PASS.
- [x] EV5 proof: recorded chaos + fuzz run (A9) — the chaos suite with
  heal records plus the deterministic fuzz harness, one recorded run.
  Tier A. Size S. Owns `target/evidence/a9/` (report + seed corpus).
  Accept: every chaos fault heals or quarantines with its record; the fuzz
  corpus runs clean and reproduces; the run's evidence line lands in the
  `FINISH-RICH.md` status table. needs: HL2, FZ1.
  Wave 44 PARTIAL (stays open): 3 healed + 1 quarantined, 8
  loud-unsupported (unshipped subsystems), 0 injection-failed, golden x2
  match, fuzz 20/20 clean; board wants every fault healed/quarantined —
  needs 8 recoveries or an operator amend to loud-unsupported-as-partial.
  Wave 45: 12/12 faults healed-or-quarantined with records (3 healed,
  9 quarantined, unsupported 0), golden + digests x2 match, a9 7/7 +
  chaos 8/8; checker PASS, judge code-PASS (evidence regen-confirmed).
- [x] EV7 proof: recorded editable-scene run (A2) — in the editor exe,
  import a licensed mesh with textures from the CT7/CT8 content, place
  three objects, edit one component and one material, save, close, reopen;
  references stay stable. Tier A. Size S. Owns   `target/evidence/a2/`
  (report + captures + scene file). Accept: the reopened scene matches the
  saved one byte for byte on its references; the run's evidence line lands
  in the `FINISH-RICH.md` status table. needs: CT8, ED3.
  Wave 46 progress (stays open): session driver + a2_evidence 1/1,
  save/reopen SHA256 MATCH, exe smoke exit 0. Still open: v4 ids break
  cross-run bytes, zero PNG captures, headless not exe-window — wave-47
  stable-ids + snapshot packet, then tick.
  Wave 47 progress (stays open): stable slot ids (cross-run SHA256 MATCH
  x2 on main), viewport.png + reopened.png present, exe smoke exit 0.
  Still open per judge: exe-driven session (not headless paths),
  material persistence in scene bytes, window-captured PNGs.
  Wave 49: packet LANDED (commits 5e47090 + 9065646 + db7f167, 13
  owned files, editor 196 green, quality PASS); judge PASS depth 4/5
  RUST 4/5. Stays open: exe-window record + window PNGs.
  Wave 56 TICK (EV7 pilot packet): `target/evidence/a2/` on disk
  (report.txt + viewport.png + reopened.png + tape.log + journal.txt),
  3 nodes placed (hero + crate + actor), inspector pose edit 0.00 -> 2.50,
  material bind `hero-mat`, 3 refusals logged (unknown asset/pose/material),
  save/reopen byte-identical fnv 4c089852df18b94b, exe-gpu-offscreen
  640x400 3 draws on RTX 3050 6GB driver 610.62 validation_errors 0;
  FINISH-RICH A2 evidence line updated.
- [x] EV8 proof: recorded editor run (A3) — viewport, picking, hierarchy,
  inspector, gizmo move, undo/redo, logs and isolated play (ED4), then a
  forced game-process crash the editor survives. Tier A. Size S. Owns
  `target/evidence/a3/` (report + captures + journal). Accept: every step
  in one recorded session, the crash recovered with no lost edit; the
  run's evidence line lands in the `FINISH-RICH.md` status table.
  needs: ED4, ED5, SU1.
  Wave 56 LANDED (EV8 pilot packet): `target/evidence/a3/` on disk
  (report.txt + viewport.png + reopened.png + journal.txt + rows.txt),
  800x600 r2, pick resolves hero, inspector edit r3, crash r10 -> r11 no
  lost edit, replay hash ea03666fe03831dd x2; FINISH-RICH A3 evidence line updated.
  Wave 49: packet LANDED (same commits as EV7, A3 crash-recovery +
  undo/redo + byte-stable reopen, a2_evidence 1/1); judge PASS depth
  4/5 RUST 4/5. Stays open: exe-window record + crash-on-window.
- [x] EV11 proof: evidence hardening — digest every `target/evidence/*/report.txt`
  into its `corpus/digests.txt` (or a sidecar `.sha`) at write time plus a
  `verify-evidence` check that fails a tampered report (ADV45 wave 45:
  edited a6/a7 reports verify nowhere). Tier A. Size S. Owns the digest
  hunks beside the a6/a7/a9 writers + the verify check. Accept: a tampered
  report fails the check loudly; untampered regens pass; golden unchanged.
  needs: EV3, EV4, EV5.
  Wave 45 finding, wave 46 close: a6/a7/a9 writers emit digests+seal,
  tamper demos fail typed, regens pass, FNV std-only; checker PASS,
  judge TICK (proof/finish verify wiring follow-up).
- [x] EV13 proof: record-path policy for the proof CLI — `--record` /
  `--replay` refuse lane-dir/traversal paths loudly (ADV46 wave 46:
  a `.lanes/`-containing record path succeeds silently today). Tier A.
  Size S. Owns the path policy + negative test in `crates/proof`.
  Accept: lane/traversal paths refused with a named error; normal
  paths unaffected; golden unchanged. needs: none.
  Wave 47: check_tape_path wired in cli read/write; lane/traversal/
  absolute refused + demos; proof.mjs tape moved to target/proof-tape;
   checker PASS, judge TICK.
- [x] EV14 proof: keyed evidence seal — replace/augment the FNV
  `digests.sha` with an HMAC (operator-held key; no hand-rolled
  crypto — `hmac` + `sha2`/`blake3` crate, license-checked) or a
  detached operator signature (ADV47 wave 47: re-sealed forgery
  passes `verify_evidence` today). Tier A. Size M. Owns the seal
  hunks beside the a6/a7/a9 writers. Accept: the ADV47-A forgery
  FAILS verify; clean regens PASS; golden unchanged. needs: EV11.
  Wave 52: TICK ed5139b (judge 4/4; seal legs green).

## SPRINT-04-finish-quality.md

- [x] PQ2 playtest: balance tournament — 200 AI-vs-AI fork matches per
  difficulty: win rate, kill share per move, stuck fighters, reachability of
  parry, riposte and clash. The P3 bar: 40–60% win rate, no move above 35% of
  kills, zero stuck. Tier A. Size M. Owns `crates/playtest/src/tournament.rs`.
  Accept: a seeded overpowered move is flagged; report reproduces twice.
  needs: GA1, FK1, VF1.
- [x] PQ8 audio: sound coherence for combat impact — hit-event-synced
  audio over the SN mix (ideas only, license UNVERIFIED). Tier B.
  Size S. Owns `crates/audio/src/coherence.rs`. Accept: 10 combat
  scenarios with audio synced to hit events; no per-tick allocation.
  Kill: float math in tick. needs: PQ2, CT3. (RES-ARXIV wave 35 Dive
  11, reviewed 2026-09-26; remapped from colliding PQ3; wave 39: 10
  scenarios + 0-alloc proof, audio 130 green, checker PASS.)
- [x] API1 mcp: Engine Insight — an always-fresh index of the engine for
  agents (the forge flax-nav idea; Unity Insight, arXiv 2609.27585, cut agent
  tokens 53% and time 52%). Indexes every public item (rustdoc JSON or a `syn`
  walk: path, signature, docs, units), MCP tool schemas, content assets and
  scene references, and which board row owns which file. BM25 search in
  plain Rust (no new dependency in the default headless graph) weighted
  name 10, path 5, docs 1 (forge `FlaxApiFts.cs` weights), plus a callers
  graph. New MCP tools in `e2040-mcp`: `api.search`, `api.describe`,
  `code.callers`, `asset.refs`; rebuilt incrementally on file change. Tier A.
  Size L. Owns `crates/endpoint/src/insight/`, `fixtures/insight-gold.json`.
  Accept: top-3 hit rate ≥ 90% on a 50-query gold set written from board
  rows; one-file rebuild < 2 s; an A/B of 10 packets with and without the
  tools shows fewer tokens and tool calls with them. Hunt 2026-09-25: Unity Insight's gains were cost-only on 28 questions; our A/B scores correctness as well as tokens and time. needs: MCP5.
- [x] MCP7 mcp: self-describing door — `rpc.discover` manifest, schema per
  tool, a read-only `scene.watch` subscription and one error shape
  `{code, message, data}` (Bevy Remote Protocol, Apache-2.0, as a spec). Tier
  A. Size M. Owns `crates/endpoint/src/discover.rs`. Accept: a 20-step VP1
  session with 0 polls gives the same digests as polling, twice. GitHub hunt 2026-09-25: subscriptions are removed on disconnect (Drop), batch preview returns per-item `{ok, error}`, methods list/query/get/set/spawn/watch map to one `state.patch` with default deny (modelcontextprotocol/servers, Bevy remote). Deep sweep 2026-09-25: hidden system tools, runtime-error capture opt-in (default off), screenshots as evidence, bursts coalesced to the latest digest, own-session changes suppressed. needs: MCP5, API1.
- [x] MCP8 mcp: scene tool guards — one apply at a time (single flight), an
  allow-list of scene operations, no arbitrary type instantiation, and
  `scene.brief` naming every object as `name [id]` so duplicate names never
  mis-place (patterns from blender-mcp, godot-mcp and Unreal_mcp, all MIT).
  Tier A. Size S. Owns `crates/endpoint/src/scene_tools.rs`. Accept: a
  duplicate-name scene places correctly; a denied operation is refused with
  a reason; the lock never stalls `play.run`. needs: MCP3.
- [x] MCP9 mcp: declarative tools and test harvest — a `#[e2_tool]` attribute
  turns a typed function into a registered MCP tool with schema and errors,
  and `test.run` returns structured results agents can act on (Unity-MCP
  pattern, Apache-2.0; kept inside the tool-list budget). Tier B. Size S.
  Owns `crates/endpoint/src/tool_macro.rs`. Accept: the EV1 suite is equal
  with generated tools; the tool list stays within its 20,480-char budget.
  needs: MCP7. (Wave 27: MCP7 DiscoverTool/WatchTool shapes land unregistered
  to hold the 16-tool gate; their router registration + budget renegotiation
  rides with this row.)
- [x] MCP10 mcp: EXPLAIN for AI edits — every AI batch gets a dry-run cost
  report (objects touched, dirty subtrees, bytes, nav tiles) and caps; over
  budget it is refused with the legal values, like a database EXPLAIN plus
  statement timeout. Tier A. Size S. Owns `crates/endpoint/src/explain.rs`.
  Accept: 20 adversarial briefs at least 80% refused before apply; p95 at
  most +10%; `play.run` never stalls. Hunt 2026-09-25: every refusal carries the violated invariant and one recovery, like a counterexample (CEGIS idea). needs: MCP8, SP6.
- [x] MCP11 mcp: capability-scoped tool permissions — 5-card taxonomy of
  tool-permission models mapped onto `#[e2_tool]` + EXPLAIN (MIT, PORT
  cite + EVIDENCE row). Tier A. Size S. Owns
  `crates/endpoint/src/tool_capabilities.rs` +
  `crates/endpoint/src/lib.rs` + seam wiring in
  `crates/endpoint/src/door_dispatch.rs`, `crates/endpoint/src/mcp.rs`,
  `crates/endpoint/src/scene_tools.rs` (judge-ordered batch gates
  2026-09-30). Accept: 20 adversarial
  briefs ≥ 80% refused by capability check; p95 ≤ +10%. Kill: float
  math in tick. needs: MCP9, MCP10. (RES-STEAL wave 36 Dive 12,
  reviewed 2026-09-26.)
- [x] MCP15 endpoint: version-pinned tool registry for replays. Tier B.
  Size S. Owns registry hunks beside the MCP9 tool table (tool
  schemas frozen). Accept: v1 replay x2 identical; v2 drift refused
  with receipt; lookup <= 0.1 ms. needs: MCP9, MCP11.
  (PLN46 verdict TAKE; waits MCP11.)
  DONE 2026-10-05: landed mcp15b (REGISTRY_VERSION=1 + digest + fail-closed replay, tools tests 6->9); judges 480 PASS + 489 PASS (land-gate clean).
- [x] MCP28 endpoint: shared guard block for tool writes — `RequireIntent` +
  `RequireDryRunId` (`rcpt_` prefix + 512 cap) + policy-gate + args/intent
  length caps ported as Rust from flax-mcp
  `src/FlaxMcp.Core/Core/Tooling/ToolGuards.cs:40-84` (license MIT, Maxim's
  own code). Tier B. Size S. Owns `crates/endpoint/src/guards.rs`. Accept:
  one Mutating + one Destructive verb behind the gate; bad `rcpt_` refused
  with a named error; old guards call into it. F2P:
  `cargo test -p engine-endpoint --lib guards_format` (fails before: no
  shared format gate). P2P:
  `cargo test -p engine-endpoint --lib build_game` stays green. needs: MCP8.
   (FLAX-MCP HARVEST EB-2026-10-02-S93;
   research/flax-mcp-harvest/kernel-core.md #4.)
- [x] MCP29 endpoint: witnessed-plan + evidence-backed effect receipts —
  every mutating MCP proposal prepares against a witnessed state digest,
  revalidates authority/conflicts, commits idempotently and retains an
  evidence receipt (pre/post digests + goal proof), replayable x2, ported
  as Rust from `Dicklesworthstone/dwarf_fortress_mcp`
  observe-intent-witness-commit-evidence loop (license MIT, shape only,
  never a dep). Tier B. Size M. Owns `crates/endpoint/src/witness.rs`.
  Accept: stale-witness commit refused with a named error; double-commit
  applies once; replay x2 identical receipts. F2P:
  `cargo test -p engine-endpoint --lib witness_plan` (fails before: no
  witnessed-state gate on the door). P2P:
  `cargo test -p engine-endpoint --lib build_game` stays green. needs:
  MCP8. (STEAL 2026-10-03 dwarf_fortress_mcp,
  https://github.com/Dicklesworthstone/dwarf_fortress_mcp.) Landed 2026-10-07 mcp29: F2P witness_plan 4/4, door_dispatch 8/8 with mcp feature (receipt lands + replays once, stale refused), P2P build_game 18/18, clippy -D warnings exit 0, quality 82 B 0 regress, packet-gate 4 paths 0 findings, review PASS builder-ai-door-r5-review-2.
- [x] PV4 provider: streaming with backpressure — streamed model output with
  cancel mid-stream, bounded buffers and a timeout that never touches the
  tick, plus a 10k-tick tape with the provider disconnected that still
  advances and replays offline. Tier B. Size M. Owns
  `crates/provider/src/stream.rs`. Accept: two real adapters stream; cancel
  within 100 ms; the disconnected tape replays x2 with no network. needs: PV2.
- [x] CT15 content: fighter bind + staged fight — load the CT7 bundles in
  WIN1, bind to fighters, run a staged fight and capture it (CT7 judge
  finding 1; CT7 worker's own next step). Tier A. Size M. Owns game
  fighter-bind glue. Accept: staged fight plays with the new characters;
  capture exists; manifest names CT7 rows. needs: CT7, WIN1.
  Wave 44: fighter_bind.rs + staged_fight bin, 600-tick fight
  (27 hits, 3 knockdowns), 4 captures, CT7 SHAs pinned; game 208+9+2
  green; checker PASS, judge mirror-fix verified.
- [x] PV5 provider: conformance matrix — every backend (mock, local, OpenAI,
  Anthropic, Gemini, Ollama, an external MCP client, offline) against nine
  columns: completion, bad output rejected, latency p50/p95, cost (unknown is
  never zero), streaming, cancel under 100 ms, timeout, offline replay x2,
  red-team (RV1). Live columns run when keys exist; recorded fixtures
  otherwise, marked as such. Tier A. Size M. Owns
  `crates/provider/tests/matrix.rs`, `fixtures/provider-matrix/`. Accept: one
  command prints the matrix; the same proposal gives the same transcript,
   preview and fork on every backend (only source, latency and cost differ).
   needs: PV4, RV1.
- [x] RV1 provider: red-team tape — a replayed suite of prompt injections
  through briefs, asset names, scene text and tool results, out-of-scope tool
  calls and secret-exfiltration tries, run every wave (deep dive 12). Tier A.
  Size S. Owns `fixtures/redteam/`, `crates/endpoint/tests/it/redteam.rs`.
  Accept: every case refused or contained with a logged reason; no secret in
  any log; a new case is one fixture file. needs: AH5, MCP5. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-endpoint --test it redteam::` 6 passed (on this PC).
- [x] VK1 playtest: keypoint verifier (GameGen-Verifier, arXiv 2605.07442:
  state injection + bounded play + assertions, 92% vs 59% accuracy) — a
  gameplay claim ("parry opens a riposte window", "ragdoll gets up in 2 s")
  is a keypoint spec: inject state into an FK1 fork, run a bounded input
  script for N ticks, assert on typed state and take one capture. Specs are
  data in `fixtures/keypoints/`, run by proof and by an MCP `sim.verify`
  tool. Tier A. Size M. Owns `crates/playtest/src/keypoint.rs`,
  `fixtures/keypoints/`. Accept: 30 keypoints from shipped moves pass; 5
  seeded gameplay bugs (wrong window, missing hit-stop, stuck state) each
  fail exactly their keypoint; the full set runs in < 60 s. Hunt 2026-09-25: an ablation without state injection must miss at least 3 of the 5 seeded bugs, else injection is not worth its cost; run1 equals run2. needs: FK1, CR1.
- [x] EV2 playtest: brief-to-game eval — 20 briefs run through AB1–AB4; each
  output scored by VK1 keypoints (does it play), a vision rubric on captures
  (does it look finished; the judge reads the PNGs) and a replayed demo tape,
  with a pass rate per stage like GameCraft-Bench (arXiv 2606.17861) and
  CraftBench-UE (arXiv 2609.23142), tracked per build. Tier C. Size M. Owns
  `crates/playtest/src/brief_eval.rs`, `fixtures/briefs/`. Accept: one
  command reproduces the report; a seeded broken stage lowers exactly its
  score. Deep dive 08: rule-based keypoints and diffs gate the score; vision scores are advisory, never the gate; the headline metric is builds that pass asset checks but fail at runtime. needs: VK1, AB4, EV1.
  (Wave 39: 8 eval tests + seeded breaks detected, judge PASS depth 3.)
- [x] VP1 mcp: agent play session — `play.step` holds a live match (window
  build or offscreen GPU), takes inputs for N ticks and returns a state
  summary plus a real capture PNG, so the vision playtester plays turn by
  turn; the capture callback is wired on the GPU path, so `capture` never
  answers `unsupported_capture` when a device exists. Tier A. Size M. Owns
  `crates/endpoint/src/play.rs`, `crates/endpoint/src/play/play_session.rs`.
  Accept: an agent plays 20 steps of a duel through MCP; every step returns a
  non-black PNG (pixel-variance check) and a state digest; the same inputs
  twice give the same digests. needs: MCP2, GX1, WIN1. Tick audit 2026-10-19: path updated to `crates/endpoint/src/play/play_session.rs`.

## SPRINT-05-code-quality.md

- [x] CQ1 tools: Rust quality scorecard, ratchet and gates — per-crate
  scorecard and baseline ratchet (`tools/rust-quality.mjs`), `rust-*` rules
  in `tools/packet-gate.mjs`, 82 new zero-hit workspace lints with
  `#[expect(reason)]` only, `tools/sim-clippy.toml` in 8 simulation crates,
  `engine_core::{hash, alloc_probe}` owners and a universal core edge. Tier A.
  Size L. Owns those files. Accept: tool tests green; clippy all-features
  clean; workspace tests green; golden x2. needs: none.
- [x] CQ2 core: one alloc probe — the hand-rolled `impl GlobalAlloc` probes
  (`git grep "impl GlobalAlloc"`: ai, anim, arena, audio, combat,
  destruction, ecs and more) move to `engine_core::alloc_probe` (the
  orchestrator adds `engine-core = { workspace = true, features =
  ["test-util"] }` dev-dependencies); then `unsafe_code = "deny"` joins
  `[workspace.lints.rust]` with one scoped `#[expect(unsafe_code, reason)]`
  in `crates/core/src/alloc_probe.rs`. Tier A. Size S per crate. Owns each
  crate's probe module. Accept: `--list unsafe` shows only core; every
  zero-allocation test unchanged and green; golden x2. needs: CQ1.
- [x] CQ13 audio: real-time proof — the mix callback renders into caller
  buffers (7 hot allocations, `--list hot_alloc --crate audio`) and is
  proven with `alloc_probe::measure` (0 calls per callback) and, on Linux,
  RealtimeSanitizer via `rtsan-standalone` (license read before adding).
  Tier A. Size S. Owns `crates/audio/src/`. Accept: audio hot_alloc 0; the
  probe test reads 0 per callback. needs: CQ1, CQ2.
- [x] CQ-NET-API1 net: one Beacon owner — delete `lobby::Beacon`,
  `pub use crate::dedicated::Beacon` in `lobby.rs` (payload validation
  maps to `LobbyError`), keep the 18 B layout test green on both sides.
  Tier A. Size S. Owns `crates/net/src/lobby.rs`. Accept: one
  `pub struct Beacon` in `crates/net`; `grep "pub struct Beacon"
  crates/net/src` prints one line; lobby + dedicated tests green;
  golden x2. needs: none. (API-REVIEW wave 54.)
  Wave 54: TICK (dedicated owns, lobby re-exports, 18 B layout green
  both sides, net 176/0, hash_owner_equiv x2, gate + quality PASS;
  one wan_soak wall-clock flake on first run, green on retry —
  BLOCKERS fingerprint).
- [x] CQ-NET-API2 net: typed netcode errors — `Session::confirm` returns
  a typed `NetcodeError` (`TooFar`/`Future` with frame+oldest/current)
  and `check_remote` returns `Result<(), NetcodeError>` matching
  `P2pSession::check_desync`; no `&'static str` errors remain. Tier A.
  Size S. Owns `crates/net/src/netcode.rs`. Accept: str_err 0 for net;
  netcode tests updated to typed asserts and green; golden x2. needs:
  CQ-NET-API1. (API-REVIEW wave 54.)
  Wave 55: TICK (NetcodeError TooFar/Future/UnknownFrame/Desync, net
  strErr 0, 176/0, engine2040 --verify golden x2, gate + quality
  PASS).

## SPRINT-07-accepted-plan-rows.md

- [x] RC1 app: `engine-app` lifecycle crate — builder, registration,
  phase order, extraction, startup/shutdown. Owns `crates/app`. Size M.
  Accept: boots in review; shutdown reverses startup; journaled.
  needs: none.
- [x] RC2 world: world-model decision + bridge — adapters translating
  arena/scene/ecs snapshots for extraction, with a recorded unify-vs-
  bridge verdict. Owns `crates/ecs/src/bridge.rs` (or the chosen home).
  Size M. Accept: one extraction test reading all three states; verdict
  row in EVIDENCE. needs: none.
- [x] RC3 jobs: deterministic fork-join — index-ordered chunks, declared-
  order reduction, order-permutation proof tests. Owns
  `crates/ecs/src/parallel.rs`. Size M. Accept: identical results under
  every chunking incl. single-thread; never in sim hot paths.
  needs: none.
- [x] RC4 schema: versioned component schemas + prefab instantiate + v1
  museum fixture. Owns `crates/scene/src/schema.rs` (+ prefab). Size M.
  Accept: prefab round-trip; v1 loads; unknown version refuses loudly.
  needs: none.
- [x] RC6 core: integer time — `time.rs`: the tick index is the truth and
  the accumulator is integer nanoseconds with the exact 60 Hz step
  (16_666_666 ns plus a carried 2/3 remainder); wall time is quantized at
  poll only and never hashed. The f32 `FixedAcc` stays for the legacy path
  until the new one proves equal. Tier B. Size S. Owns
  `crates/core/src/time.rs`, `crates/core/src/tick.rs`. Accept: measure the
  f32 residual on a 10M-push soak first, then zero drift with the new one;
  same tape twice identical; golden `d57b5c8ae3096614` unchanged. needs: none.
- [x] RC7 core: counter RNG v2 — keyed counter streams (Philox/ThreeFry class,
  Salmon et al., SC 2011; reimplemented) so every fork, island and chunk
  draws `F(key, counter)` with no shared state; new fixtures stop the
  `2k`/`2k+1` seed aliasing; v1 `Rng` stays frozen for the golden. Tier B.
  Size S. Owns `crates/core/src/counter_rng.rs`. Accept: same seed twice
  identical; chunkings 1, 2, 4 and 8 give the same numbers; seeds 2k and
  2k+1 differ; cost at most 2x xorshift or the row is killed with numbers.
  needs: none.
- [x] RC10 ecs: frame arenas — per-phase bump temporaries reset each phase and
  pre-sized SoA columns, std-only, with a counting-allocator gate. Tier B.
  Size S. Owns `crates/ecs/src/archetype.rs`, `crates/ecs/src/arena.rs`.
  Accept: 0 allocations per tick after a 10-tick warmup; checkpoints
  identical; if pre-sized reuse alone gets 80% of the gain, keep that and
  kill the arena. needs: none.
  Wave 52 R2 diagnosis (judge FAIL, reverted): PhaseArena has zero callers
  outside `engine-ecs` + `lib.rs:27` vs `archetype.rs:145` World fork.
  R2 must wire one Schedule/jobs tick caller + resolve the World fork so
  ai/proof/perf exercise the arena path.
  Wave 53: TICK 3ba9bfd (judge 3/4; run_arena_tick wired + fork decision;
  ai/proof/perf migration → RC29).
- [x] RC29 ecs: migrate ai/proof/perf onto archetype World + run_arena_tick
  (RC10-WIRE follow-up: legacy `crate::World` kept byte-stable for the
  golden; WORLD_FORK_DECISION routes new code to the archetype path).
  Tier B. Size M. Owns call-site hunks in `crates/ai`, `crates/proof`,
  `crates/perf` (three crates — one packet per crate per wave, split if
  needed). Accept: all three tick through run_arena_tick + suites green +
  golden x2. needs: RC10. (Judge wave 53.)
  Wave 54: TICK (ai 157 + proof 82 + perf 65 green, golden x2, gates PASS,
  quality PASS all three crates).
- [x] RC14 net: server-authoritative replication — Lightyear-style
  component replication with client/server plugins reimplemented
  (MIT OR Apache-2.0, PORT cite + EVIDENCE row). Tier B. Size M. Owns
  `crates/net/src/replicate.rs`. Accept: component deltas replicate;
  client/server agree x2; replay exact. needs: NET1. (RES-REPO wave 32
  corner 3, reviewed 2026-09-26; wave 38: deltas + agree x2 + replay,
  net 165 green, judge PASS depth 3; EVIDENCE row landed.) Tick audit 2026-10-19: the named net `replicate.rs` is deleted net code that stays deleted until a caller exists (docs/plan/2026-10-19-owner-queue.md).
- [x] ARC1 ecs: spine components — `Transform` (Fixed xyz + yaw),
  `Body` (Fixed vel + radius/height), `Fighter` (hp/stamina/poise +
  state), `AiState` (intent + clock), `AnimState` (loco state + pose
  ref) as SoA columns with `CanonicalBytes`, BTree/index order only,
  zero deps beyond `engine-core`. Tier A. Size S. Owns
  `crates/ecs/src/spine.rs`. Accept: round-trip snapshot bytes x2;
  no `HashMap` in the module (workspace-check); 0 allocs per step
  after warmup. needs: none.
  Done 2026-10-04: `crates/ecs/src/spine.rs` (Fighter column is `Vitals`; pose and gait
  stay f32 by the parity rule in the module doc, everything else Fixed or integer;
  `Spine::snapshot/restore`, `column_digests`); `cargo test -p engine-ecs --lib` 64
  passed, snapshot bytes+hash x2, 0 allocs per step (counting allocator), no
  HashMap; golden unchanged.
- [x] ARC2 ecs: spine schedule — register the five spine systems on
  `schedule::Phase` (PreUpdate inputs, Update rules, PostUpdate
  extract) with duplicate-registration refusal and a fixed-order
  test; asserts the SL4 postcondition (no `systems.rs` import).
  Tier A. Size S. Owns `crates/ecs/src/spine_schedule.rs` (or the
  ARC1 module). Accept: order test green; `cargo test -p engine-ecs`
  green; golden unchanged. needs: ARC1, SL4.
  Done 2026-10-04: `crates/ecs/src/spine_schedule.rs` (`SpineSystem::ORDER`,
  `SpineSchedule` refuses a duplicate and a missing system; order, duplicate,
  missing and SL4 tests green in the 64-test lib run).
- [x] ARC3 arena: parity harness — run legacy step and spine step
  600 ticks on seeded tapes behind the `spine` flag and report the
  first divergent tick and component; golden stays on the legacy
  path. Tier A. Size S. Owns `crates/arena` parity test + flag.
  Accept: harness reports exact-match or exact-divergence on 20 of
  20 seeded tapes; proof time posted. needs: ARC1.
  Done 2026-10-04: `crates/arena/src/parity.rs` behind the `spine` cargo flag
  (off by default, `engine-ecs` optional dep): `parity(config, tape, world)`
  steps the frozen legacy `Sim` and `SpineMatch` in lockstep and compares the
  five columns then the checkpoint hash each tick. `cargo test -p engine-arena
  --features spine --lib parity` 2 passed: 20 of 20 seeded 600-tick tapes
  (2 to 4 fighters, scripted human on slot 0, brains elsewhere) exact match;
  with a 6 m block in the arena the same call names the exact first divergent
  tick and column (Transform) twice. Whole arena suite 106 passed with the
  flag, 99 without; golden path untouched.
- [x] ARC4 arena: spine step — translate tape inputs into spine
  components once, step resolution order (sorted IDs, ties by ID),
  translate checkpoints back to the legacy shape; flag-gated. Ports the
  GA3 DepthBrain steering semantics (foe-seeking + wall-escape,
  `depth/steer.rs`) under the same tie-break. Tier A. Size M. Owns
  `crates/arena/src/spine_step.rs`. Accept: parity harness green x2
  (600-tick hashes equal); PQ2 stuck legs stay empty on spine fixtures;
  PF1 workload within 10% of legacy or killed with numbers. needs:
  ARC2, ARC3.
  Done 2026-10-04: `crates/arena/src/spine_step.rs` (`SpineMatch`, `run_spine`)
  runs the five ARC2 systems over one `Spine` per tick; `crates/arena/src/
  world.rs` adds `MatchWorld` (oriented floor boxes, circle push-out, digest
  folded into the checkpoint only when non-empty). The rule libraries keep
  their machines (combat, locomotion, DepthBrain with its steering, the
  bonded crate), so the spine step moves no rule: GA3 steering is the
  DepthBrain already in the loop. Spine columns own pose, body, vitals,
  intent and gait; checkpoints are rebuilt in the legacy shape. Default track
  only (no physics controllers). Accept: empty-world checkpoints equal the
  legacy ones on 4 seeds x 300 ticks and 20 tapes x 600 ticks; PQ2 stuck legs
  empty on 20 AI-vs-AI spine fights; PF1-shape workload (2 fighters, 600
  ticks, best of 15 batches of 40) spine 1.00x legacy (ignored timing test
  `spine_cost_within_ten_percent_of_legacy`, 0.78x to 1.10x on a loaded box).
  A 2 m wall: 0 of 20 seeded fights put a fighter inside it and all 20 final
  hashes differ from the open arena.
- [x] NV1 nav: navmesh bake + query — walkable polygons from cooker
  geometry (Recast patterns, reimplemented), deterministic bytes x2,
  point/area queries. Owns `crates/nav/src/mesh.rs`. Size M. Accept:
  byte-identical bakes; query fixtures pick correct polygons.
  needs: none.
- [x] NV2 nav: crowd steering + traversal links — separation/arrive LOD,
  link traverse for TV1 vault/hurdle/mantle/climb/drop, rebake hooks
  for DX1 break sets. Owns `crates/nav/src/crowd.rs` (+ links). Size M.
  Accept: probe fixtures pick TV1-expected actions; density test flat
  per-tick cost. needs: NV1, TV1. Tick audit 2026-10-19: accepted as closed by its replacement — REPLACED: `nav/orca.rs` (docs/plan/2026-10-19-owner-queue.md).
- [x] GA1 brain: gladiator depth — phases, feints, stamina-aware pacing
  on the BR1 utility core; squad roles via the director; difficulty
  tension tables. Owns `crates/ai/src/depth.rs` (+ squad). Size M.
  Accept: AI-vs-AI identical twice; no stuck fighters; tension tables
  posted. needs: BR1.
- [x] RA2 render: frame graph executes — resources, transient aliasing,
  barriers, one submit; twin and window run the same compiled graph.
  Owns `crates/render/src/graph.rs`. Size L. Accept: cycle refused
  loudly; ≥80% transient aliasing; llvmpipe parity within tolerance.
  needs: RA1.
- [x] RA3 render: scene draw list with culling + LOD — frustum cull,
  hysteresis LOD, integer-stable order under the draw cap. Owns
  draw-list code beside `present.rs`. Size M. Accept: 300-frame
  capture stable; order bytes twice. needs: RA1, RC4.
- [x] RA4 render: GPU caches with budgets — mesh/texture budgets,
  eviction telemetry, missing texture fails the content gate loudly.
  Owns cache code in `present.rs`. Size M. Accept: overflow fixtures
  evict without error; telemetry names assets. needs: RA1, AS1.
- [x] RA5 render: pipeline cache + async compile — persisted cache,
  0 compiles on second run, last-good on failure. Owns new
  `crates/render/src/cache.rs`. Size M. Accept: cold/warm numbers
  posted; broken-shader drill keeps last-good. needs: RA1.
- [x] RA6 render: staging belts — generational uploads retired after
  in-flight frames; AH2 paths invalidate generations. Owns upload
  code beside `gpu.rs`. Size M. Accept: resize/loss torture clean.
  needs: RA1, AH2.
- [x] RA7 render: shader variants + material hot-reload — define
  manifest with count gate, frame-exact swap through `live.rs`.
  Size M. Accept: variant count gated; ED1 drives one reload.
  needs: RA5, ED1.
- [x] RA8 render: frame diagnostics — per-pass CPU/GPU timestamps
  into PF2 captures, manifest with adapter/driver/scene digest.
  Size S. Accept: costliest pass named; four fields present.
  needs: RA2, PF2.
- [x] AM1 anim: runtime graph as data — nodes compile to a flat eval
  list, fixed order, SoA poses. Tier A. Size L. Owns
  `crates/anim/src/graph.rs`. Accept: graph eval equals hand-rolled
  baseline poses; replay exact x2. needs: AN1.
- [x] AM2 anim: 2D blend spaces (speed/angle) with inertialized
  interruption. Tier A. Size M. Owns `crates/anim/src/space.rs`.
  Accept: no pops on interrupt; exact replay x2. needs: AM1, LM1.
- [x] AM3 anim: body layers — legs base, arms combat, additive
  hit-react. Tier A. Size M. Owns `crates/anim/src/layers.rs`.
  Accept: masked eval equals full-body reference within tolerance;
  mask tables posted. needs: AM1, CB1.
- [x] AM4 anim: event tracks with frame-exact combat/audio callbacks.
  Tier A. Size S. Owns `crates/anim/src/events.rs`. Accept:
  footstep/hit-cue events fire on the exact frame twice. needs: AM1.
- [x] AM6 anim: foot lock — analytic two-bone IK, ground clamp, slide
  budget. Tier A. Size M. Owns `crates/anim/src/feet.rs`. Accept:
  slide under 2 cm on straights; replay exact x2. needs: C4, AM1.
- [x] AM8 anim: exertion/gait layer on LM1 features + TU1 tuning.
  Tier B. Size M. Owns `crates/anim/src/exert.rs`. Accept: tired
  locomotion reads tired on blind review; tuned fight replays
  exactly. needs: LM1, TU1.
- [x] PX2 physics: cloth — CPU verlet capes/banners on fixed-point,
  bone-anchored, presentation only. Tier B. Size L. Owns
  `crates/physics/src/cloth.rs`. Accept: no blow-ups over soak;
  replay exact x2; 0.5 ms budget at 8 fighters. needs: PH1.
- [x] PX3 physics: grapple — servo tracks the opponent ragdoll with
  torque caps and breakaway. Tier A. Size M. Owns
  `crates/physics/src/grapple.rs`. Accept: hash x2 on scripted
  grapple tapes; breakaway within budget ticks. needs: PX1. Tick audit 2026-10-19: accepted as closed by its replacement (docs/plan/2026-10-19-owner-queue.md); the named physics `grapple.rs` is gone.
- [x] PX27 physics: D-CLOTH-1 baseline tape — 50 seeded cape drapes over
  capsule colliders + folded-curtain set at FIXED_DT 60 Hz, seed pinned.
  Tier A spike. Size S. Owns measurement hunks beside
  `crates/physics/src/cloth.rs` + `crates/physics/tests/`. Accept:
  `cargo test -p engine-physics --lib cloth_gap` green; penetration max +
  self-penetration + ms + allocs + hash x2 posted; golden unchanged.
  needs: PH1. (PROPOSED planner-future-r1, D-CLOTH-1.) Tick audit 2026-10-19: the `cloth_gap` filter is gone — `crates/physics/src/cloth.rs` was deleted 3d6ee4a1 and the cloth module was rewritten fresh by RZ7 (331dca1b) with other test names, so the tick stands.
- [x] PX28 physics: one-way cloth-vs-collider coupling (held on PX27 +
  PX12). Tier B. Size S. Owns seam beside `crates/physics/src/cloth.rs`.
  Accept: `cargo test -p engine-physics --lib cloth_couple` green; 1 cm
  in 50 drapes; 0.5 ms; hash x2. needs: PX27, PX12. Tick audit 2026-10-19: the `cloth_couple` filter is gone; the rewritten `crates/physics/src/cloth.rs` covers one-way colliders in `a_capsule_keeps_the_cape_off_the_body` and `colliders_push_to_the_surface_and_leave_outside_points`, so the tick stands.
  (PROPOSED planner-future-r1, held.)
- [x] PX29 physics: folded-curtain self-collision via intercollision sets
  (held on PX28). Tier B. Size S. Owns hunks beside
  `crates/physics/src/cloth.rs`. Accept: `cargo test -p
  engine-physics --lib cloth_self` green; zero self-penetration; hash x2.
  needs: PX28. (PROPOSED planner-future-r1, held.)
  Landed 2026-10-05 (builder-body-r3 + repair, PASS): cloth_self 2/2,
  lib 101 passed 0 failed 2 ignored, couple 481.8 ms <2000, digests
  bit-identical, UNLOCKS VX16. SHA follows in Receipts. Tick audit 2026-10-19: the `cloth_self` filter is gone; the cloth module was rewritten fresh (RZ7 331dca1b), so the named self-collision tests no longer exist; the tick stands.
- [x] PX30 physics: balance-COM hash quantization — `state_hash` folds the
  `BalanceState::com` `f32` triple (canonical bits today, `balance.rs:586`);
  hash integer millimetres instead so FMA or summation-order drift on
  another CPU cannot split the golden. Tier A. Size S. Owns the
  `state_hash` hunk plus a `com_mm` helper in
  `crates/physics/src/balance.rs` (servo math untouched). Accept: `cargo
  test -p engine-physics --lib balance` green; a 1e-4 m COM jitter hashes
  equal, a 1 mm move hashes different; hash x2; golden re-pinned once with
  a HASH-LAW note. needs: PX23. (Boarded 2026-10-01 from PLAN-REVIEW-B.)
  Landed 2026-10-06 (lanes px30-casts + px-casts, PASS): balance.rs as-free
  via meters_to_mm+quantize_mm (3a79a98), cloth/vehicle integer casts as-free
  (8c9aaf9); balance 14/14 + matrix 3/3 + golden d57b5c8ae3096614 x2 on
  master; equiv 14,923,880 equal + digests identical; lead accept physics
  with residual note; HASH-LAW PX30 note stands; lane px30 superseded, never
  landed.
- [x] DX3 destruction: crack spread — sorted flood over baked bonds
  with strain thresholds, DX1 break events out. Tier B. Size M. Owns
  `crates/destruction/src/spread.rs`. Accept: identical spread sets
  twice; anchored chunks stay until their bonds fail. needs: DX1.
- [x] DX4 destruction: debris budgets — per-event caps, sleep islands,
  distance LOD at 8-fighter scale. Tier A. Size M. Owns
  `crates/destruction/src/budget.rs`. Accept: soak report with caps
  held; hash x2. needs: DX1, PF1.
- [x] PX14 physics: cloth + grapple PF1 budget lock — post PX2 0.5 ms
  and servo costs into PF1 workloads with kill bars. Tier B. Size S.
  Owns perf baseline hunks (no new pass). Accept: seeded regression
  fails gate; 8-fighter tick in budget; golden unchanged.
  needs: PX2, PX3, PF1. (PLN27 physics.md, reviewed 2026-09-25.)
  Landed 2026-10-06 (lane px14, PASS): cloth_step 0.5 ms +
  grapple_servo_per_pair 0.1 ms kill bars, 4 gate tests red->green.
  Lead re-ran: cloth_grapple 4/4, quality 91 (A) 0 regressions.
  Full-lib alloc baselines need the recording machine (pre-existing).
- [x] DX10 destruction: break-to-nav dirty-tile contract proof — every
  DX1 break set names exactly the NV3 dirty tiles and rebakes exactly
  those tiles, proven x2. Tier A. Size S. Owns contract test +
  dirty-set records (no new system). Accept: exact dirty tiles;
  versions kept; repath; bytes x2. needs: DX1, DX4, NV3.
  (PLN27 physics.md, reviewed 2026-09-25; waits NV3.)
- [x] MP2 net: 8–16 fighter hardening — bandwidth caps, priority
  deltas, played sessions (not just soak). Tier A. Size L. Owns
  `crates/net/src/scale.rs`. Accept: 8 humans/AI mix over LAN
  converges; 16-bot soak converges; per-client kbps charted.
  needs: MP1, GM2.
- [x] MP3 net: lobby — beacon browser, ready-up, AI-fill slots, typed
  messages. Tier A. Size M. Owns `crates/net/src/lobby.rs`. Accept:
  two machines form a fight from the lobby with no manual addresses.
  needs: MP1.
- [x] MP4 net: replays — record tapes + keyframes, theater playback
  with frame-step and divergence report. Tier B. Size M. Owns
  `crates/net/src/replay.rs`. Accept: any QA1 tape plays; seeded bug
  attributes to its input (CR1 bar). needs: MP1, QA1.
- [x] MP5 net: melee lag compensation — bounded attacker-view window,
  recorded per hit. Tier A. Size M. Owns
  `crates/net/src/lagcomp.rs`. Accept: fair hits under 80 ms scripted
  ping; window in receipts; replay exact. needs: NET1, CB1.
- [x] MP7 net: theater that attributes — record plus keyframe plus
  frame-step over three tapes; every keyframe index resumes through
  the arena snapshot codec; divergence reports match CR1 tick-for-tick.
  Tier A. Size S. Owns `crates/net/src/replay.rs` proof hunks (no new
  file). Accept: three tapes play with frame-step; same first divergent
  tick as CR1 on each; seeded corruption refused loudly. needs: MP4,
  QA1, CR1. (PLN31 multiplayer.md, reviewed 2026-09-25.)
  (done 2026-10-09: c879e3d, VERDICT PASS target/queue/archive-2026-10-07/builder-net-r3-review.md.)
- [x] MP9 net: input-compression experiment — XOR consecutive redundant
  inputs, RLE zero-runs, byte-exact roundtrip on adversarial vectors.
  Tier B. Size S. Owns compression hunks inside the session codec (no
  second codec). Accept: ≥20% kbps saving at 16 fighters with identical
  hashes, else killed with numbers. needs: MP2. (PLN31 multiplayer.md,
  reviewed 2026-09-25.)
- [x] MP11 net: scripted-human bout fixture — human-shaped input
  tapes (reaction jitter, burst loss, one mid-fight drop+rejoin)
  converge headless with full checkpoint equality, no operator
  hardware. Tier A. Size S. Owns fixture hunks in
  `crates/net/src/scale.rs` (no new file). Accept: human-shaped
  bout converges x2; distributions draft-posted for MP6 to confirm
  or correct; 16-bot x600 soak still hashes x2. needs: MP1, MP2.
  (PLN31 multiplayer.md EXTEND, reviewed 2026-09-28.)
  (MP11-A landed 2026-10-10: tape-bytes rebuild, scale 19/19,
  golden d57b5c8ae3096614 stands, quality 94 A. MP6 distributions
  confirm follows as comment-only follow-up.)
- [x] GP1 game: parry/riposte + clash — timed answers with poise
  stakes. Tier A. Size M. Owns `crates/combat/src/exchange.rs`.
  Accept: every attack has a timed answer on scripted tapes; clash
  ties break by ID+poise; replay exact. needs: CB1, HR3.
- [x] GP2 game: continuous swept detection — segment-vs-capsule per
  tick, shield cones. Tier A. Size M. Owns
  `crates/combat/src/sweep.rs`. Accept: thin fast weapons never
  tunnel in fixtures; sorted order proven. needs: CB1, PH1.
- [x] GP3 game: arena camera director — lock-on, trauma shake in
  ticks. Tier A. Size M. Owns `crates/game/src/camera.rs`. Accept:
  scripted fights keep both fighters framed; trauma replays exactly.
  needs: AR2, WIN1.
- [x] GP4 game: modes — ladder table, glory-rank counters, repair-craft
  hooks (LIGHT slice). Tier B. Size M. Owns
  `crates/game/src/ladder.rs` + `content/` tables (split from WIN2's
  display `modes.rs` — same file claimed twice, orchestrator review
  2026-09-24). Accept: ladder runs start to rank; craft hook fires
  once per loop; nothing CUT added. needs: AR1, HR5.
- [x] GP11 game: feel survey framework — Pichlmair game-feel survey
  taxonomy (tuning/juicing/streamlining) reimplemented as engine
  parameter schemas in `crates/combat` + `crates/game` (ideas only,
  license UNVERIFIED). Tier B. Size M. Owns
  `crates/combat/src/feel.rs`. Accept: PQ1 latency ≤ 50ms, PQ2
  balanced across 200 matches. Kill: float math in tick. needs: GP1,
  GP2. (RES-STEAL wave 35 Dive 11 note, reviewed 2026-09-26; wave 37:
  integer schemas + PQ1/PQ2 tests, combat 80 green, judge PASS
  depth 4.)
- [x] GP12 game: FootsiesGym balance harness — fighting-game balance
  testing methodology reimplemented as a falsifiable experiment
  harness in `crates/game` (ideas only, license UNVERIFIED). Tier B.
  Size M. Owns `crates/game/src/bench.rs`. Accept: balance
  conclusions within 5% of reference; PQ2 stable. Kill: float math
  in tick. needs: GP11, GP2. (RES-STEAL wave 35 Dive 11 note,
  reviewed 2026-09-26; wave 38: A/B harness + 5% bound + PQ2 stable,
  game bench 5 green, judge PASS depth 3.)
- [x] GP16 game: camera WIN1 presentation feed — drive the landed GP3
  director from the WIN1 frame loop with trauma recorded in-sim and
  shake applied presentation-side. Tier A. Size S. Owns
  presentation-feed hunks beside `crates/game/src/camera.rs`
  (director API frozen). Accept: scripted 2-fighter orbits keep both
  fighters framed 600 ticks through WIN1; trauma replays exactly x2;
  0.05ms budget held. needs: GP3, WIN1. (PLN gameplay, reviewed
  2026-09-26; wave 40: feed + windowed callers, game 185 green,
  checker PASS.)
- [x] GP13 game: impact-feel instrumentation — Lin et al. 2208.06155v3
  19-feature framework vs the deterministic hit-stop baseline
  (ideas only, license UNVERIFIED). Tier A. Size M. Owns
  `crates/combat/src/feel_instrument.rs`. Accept: 20 combat scenarios
  with hit-stop + camera coordination; PQ2 balance ≥ +10%; p95 input
  ≤ 50ms. Kill: latency rise; float math in tick. needs: HR3, PQ2.
  (RES-ARXIV wave 35 Dive 11, reviewed 2026-09-26; wave 42: 11 tests
  green, judge PASS depth 4; arena consumer open, follow-up.)
- [x] ED2 editor: schema inspector — RC4-driven grid, prefab overrides,
  batched edits. Tier A. Size M. Owns
  `crates/editor/src/inspector.rs`. Accept: every descriptor renders;
  overrides badged; panel tests + Xvfb smoke. needs: E1, RC4.
- [x] ED3 editor: asset browser — bundle search, drag-to-place through
  SP5 + preview. Tier A. Size M. Owns
  `crates/editor/src/browser.rs`. Accept: full-bundle search under
  budget; bad drops refused with conflict sets. needs: E1, AS1.
- [x] ED4 editor: play-in-editor — WIN1 loop in the viewport with
  pause/step/restart over PL1 snapshots. Tier A. Size L. Owns
  `crates/editor/src/play_in.rs`. Accept: same tape as child play
  gives same checkpoints; Xvfb smoke. needs: ED1, WIN1.
- [x] ED5 editor: onboarding — first-run checklist over TPL1 + ED4,
  tutorial-1 flow. Tier B. Size M. Owns
  `crates/editor/src/onboard.rs`. Accept: measured time-to-first-fight
  under 15 minutes; hints only, no docs. needs: ED1, TPL1.
  Wave 43: 6/6 green, budget 600/900 s, panel consumes next.
- [x] ED6 editor: outliner depth — search, selection sets, shared
  selection model. Tier B. Size S. Owns
  `crates/editor/src/outline.rs`. Accept: viewport/outliner/inspector
  agree on one selection type; no mirrors. needs: E1.
- [x] AS2 assets: streaming — async jobs, priorities, memory budget,
  placeholders, frame-boundary publish. Tier A. Size L. Owns
  `crates/assets/src/stream.rs`. Accept: bundle streams under budget
  with zero tick hitches; eviction fixtures pass. needs: AS1.
- [x] CT5 content: runtime LOD swap — hysteresis bands over CT2 chains,
  budget-checked. Tier A. Size M. Owns
  `crates/assets/src/lod_swap.rs`. Accept: no oscillation in fixtures;
  distance swap proven in match captures. needs: CT2, AS1.
- [x] CT7 content: hero and fighters — pick a CC0 or CC-BY hero gladiator and
  three opponent bodies (Quaternius Universal Base Characters first, then
  other CC0 sources in `docs/plan/content.md`), plus the combat and
  traversal clips; cook them through CT1/CT2 onto the hero skeleton within
  the hero 40k and opponent 25k triangle budgets. License read live, an
  `EVIDENCE.md` row per asset (source URL, SPDX, author, SHA); no Mixamo or
  non-commercial data in git. Tier A. Size L. Owns `content/`. Accept: the
  window build fights with the new characters; cook identical twice; every
  asset has a provenance row. needs: CT1, CT2, CT3.
  Wave 43: OGA System G6 CC0-1.0, CT7-OK, provenance in
  content/fighters/manifest.json, live-fight demo rides RL5/Q1.
- [x] CT8 content: arena kit — sand floor, stands, walls, gate tunnel,
  torches, banners, crates and debris from CC0 sources (Poly Haven,
  ambientCG, Quaternius, Kenney), PBR textures up to 2k, one CC0 HDRI per
  time of day; cooked with LODs, collision and `FRAC_` chunks for DX1.
  Tier A. Size L. Owns `content/`. Accept: the arena loads in the window
  build with zero spatial findings and a provenance row per asset; budgets
  held. needs: CT2, AS1.
- [x] CT9 content: map kits for the ludus, the Rome district and the siege
  field — CC0/CC-BY modular buildings, walls, gates, market and camp props,
  same cook, budgets and provenance rules as CT8. Tier C. Size L. Owns
  `content/maps/`. Accept: WD2–WD4 have what they need; provenance complete.
  needs: CT8.
- [x] CT6 content: museum automation — version registry + fixture
  template; gateless releases refused. Tier B. Size S. Owns
  `crates/assets/src/museum.rs`. Accept: a version without a fixture
  fails proof by construction. needs: LT1.
- [x] SN1 audio: reverb zones — parametric tails, tunnel preset first.
  Tier A. Size M. Owns `crates/audio/src/reverb.rs`. Accept: tunnel
  tails on blind listen; 0.4 ms budget; zero audio-thread allocs.
  needs: AU1.
- [x] SN2 audio: occlusion — spatial-index queries per voice per tick,
  smoothed gain. Tier A. Size M. Owns
  `crates/audio/src/occlude.rs`. Accept: walls muffle in fixtures;
  0.1 ms/voice; replay-safe (presentation). needs: AU1, SP3.
- [x] SN6 audio: WIN1 mix wiring for reverb sends + occlusion gains —
  game-thread bridge over the frozen SN1/SN2 APIs (the SN3
  `audio_director.rs` shape), tunnel preset on, voice floor −12 dB,
  tails glued across AH3 reopen. Tier A. Size S. Owns the bridge hunks
  in `crates/game/src/` (audio API frozen). Accept: the window build
  plays tunnel tails + wall muffle; blind wall A/B 4/4; 0
  audio-thread allocs; suites green. needs: SN1, SN2, WIN1. (PLN audio,
  reviewed 2026-09-26; wave 36: bridge + windowed wiring, game 174
  green, checker PASS; the bar is the wall-muffle proxy: charted gains
  asserted in `crates/audio/src/occlude.rs` tests, no human blind A/B.)
- [x] WIN2 game: display modes — enumerate, fullscreen/borderless/
  resolution, honest refusal. Tier A. Size M. Owns
  `crates/game/src/modes.rs`. Accept: mode list matches adapter; swap
  without losing last-good (AH2 paths). needs: WIN1.
- [x] WIN3 game: gamepads through IN1 — sticks as axes, full rebind.
  Tier A. Size M. Owns `crates/game/src/pad.rs`. Accept: pad plays a
  full fight; dead zones proven; no parallel input state. needs: IN1.
- [x] WIN4 game: crash reports — version, adapter, tape tail, checkpoint
  to a local file, loud exit. Tier A. Size S. Owns
  `crates/game/src/crash.rs`. Accept: forced crash leaves a readable
  report; restart clean. needs: WIN1.
- [x] UIX1 ui: menus — main/pause flows over mode/match functions.
  Tier A. Size M. Owns `crates/ui/src/menus.rs`. Accept:
  click-through to a fight with no console; Xvfb smoke. needs: WIN1.
- [x] UIX5 ui: tutorial-1 first fight — guided first fight over UIX1
  commands (move, attack, block, pause) built from TPL1 templates,
  steps as data, skippable. Tier B. Size M. Owns
  `crates/ui/src/tutorial.rs`. Accept: a fresh profile reaches a
  finished fight with no console; the step script replays exactly;
  skip leaves a clean menu state. needs: UIX1, TPL1. (PLN shipping.md,
  reviewed 2026-09-26.)
  Wave 51: packet LANDED (dbe544a, tutorial.rs + hook, 64 green);
  checker PASS, score 81. TICK.
- [x] UIX3 ui: lobby screen over MP3 state. Tier B. Size M. Owns
  `crates/ui/src/lobby_ui.rs`. Accept: lobby-to-fight with no addresses
  typed; refresh budget held. needs: MP3, WIN1.
  2026-10-04: LANDED `crates/ui/src/lobby_ui.rs` (`LobbyScreen`: host list by
  position, join, host, ready, AI fill, start, leave; commands carry host ids
  and flags only, never an address or a string; refresh paced from the net
  lobby's own period through `LobbyLimits`). `cargo test -p engine-ui --lib
  lobby_ui` 12 passed. `cargo test -p engine-game --test it lobby_ui_flow::`
  2 passed: a host screen and a guest screen over the real `engine-net`
  lobby on loopback UDP go from the host list to one identical
  `FightRequest` (seed 777, 4 fighters, 2 humans plus 2 AI) with every address
  taken from a datagram, none typed; ten seconds of 16 ms frames ask 38-41
  refreshes and the net browser accepts every one. clippy -D warnings clean,
  rust-quality ui 99 (A) and game 94 (A), 0 regressions. The windowed loop
  does not draw this screen yet. TICK. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-game --test it lobby_ui_flow::` 3 passed, run 39230103927.
- [x] PQ10 proof: offline MCTS exploit sweep beside the tournament —
  bounded search over the engine forward model, never the tick. Tier B.
  Size M. Owns sweep hunks beside the PQ2 harness (sim frozen). Accept:
  planted degenerate found <= 1 sweep while scripted bots miss; 0 tick
  code; cap holds. needs: PQ2. (PLN48 STEAL dive14 Card 5.) Landed 2026-10-07 builder-speed-r3: exploit_gate wired via EXPLOIT_GATE_NAME + proof.mjs row, killer_of shared (dup back to 3), gate 1/1 + exploit 7/7, lib 198 x2 prior, quality 95 A 0 regress, packet-gate 3 paths 0 findings, review PASS speed-review-3. Note: workspace casts 110→115 pre-exists on master (giant wave, 0 from these files) — 563-playtest-casts queued to burn it.
- [x] SP22 spatial: affordance gold + probe check — pure tag functions of
  collider + nav + anim-set tags with a 50-object gold and a mistag-flag
  probe. Tier A. Size S. Owns `crates/spatial/src/afford.rs`. Accept:
  >= 90% tags + 100% seeded-mistag flags; afford query <= 0.2 ms/object.
  needs: SP7, SP8. (PLN48 reader P1 Card 1; habitat-lab PORT.)
- [x] SP23 mcp: constraint-first brief compile — pure `compile_plan`
  beside `crates/endpoint/src/build_level.rs` tags brief wants by
  category (placement, separation/nav, support/surface, spawn/reach)
  before SP5 solves, validates before the first solve, bounded
  repair per category (Holodeck loop shape reimplemented; no MILP
  port; solver API frozen). Tier A. Size S. Owns compile hunks
  beside `crates/endpoint/src/build_level.rs` (no `place.rs`
  semantics change). Accept: +30% clean-first-pass measured before
  the first repair on the 10 in-file AB1 briefs with <= 3 rounds;
  `level_digest` + `cell_digest` dry-run == applied 10/10, bytes x2;
  proof `cargo test -p engine-endpoint --lib build_level` plus
  `cargo test -p engine-spatial --lib place`, each green x2 identical.
  needs: SP5, SP10. (PLN48 reader P1 Card 2; SP23 VERDICT wave 86:
  frozen-solver rescope, writer builds compile_plan beside build_level.)
- [x] EV18 playtest: retrieval-stress leg deciding AX10. Tier B.
  Size S. Owns distractor harness hunks in `crates/playtest/src/eval.rs`
  + `fixtures/eval-distractors/` (task list frozen). Accept: 3
  authoring tasks x 3 backends with/without 5–10 distractors names ≥1
  failure mode clean EV1 misses or kills AX10 with a no-gain report;
  one command reproduces the report. needs: EV1, EV3. (PLN20 ai-native.)
  (AIDOOR.c landed 2026-10-10: retrieval_dilution mode named, 207 lib
  green, golden stands, quality 95 A; baseline openly accepts eval.rs
  as 3rd big_file, carve-out later.)
- [x] EV19 proof: verify-after-record for the proof record path with
  refuse-partial (rosbag2 sqlite resilient-preset discipline
  reimplemented, Apache-2.0; in-memory truncation refusal already in
  `crates/proof/src/tape.rs`, this row owns the file record path
  only). Tier A. Size S. Owns verify hunks beside
  `crates/proof/src/tape.rs` + the proof CLI `--verify` leg (tape
  format frozen). Accept: kill -9 mid-record refuses verify with the
  partial file named as reason; clean replay x2 byte-identical; record
  overhead ≤ 1% on the 600-tick fixture with numbers posted. Kill:
  record overhead >1% or any partial replay ever accepted. needs:
  EV13. (WILD corner-23 Card 2.)
  (EV19.a+b landed 2026-10-10: file refuse-partial names the path,
  CLI x2 byte-identical at 0.29% overhead, proof lib 81 + proof_cli
  5 green on HEAD, quality 93 A / 91 A.)
- [x] EV20 eyes: wired pixels — one game capture path into `capture_with`
  plus a 15-frame showcase set on the reference GPU. Tier A. Size S.
  Owns capture-wiring hunks in `crates/game/src/camera_feed.rs` (no
  second path). Consumer: Q1 reference captures plus the forge PIE pilot
  per the RomeScene note. Gold task: 15-frame set sha-pinned (manifest
  with per-file sha256 plus bytes, uniform size, PNG magic). Accept:
  15-frame set sha-pinned and green; fail-closed on 0 PNG; golden
  `d57b5c8ae3096614` held x2. needs: EV3. (PLAN eyes.md E1.)
- [x] PF13 perf: frame-time baseline gate on the reference GPU (RTX 3050
  laptop) for the arena — CPU pins plus pending GPU/VRAM rows with the
  adapter line. Tier A. Size S. Owns `tools/perf-baseline.mjs` +
  `tools/perf-baseline.json` hunks (no new file; consumer RT14/VG12/PF15
  spikes plus CT41/CT51/CT61 compare against this file; budget one
  loaded-box session, gate <5 s). Accept: `node tools/perf-baseline.mjs
  --check` PASS (11 rows: 4 baseline CPU pins with ticks/hash/alloc,
  6 GPU passes + VRAM PENDING with box-conditions lines, quiesce rule
  pinned) + `node tools/perf-baseline.mjs --record-observed --out
  target/perf-observed.json` PASS without touching the committed
  baseline + `node tools/workspace-check.mjs` PASS. needs: GX1. (PLN48
  frontier.md.)
  Wave 54: TICK (script + 11-row file, --check PASS; GPU rows pending
  quiesced capture in follow-up).
  Deep-dive 2026-09-30: the bundled gate-plus-quiesced-fill kept burning
  loaded-box helpers on the GPU tail (3 helpers in 72 h, same fingerprint);
  PF13 closes headless on the gate, the quiesced fill rides pilot-only
  PF19 (PF13 → PF19, never the reverse).
- [x] CT26 content: arena PBR close-out (metal + gate/debris stone wiring + authored ORM + report re-baseline + gen clobber gate; hdri/ frozen, LODs/hulls frozen).
  Tier A. Size M. Owns `content/arena/src/` + textures + placement + manifest + `content/arena/report.json` (`content/arena/hdri/` untouched — CT8-HDRI lane; LODs/hulls already proven in the CT8 report, never re-proven here). Accept: `cargo test --locked -p engine-assets --lib` green; spatial 0 findings over the placed kit; every 2k map carries URL+SPDX+author+SHA in `*-PROVENANCE.md` + manifest entry; committed set cooks x2 byte-identical (`crates/assets/src/texture.rs:17` 4096 cap, `:21` 13 mips); LOD0 tris/budgets hold; eyes PASS on each new albedo; `report.json` re-based with golden unchanged; `tools/gen.mjs` clobber gate shut; Q1/RA14 window pixels explicitly deferred to CT45, not required to tick. needs: CT2, CT8.
- [x] CT35 content: fighter LOD + impostor bake for 64-player scale.
  Tier A. Size M. Owns `content/fighters/lod/` + LOD/impostor hunks
  beside `crates/assets/src/lod_swap.rs` (CT5 frozen). Accept:
  `cargo test --locked -p engine-assets lod_swap` + `mesh_cook` green;
  LOD1-3 monotonic with hulls containing; cook x2 identical; P6
  ms/VRAM per LOD posted on the reference GPU; 0 tick allocs. needs:
  CT2, CT5. (EXTEND wave 85.)
- [x] CT51 content: cold-load + stream report for arena + fighters. Tier A.
  Size S. Owns `content/arena/LOAD-REPORT.md` (bundles frozen). Accept:
  cold arena load <= 5 s on the reference GPU; per-bundle bytes + VRAM
  posted with adapter line; bundle loads via AS1; cook x2 identical;
  0 tick allocs. Consumer: CT27 soak, PF13 baseline. needs: CT7, CT8.
  (EXTEND wave 87.)
- [x] CT53 content: arena dressing — banners + torch stands for silhouette.
  Tier A. Size S. Owns `content/arena/dressing.json` + staging glue (no
  pass code; `content/arena/establishing.json` and CT25 crowd kit frozen).
  Accept: capture pair before
  `target/pilot-owner-view/r3-20260929/arena/frame_000014.png` vs after
  on the reference GPU scores higher by this seat on silhouette +
  composition with 0 validation errors; golden unchanged. needs: CT8.
     (LOOK-ART 2026-09-29.)
- [x] CT81 content: placement + dressing joint SP5 audit. Tier A. Size S.
  Owns `content/arena/PLACEMENT-AUDIT.md` (`placement.json` +
  `dressing.json` frozen; no pass code). Accept: 11 placement + 6
  dressing actors with SP5 placement 0 findings; audit names SHAs +
  budgets; bytes x2 identical; golden unchanged. Consumer RA14/Q1,
  CT60 joint proof. needs: CT8, CT53.
- [x] SP38 spatial: minimal-conflict subset audit beneath SP17 hints
   (READER-P1 wave 54). Tier B. Size S. Owns audit hunks beside
   `crates/spatial/src/place.rs` + `repair.rs`. Accept: 60 seeded
   over-constrained intents 100% subset-minimal + 20/20
   permutation-identical ConflictSets. needs: SP17. (P1.)
  Receipt 2026-10-09: landed 049cdd8d (place.rs +168, repair.rs +94, 262 insertions, integer-only audit fns audit_subset/audit_identical/audit_conflict_ids/audit_proposals); engine-spatial lib 216 passed 0 failed, 60/60 subset-minimal + byte-stable x2 + 20/20 permutation-identical + fresh seed77 probe, golden d57b5c8ae3096614 x2, clippy -D warnings exit 0, rust-quality 94 A 0 regressions, packet-gate 2 paths 0 findings; consumer wiring rides later wave SP36/MCP/AX/EV.
- [x] SP55 spatial: minimal-conflict audit baseline tape — 60
  over-constrained wall intents, read-only on the solver. Tier B.
  Size S. Owns oracle-fixture hunks beside
  `crates/spatial/src/place.rs`. Accept: `cargo test -p
  engine-spatial --lib conflict_audit_baseline` green; no-shrink 23/60
  vs shrink minimality, perm 20/20, digest x2, golden unchanged; feeds
  SP38. needs: SP7, SP10. (PLN future-spatial 2026-09-30, Holodeck
  Apache-2.0 constraint_plan shape reimplemented integer-only.)
- [x] SP54 spatial: solver-computed repair hints for place conflicts
  (library: hints beside ConflictSet plus dry-run diff; landed
  1effde8, 159/0 green). Tier A. Size S. Owns hint fields in
  crates/spatial/src/place.rs (RepairHint, ConflictSet.hints)
  plus dry_run_diff in crates/spatial/src/diff.rs beside the frozen
  SP3/solve API. Accept: cargo test -p engine-spatial green plus 20
  seeded conflicts 20/20 hints verify-clean plus 10/10 dry-run
  equals applied plus golden d57b5c8ae3096614 x2 plus brief digest x2.
  needs: SP7, SP10. (Re-ticked wave103: library proof in hand,
  judge PASS RUST 4/5 DEPTH 3/5; wiring rides SP17.)
- [x] MCP18 endpoint: `SetMaterial` end-to-end (BUILD BREAK, wave-49 API
  review: `cargo check -p engine-endpoint --features mcp` fails E0004 at
  `explain.rs:277` + `scene_tools.rs:163`; `OpDto` has no `SetMaterial`
  variant so `tx.preview/apply` cannot express material edits). Tier A.
  Size S. Owns `crates/endpoint/src/explain.rs` arms,
  `scene_tools.rs` `check_batch_allowed`, `mcp.rs` `OpDto` + op-name map
  (after DOCS-R2 lands — same files). Accept: mcp-feature check green;
  `tx.preview→apply` round-trips a `SetMaterial` op; allow-list decision
  tested. needs: none. (API review wave 49, verified by orchestrator.)
  Wave 49: commits bcf7900 + fb18a4f; mcp check green, round-trip +
  allow-list tested, endpoint all-features 298/0, manifest 32714/32768,
  full proof PASS golden x2. TICK.
- [x] RA15 render: draw-storm scale proof — 1,000-agent fixture with
  the 256 cap + instancing budget, cull under 1 ms. Size M. Owns
  `crates/render/src/draw.rs`. Accept: `cargo test -p engine-render
  --lib draw` green. needs: RA3.
  (PLN render-frame.md EXTEND, proposed 2026-09-29.)
  Wave RB 2026-10-05: LANDED 71c5588 (instanced storm fixture, draw 25/1i green on tree 6a8447b; judge 477 PASS RUST 4/5 DEPTH 3/5). TICK.
- [x] CB5 combat: visible hit reactions — kp01/kp05/kp06 each show
  hit-stop freeze + flash + damage number in captures; P2 same-tick
  window met. Tier A. Size S. Owns reaction hunks beside
  `crates/combat/` fight step (CB4 layers frozen). Accept: contact
  fixtures show freeze + flash + number; sim bytes x2. needs: CB1, PQ1,
  VK1. (PLAYTEST56 B3 wave 56.)
  Landed 2026-10-06 (owner ce21f2a + lead dedup, 2× judge PASS): `reaction.rs` 618 lines 7 tests + `feel::capture_input` dedup (dup_fns 1→0). Lead re-ran on applied tree: reaction 7/7, full lib 182, quality 96 (A) 0 regressions, packet-gate PASS. needs CB1/PQ1/VK1 ticked. Residual: proof via capture-harness fixtures, no in-game PNG yet — wiring row to follow.
- [x] QA4 playtest: white-screen gate on real captures — seeded white
  faults plus black/missing/clipping caught on twin and play-step PNGs,
  run in proof. Tier A. Size S. Owns white-check hunks beside
  `crates/playtest/src/frame_check.rs` + gate wiring (QA2 fixtures
  frozen). Accept: seeded white + black faults each caught; clean
  arena/twin/play-step frames pass; the gate fails proof on a seeded
  fault. needs: QA1, VP1. (PLN49 R6.)
- [x] MCP19 endpoint: kill secret-leaking `Debug` — `VaultSecrets` /
  `LaunchSpec` manual redacting impls; audit test asserts no value leaks.
  Tier A. Size S. Owns `crates/endpoint/src/secret_launch.rs` Debug
  impls (MCP18 first). Accept: 20-canary suite + redaction test green;
  M-PUBLIC-DEBUG holds. needs: MCP18. (API review wave 49.)
  Wave 55: redacting Debug + redaction test, secret_launch 10/0, packet-gate PASS, judge PASS, rust-quality 79 PASS 0 regressions. TICK.
- [x] MCP24 endpoint: close the `scene.place` roots-only + yaw gap —
  world-to-local helper or loud refusal, never silent flatten. Tier B.
  Size S. Owns `scene_tools.rs` snapshot/yaw (MCP18 first). Accept:
  parented-node test documents placement or refusal; yaw round-trip.
  needs: MCP18. (API review wave 49.)
  (AIDOOR.b landed 2026-10-10: loud parented refusal + pure-Y yaw,
  F2P + full mcp lib 448 green in lane, quality 89 B. gym.rs hunk
  excluded from landing: already on master via 440ad21d.)
- [x] PF15 perf: secondary tick-frame marks + plot series — tick-vs-frame
  p50s beside the PF2 FrameReport. Tier A. Size S. Owns mark/plot hunks
  beside `crates/perf/src/profiler.rs` (report shape frozen; lands after
  PF13). Accept: 600-tick tick-vs-frame p50s + plot read clean, hashes
  x2; numbers join the PF13 baseline file. needs: PF2, PF13. (PLN50
  wave-49 REPO card 2; rust_tracy_client reimplement-only, no server.)
- [x] PF17 perf: committed-baseline re-record receipt — an honest PASS
  plus a poisoned-only trip plus a committed workload+baseline tamper
  currently PASSES after commit (ADV50 wave 50: GP5 dirty-tree tripwire
  goes green on commit). Tier A. Size S. Owns the receipt/check beside
  `crates/perf` baselines (seal shape is the perf lane's call).
  Accept: committed tamper fails loudly; clean regens pass; golden
  unchanged. needs: PF1. (ADV50 R1.)
  Wave 51: packet LANDED (227fc3f, in-code seal + 4 gate tests, perf
  suite green); checker PASS. TICK.

## SPRINT-08-hard-stuff.md

- [x] AM9 anim: full-body IK — reach, look-at, aim, two-hand and shield
  grips; analytic chains first, FABRIK with a fixed iteration count as
  fallback. Tier A. Size L. Owns `crates/anim/src/fbik.rs`. Accept: hand
  error under 1 cm on reach fixtures; fixed cost per character posted;
  replay exact x2. needs: AM1, AM6.
- [x] AM10 anim: procedural hit reactions — additive flinch from hit
  direction and impulse through the PX1 partial ragdoll; no clip per
  direction. Tier A. Size M. Owns `crates/anim/src/hit_react.rs`. Accept:
  8 hit directions read correctly on blind review; replay exact x2. needs:
  AM3, PX1.
- [x] AM12 anim: faces — GPU morph targets, jaw, eye and brow rig, visemes
  baked offline from voice lines (LIGHT). Tier B. Size M. Owns
  `crates/anim/src/face.rs`. Accept: announcer and taunt lines lip-sync on
  blind review; morph cost posted. needs: R3, AM4.
- [x] AM16 anim: eval determinism + allocation audit — `graph.rs`/
  `space.rs`/`layers.rs`/`events.rs`/`feet.rs`/`fbik.rs`/`hit_react.rs`
  replay x2 plus snapshot/restore equality on scripted tapes, with 0
  per-tick heap allocs on the eval path. Tier A. Size S. Owns audit
  hunks (no new file, no order change). Accept: eval x2;
  restore-then-eval equal; 0 allocs per tick after warmup; golden
  unchanged. needs: AM1, AM3. (PLN28 animation.md, reviewed 2026-09-25.)
- [x] AM17 anim: readable locomotion — `mx`/`mz`/`facing` drive a facing
  arrow + lean with velocity continuity; foot-slide <1 cm, no
  snap-together on the playtest56 seed95 ticks20-30. Tier A. Size S.
  Owns locomotion-presentation hunks beside `crates/anim/` (LM1 search
  frozen). Accept: seed95 capture shows arrow + lean, slide <1 cm, no
   snap; sim bytes x2. needs: LOC1, LM1, PQ1. (PLAYTEST56 B4/B5 wave 56.)
- [x] AM19 anim: eval determinism + allocation audit part 2 — extend the
  AM16 proof over `springs.rs` and every module that lands after it, with
  0 per-tick heap allocs on the springs path. Tier A. Size S.
  Owns the audit hunks (no new file, no order change). Accept: `cargo
  test -p engine-anim --lib` green including the springs
  determinism/alloc suites; counting allocator reads 0 per tick after
  warmup on every eval path; golden unchanged. needs: AM16.
  (PLN animation.md EXTEND, proposed 2026-09-29.)
- [x] AM20 anim: hit-react/exertion ordering proof — flinch magnitude stable
  across the AM8 fatigue sweep with the fixed layer order (IK first, react
  after). Tier A. Size S. Owns audit hunks beside
  `crates/anim/src/hit_react.rs` (AM10 shape frozen). Accept: `cargo test
  -p engine-anim --lib react_order` green; magnitude within band rested to
  exhausted; slide bars hold; replay x2; golden unchanged. needs: AM8, AM10.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.)
- [x] AM23 anim: viseme-bake drift gate — bake-only tables never touch sim
  bytes, morph cost pre-posted for AM12. Tier C. Size S. Owns gate test
  beside `crates/anim/src/viseme.rs` (bake-only, no runtime change).
  Accept: `cargo test -p engine-anim --lib bake_drift` green; sim bytes x2
  with bakes on; cost table posted; golden unchanged. needs: AM4.
  (PLN animation.md EXTEND r7, proposed 2026-09-29.)
- [x] PX4 physics: balance and stumble — powered ragdoll with
  center-of-mass balance, stagger, stumble, fall-and-catch and shove
  response (SIMBICON lineage, reimplemented; Euphoria-class feel). Tier A.
  Size L. Owns `crates/physics/src/balance.rs`. Accept: a shove at 5
   strengths gives stagger, stumble or fall in order; recovers within budget
   ticks; hash x2. needs: PX1, AM10. (Split into PX23–PX25 S slices,
   2026-09-29; this L stays open until PX25 closes it.)
  2026-10-04: CLOSED by PX25 (ticked). Re-run on today's tree:
  `cargo test --locked -p engine-physics --lib balance` 13 passed
  (`balance_matrix_orders_five_strengths_within_budget`,
  `balance_stagger_threshold_orders_five_strengths`,
  `balance_stumble_gait_recovers_without_downgrading_fall`,
  `balance_stagger_hash_stable_x2`) and `--test balance_matrix` 3 passed
  (`matrix_orders_five_strengths_and_recovers_within_budget`); logs
  `target/evidence/px4/`. Golden `d57b5c8ae3096614` x2 via
  `node tools/proof.mjs --skip-build` the same day; no sim file changed. TICK.
- [x] PX23 physics: balance core + COM + stagger — `BalanceState` plus
  COM from bone masses, stagger threshold, reads AM10 `HitDir` + mask
  into the PX1 servo (SIMBICON lineage, reimplemented; no donor code).
  Tier A. Size S. Owns `crates/physics/src/balance.rs` core hunks
  (state + COM + stagger only). Accept: `cargo test -p engine-physics
  --lib balance_stagger` green; COM within 1 cm; stagger at 2/5; hash
  x2; golden unchanged. needs: PX1, AM10.
  (PLN physics.md, proposed 2026-09-29.)
- [x] PX24 physics: stumble + fall-and-catch + get-up — stumble gait,
  fall catch, blend into the PX1 60-tick get-up (presentation never
  enters the hash). Tier A. Size S. Owns `crates/physics/src/balance.rs`
  stumble/fall hunks only. Accept: `cargo test -p engine-physics --lib
  balance_stumble` green; stumble at 3–4/5, fall at 5/5, get-up within
  60 ticks; hash x2. needs: PX23. (PLN physics.md, proposed 2026-09-29.)
- [x] PX25 physics: shove-response matrix + budget proof — 5-strength
  ordered stagger/stumble/fall, recovery within budget, P2 bar.
  Tier A. Size S. Owns `crates/physics/src/balance.rs` response hunks +
  `crates/physics/tests/balance_matrix.rs`. Accept: `cargo test
  -p engine-physics --lib balance_matrix` green; 5 strengths order
  correctly; hash x2; golden unchanged; closes PX4. needs: PX24.
  (PLN physics.md, proposed 2026-09-29.)
- [x] PX33 physics: PD drive law for the powered servo (steal, MIT OR
  Apache-2.0: `bhubbard/active-ragdoll-rs@a2fc2f1` `src/pd.rs`, license
  read live in `Cargo.toml` 2026-10-03; ideas reimplemented in integer
  fixed point, no bytes copied) — D-term on the pose error, a magnitude
  clamp on the output, and a dt-guard that falls back to P-only. Tier B.
  Size S. Owns drive hunks beside `crates/physics/src/powered.rs`
  (`DriveParams`) + `crates/physics/tests/it/pd_drive.rs`. Accept: F2P:
  `cargo test -p engine-physics --lib pd_drive` (fails before: P-only
  velocity servo) — fallen-chain recovery settles in at least 15% fewer
  ticks than P-only at matched overshoot, a degenerate dt returns the P
  term with no NaN, digest x2 identical. P2P: `cargo test -p
  engine-physics --lib balance_matrix` stays green and the golden hash is
  unchanged. Kill: under 15% or any digest drift. needs: PX25.
  (Source `docs/research/steal-hunt-2026-10-03-29.md` CARD 1; focus
  ragdoll.) Landed 2026-10-07 builder-body-r14: F2P pd_drive 6/6 + rapier 5/5, P2P lib 174 passed 0 failed + balance_matrix 3/3, golden d57b5c8ae3096614 unchanged x2, quality 100 A 0 regress, review PASS builder-body-r14-review-2. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-physics --test it pd_drive::` 4 passed, run 39231145288.
- [x] PX34 physics: stagger memory (steal, MIT: `blugart-dev/kickback@1d2c5ae`
  `addons/kickback/active_ragdoll_controller.gd:71-80` and `:347`, LICENSE
  read live 2026-10-03; GDScript, ideas only, reimplemented in integer
  fixed point) — three terms `balance.rs` lacks: cumulative pain that
  escalates flinch to stagger to ragdoll with no dice, fatigue that lowers
  servo strength on repeated hits and decays slowly, and per-bone injury
  that persists after the spring recovers (a leg hit leaves a limp). Tier B.
  Size S. Owns spike hunks beside `crates/physics/src/balance.rs` +
  `crates/physics/tests/stagger_memory.rs`. Accept: F2P: `cargo test -p
  engine-physics --lib stagger_memory` (fails before: no pain, fatigue or
  injury state) — 10 hits in 2 s escalate flinch, stagger, ragdoll in that
  order, a leg hit leaves a limp offset that decays inside its budget,
  digest x2 identical. P2P: `cargo test -p engine-physics --lib
  balance_matrix` stays green (PX25 order unchanged) and the golden hash is
  unchanged. Kill: the PX25 order changes or any digest drift.
  needs: PX25. (Source `docs/research/steal-hunt-2026-10-03-29.md` item 2
  of the rejects, re-opened with the `path:line` it asked for; focus
  ragdoll.) Landed 2026-10-07 px34-stagger-memory: F2P stagger_memory 5/5, P2P lib 168 passed 0 failed + balance_matrix 3/3, golden d57b5c8ae3096614 verified x4 (builder x2, judge, lead), quality 100 A 0 regress,    review PASS builder-body-r15-review.
- [x] PX36 physics: capture-point servo bias — CP error from PX23 COM plus
  support polygon biases stance/ankle/hip servo targets (reimplemented ideas
  from 1710.10598v1 capture-point feedback, no donor code; torque-level CMP
  and MPC variants do NOT transfer). Tier B. Size S. Owns bias hunks beside
  `crates/physics/src/balance.rs` (PX1 servo frozen). Accept: F2P:
  `cargo test -p engine-physics --lib balance_matrix` green with
  recovery-within-budget beating the PX24 baseline by a posted number on the
  5-strength shove set (fails before: no CP bias). P2P: `cargo test -p
  engine-physics --lib` stays green, digest x2 identical on shuffled
  insertion, golden `d57b5c8ae3096614` unchanged. Kill: any hash drift from
  the bias, any float in an event/checkpoint path, any per-tick allocation.
  needs: PX23, PX1. (RES-ARXIV 677-res-ragdoll3, 2026-10-08.)
  (PX36 landed 2026-10-10: 165→148 ticks, full lib 145 green on HEAD
  incl. the stale-base flake passing fresh, quality 100 A.)
- [x] DT5 distill: dataset provenance — training refuses any dataset without
  a manifest (source, license, digest, what it may train). Tier A. Size S.
  Owns `crates/distill/src/data.rs`. Accept: a dataset without a manifest or
  with an unreadable license is refused with a typed error; every shipped
  `.e2nn` lists its datasets. needs: DT1.
- [x] DT4 distill: replay buffer — episodes from fork matches feed an offline
  learner for PX5 teachers and students (online RL stays out of the tick).
  Tier B. Size M. Owns `crates/distill/src/replay.rs`. Accept: the same seed
  gives the same buffer x2; a student trained from it beats the static slice
  on the DT3 tournament, else killed. needs: DT2, FK1.
- [x] PX6 physics: deterministic parallel islands — Rapier islands split
  into RC3 chunks with a sorted merge (Rapier's own parallel feature does
  not keep enhanced determinism). Tier A. Size L. Owns
   `crates/physics/src/islands.rs`. Accept: the same hash under 1, 2, 4 and
   8 threads; at least 2x on a 64-fighter scene. needs: PH1, RC3.
- [x] PX7 physics: physics LOD for armies — far soldiers as capsules only,
  near ones get full ragdoll, CCD only for fast weapons and projectiles,
  sleep islands. Tier A. Size M. Owns `crates/physics/src/lod.rs`. Accept:
  500 AI soldiers plus 64 fighters under 4 ms physics on the reference CPU;
  hash x2. needs: PX1, PF1.
- [x] PX8 physics: character controller v2 — steps, slopes, moving
  platforms, ledge detection feeding TV1, fighter push and crowd shove
  (bevy-tnua and bevy_ahoy patterns, license read first). Tier A. Size M.
  Owns `crates/physics/src/controller2.rs`. Accept: step, slope and
  platform fixtures pass; ledges reach TV1 probes; replay exact x2. needs:
  PH2, TV1.
- [x] DX5 destruction: runtime fracture — seeded Voronoi cuts at the impact
  point with capped chunk counts, break events on the DX1 shape, for siege
  walls and big props. Tier B. Size L. Owns
  `crates/destruction/src/fracture.rs`. Accept: identical chunk sets x2
  from the same hit; chunk cap held; 1 ms per fracture event. needs: DX4. (Island split dropped from the needs 2026-10-04: it is arena integration, not an input of the fracture function.)
  Done 2026-10-04: `crates/destruction/src/fracture.rs`: `fracture_at(shell, hit, FractureSpec{cells, radius_m, seed})` clips the prop shell by the
  bisector half-spaces of seeded Voronoi sites (site 0 = the hit, 5/8 of the rest within the radius, the rest over the shell), nearest
  neighbor first; cells 2..=64 refused outside; `Fracture::break_events(tick)` gives the DX1-shaped BondBroken then ChunkFreed events
  through `to_bonded_desc` + `sever_chunk`. `cargo test -p engine-destruction --lib fracture` 7 green (1 ignored timing): same hit,
  spec and shell give identical chunk bytes x2 (digest `e8370912b2c4e50a`, 24 cells); chunk cap held for every cell count 2 to 64; bad cells,
  radius, hit and shell refused; piece area equals the shell area within 0.1% on 10 seeds. Budget (release, loaded box, 200 events
  on a 4 x 3 x 0.4 m wall): 16 cells p50 248 us p99 727 us (the arena admits 16 shards per event), 24 cells p99 995 us, 32 cells
  p99 1.4 ms, 64 cells p99 2.7 ms: larger events exceed 1 ms and should come from the NS2 cache. PX6 (parallel islands) not needed for the
  function; arena wiring open. Golden unchanged.
- [x] PX10 physics: continuous collision — the CCD solver is built but no body
  enables it (`ccd_enabled` appears 0 times in crates/), so a fast sword can
  tunnel through a shield. Turn CCD on for weapons and fast limbs, and read
  contacts during the step but mutate after it (Jolt `ContactListener`
  pattern, MIT, patterns only). Tier A. Size S. Owns
  `crates/physics/src/powered.rs`, `crates/physics/src/ragdoll.rs`. Accept: a
  40 m/s blade never passes a 5 cm shield in 1,000 seeded swings; hash x2;
  step cost within the PF1 budget. needs: PX1.
- [x] PX31 physics: substepped deterministic solver schedule — integrate,
  solve contacts/joints, sleep phases with an explicit substep count from
  `avianphysics/avian` `src/dynamics/solver/mod.rs` (`SolverSchedulePlugin`
  + `XpbdSolverPlugin`, license Apache-2.0/MIT dual). Tier A. Size M. Owns
  `crates/physics/src/substep.rs`. Accept: phase order + substep count
  recorded in the hash fixture; 600-tick tape x2 identical; joint drift
  within tol. F2P: `cargo test -p engine-physics --lib substep_order`
  (fails before: ordering only implicit). P2P:
  `cargo test -p engine-physics --lib balance_matrix` stays green. needs:
  PX1. (STEAL 2026-10-02 engine2040.md #3; EB-2026-10-02-S93.) Tick audit 2026-10-19: accepted as closed by its replacement (docs/plan/2026-10-19-owner-queue.md); the named physics `substep.rs` was deleted 3d6ee4a1.
- [x] PX12 anim: DAT penetration-free coupling for active ragdoll — Divide
  and Truncate (arXiv 2604.15513v1, idea only, reimplemented) as a coupling
  seam for animated-vs-sim bodies. Tier B. Size M. Owns the coupling seam
  in `crates/physics/src/ragdoll.rs`. Accept: hash x2 through a 600-tick
  knock/drive/stand fixture; zero penetration events; get-up within budget
  ticks, else killed with numbers. needs: PH3, PX1.
- [x] NV3 nav: dynamic navmesh tiles and flow fields — tiles rebake when
  walls break (rerecast patterns, license read first), flow fields steer
  legions. Tier A. Size M. Owns `crates/nav/src/tiles.rs`. Accept: a broken
  wall opens a path within one second of ticks; 500 agents under 1 ms;
  bytes x2. Truth (2026-09-25): `crates/nav/src/rebake.rs` is an `Unsupported` stub today; this row replaces it (recastnavigation Zlib and rerecast Apache-2.0 allow a PORT with rows). Rebake ≤ 50 ms per dirty tile at p95. needs: NV2, DX1.
- [x] NV7 nav: version-stamped paths over NV3 tiles — dirty-tile bump plus
  stale-path repath exact. Tier A. Size S. Owns tiles.rs version hunks.
  Accept: dirty-tile bump plus stale-path repath exact; bytes x2.
  needs: NV3. (PLN30 game-ai.md, reviewed 2026-09-25.)
- [x] GA2 brain: legion AI — formations, shield wall, flanking and squad
  orders, with thinking LOD for 500 soldiers. Tier B. Size L. Owns
  `crates/ai/src/legion.rs`. Accept: a shield wall holds against a charge
  in fixtures; 500 soldiers under 1.5 ms of AI; identical twice. needs:
  GA1, NV3.
- [x] GA6 brain: EQS cover/flank queries — flax-mcp cover-spot
  generator, alignment/sightline tests and flank scoring rewritten in
  Rust as a `crates/ai` EQS module (MIT, PORT cite + EVIDENCE row).
  Tier B. Size M. Owns `crates/ai/src/eqs.rs`. Accept: generator
  emits standing spots; flank test scores flank points; tactics
  composes cover/ambush queries with GA1; fixtures green. Kill: float
  math in the tick; divergence. needs: GA1, GA2.
  (RES-STEAL wave 32 flax-mcp, reviewed 2026-09-26; wave 37: 15 eqs
  tests + GA1 seam x2, ai 156 green, judge PASS depth 4; arena
  wiring honestly unwired.)
- [x] PF3 perf: release profile — fat LTO, one codegen unit, abort on panic
  for the game exe, PGO trained on a recorded arena tape. Tier A. Size S.
  Owns the release profile proposal (orchestrator lands `Cargo.toml`).
  Accept: A/B on PF1 workloads; keep only if at least 10% faster. needs:
  PF1. (Wave 32: CONDITIONAL KEEP — arena -32% clears bar, hashes
  identical; LTO-fat + CGU-1 landed without panic=abort/PGO; physics/AI
  wins need quiet-machine rerun.)
- [x] PF11 perf: baseline append-only enforcement — editing a
  `crates/perf/src/baselines.rs` row without a same-packet workload-hash
  change fails `node tools/proof.mjs` (ADV1 finding 2: the automated
  budget gate alone cannot tell honest FAIL from poisoned PASS; only
  GP5 diff review catches it today). Tier A. Size S. Owns
  `crates/perf/src/baselines.rs` hunk + workspace-check rule
  (orchestrator lands `tools/` edits). Accept: the ADV1 poisoned-row
  demo trips the new gate; honest re-records with hash receipts still
  pass. needs: PF1. (ADV1 wave 32, fingerprint
  `adv1-baseline-poison-arena_tick-plus1-blind-to-check_report-caught-only-by-GP5-review`;
  wave 34: exact-gate tests + destruction re-record 32532/1321→3116/29
  + presence rule, judge-checked.)

## SPRINT-09-empire.md

- [x] GE2 proof: genre-assumption ledger — every `Fighter|Gladiator|Arena`
  name plus every fighter cap, unit and camera/mode assumption in engine
  crates below `arena`/`game`, each with a verdict (contract site, move,
  or budget-shape) and the row that moves it. Tier A. Size S. Owns the
  ledger section in `docs/plan/general.md` plus the `workspace-check` grep
  gate (orchestrator lands `tools/` edits). Accept: zero unlisted genre
  names in engine crates; gate fails on a seeded new assumption. needs: none.
- [x] GE3 input: one canonical participant cap — input owns
  `MAX_PARTICIPANTS`; arena, combat, net, ui and game read it and delete
  their private caps; C1 types byte-identical. Tier A. Size M. Owns
  `crates/input/src/fighter.rs` plus the cap deletions. Accept: 4-fighter
  golden identical twice; 16-slot validation green; grep shows one cap.
  needs: GE2.
- [x] LW1 ai: NPC mind contract — memory stream (observe, recall, reflect),
  goals and relationships as engine-owned types; async model requests with
  deadline and scripted fallback; accepted outputs recorded as actions or
  lines. Tier C. Size L. Owns `crates/ai/src/mind.rs`. Accept: mock, local and
  remote adapters pass one conformance suite; a replay with providers off
  reproduces every checkpoint. needs: C2, PV3.
- [x] LW2 ai: memory store — bounded per NPC, integer salience and recency
  scoring, deterministic order, reflection summaries recorded. Tier C.
  Size M. Owns `crates/ai/src/memory.rs`. Accept: same event log gives the
  same memory bytes x2; cap held under a 10k-event soak. needs: LW1.
- [x] LW4 playtest: living-world eval — 20 conversation and goal fixtures
  across mock, local and remote: consistency, contradiction rate, latency,
  replay exactness. Tier C. Size M. Owns `crates/playtest/src/living.rs`.
  Accept: one command reproduces the report. needs: LW2, EV1.
- [x] LW6 ai: provenance for every living-world and AI write — a record of
  observed tick, recorded sequence, source and request id per change, so
  every AB and LW edit can be traced and undone. Tier B. Size S. Owns
  `crates/ai/src/provenance.rs`. Accept: 100% of AB and LW writes carry a
  record; undo restores the digest; the record survives save and load.
  needs: LW1, EV1.
- [x] LW8 ai: tick-boundary apply — validated `MindAction`s become typed
  effects at the drain: Say/Gesture emit presentation events only,
  Approach/Leave become movement intents for the existing brains, stamps
  outside `[issued, deadline]` refuse. Tier B. Size S. Owns
  `crates/ai/src/apply.rs`. Accept: 20 scripted asks apply identically
  twice with providers off; a Say action never changes a body position;
  late answers stay history. needs: LW1, LW2. (Wave 32 CHECK FAIL: drain
  lands tested but no brain/presentation caller imports it — retry wave 33
  wiring a real consumer.)
- [x] AB1 mcp: brief to level — a text brief becomes a relation plan, SP5
  placement and a cell layout with nav checks, then preview, apply and undo.
  Tier C. Size L. Owns `crates/endpoint/src/build_level.rs`. Accept: 10 briefs
  produce levels with zero spatial findings and navigable routes; every one
  undoes cleanly. Deep sweep 2026-09-25: actor proposes parameters, deterministic gate findings feed back, at most 3 repair rounds (dual-agent 2512.10501), else refuse with the findings. needs: WD1, SP5, EA1e.
- [x] AB2 mcp: brief to character — assemble a hero or NPC from licensed kit
  parts, CT1 retarget and RL6 material layers, with provenance rows. Tier C.
  Size L. Owns `crates/endpoint/src/build_character.rs`. Accept: 5 briefs give
  rigged, animated characters that pass import gates; unlicensed parts are
  refused. Motion 2026-09-25: a character brief that needs a move the library lacks asks CT14 for it and runs HR6 before the move is used. needs: CT1, RL6.
- [x] AB3 mcp: brief to encounter — AI roster and HR3-validated move tables,
  balance-checked by EA1c fork play-tests before apply. Tier C. Size M. Owns
  `crates/endpoint/src/build_encounter.rs`. Accept: a balance-breaking brief
  is refused by its fork; 10 briefs land inside win-rate bounds. Deep sweep 2026-09-25: brief to closed goal enums, a pure expander, validate_tables, FK1 fork asserts, at most 5 typed repairs then refuse with the conflict set; difficulty rungs with hysteresis and dwell. needs: EA1c,
  GA1.
- [x] AB4 mcp: brief to playable slice — chain AB1, AB2 and AB3 with the WD6
  mode framework into a packaged playable build. Tier C. Size L. Owns
  `crates/endpoint/src/build_game.rs`. Accept: 10 briefs produce builds that
  boot, play 5 minutes and replay exactly; eval report per brief. needs: AB1,
  AB2, AB3, WD6. (Wave 34: 12 tests + endpoint 236 green, judge PASS
  depth 3; door/game wiring rides AB8/AB9.)
- [x] AB6 mcp: variant dedup experiment — canonicalize generated encounters,
  levels and prop sets with a small rewrite-rule engine (e-graph idea from
  egg, arXiv 2004.03082, MIT; at most 30 rules, no dependency) so
  near-duplicates collapse before review. Experiment. Size S. Owns
  `crates/endpoint/src/canon.rs`. Accept: at least 30% fewer variants on 30
  seeded sets with 0 false merges on VK1 keypoints, within 60 s, else killed.
  needs: AB1, VK1. (Wave 39: ADOPT — 87.5% reduction, 0 false merges,
  canon 8 green, judge PASS depth 3.)
- [x] AB7 mcp: shared brief envelope — one typed envelope (brief id, seed,
  stage, repair budget, refusal shape) that AB1–AB4 and AB9 consume instead
  of per-stage budgets; over-budget stages refuse with legal values.
  Tier B. Size S. Owns `crates/endpoint/src/brief.rs`. Accept: 20 briefs
  across AB1 and AB3 share one envelope type; a 4th repair is refused with
  the findings named. needs: EA1b, SP5.
- [x] AB9 mcp: offline brief replay — every AB stage records its
  inputs/outputs so the full chain replays with providers off; a missing
  record fails loudly instead of calling a model. Tier B. Size S. Owns
  replay glue in `crates/endpoint/src/build_game.rs`. Accept: 10 recorded
  AB1 briefs replay byte-identically twice offline; live and offline
  digests match. needs: AB1, AB7, LW1. (Wave 32 note: same missing
  `build_game.rs` as AB8 — rides the AB4 landing, queue behind AB8.)
  (Wave 36: TICKED — replay glue, single AB7-envelope format shared
  with AB12's note, 23 build_game tests green, judge PASS depth 3;
  verification artifact, consumer is the offline replay proofs.)
- [x] SP6 spatial: 3D index — integer-cm 3D grid or BVH replacing the XZ-only
  index: levels, stairs, stacking, overhangs, walls, with incremental updates.
  Tier A. Size M. Owns `crates/spatial/src/index3.rs`. Accept: every SP1-SP5
  test passes on it; a two-floor ludus and a siege wall answer vertical
  queries; 10k objects query within 1 ms p95; bytes x2. Hunt 2026-09-25: compare an integer-cm R*-tree (Guttman, Beckmann; rstar MIT/Apache as a pattern) with the grid, plus dirty-subtree incremental updates (salsa red-green idea); at least 3x faster pair phase at 5,000 objects with no regression at 50; `place.rs` solve under 50 ms p95 on 500 objects. needs: SP3.
- [x] SP7 spatial: relation vocabulary v2 — above/below, in front of, behind,
  left/right in three frames (world, the object's own facing, a viewer),
  between, against, around, aligned, near/far bands and blocking, each with
  integer thresholds. Tier A. Size M. Owns `crates/spatial/src/relation.rs`.
  Accept: a 200-case gold set of relation questions at least 98% correct;
  digest stable x2. Hunt 2026-09-25: the gold set includes contrastive pairs that must flip the digest (stacked vs beside, near vs far, a watcher turned away, inside a zone vs inside the arena; 2502.03450). needs: SP6.
- [x] SP8 spatial: sight and reach — line of sight and visibility from any
  point or camera on proxy geometry, cover height against a crouched or
  standing capsule, and reachability through nav. Tier A. Size M. Owns
  `crates/spatial/src/sight.rs`. Accept: 100 gold sight and reach questions
  at least 95% correct; at most 0.2 ms per ray query; no floats reach the
  hash. needs: SP6, NV1.
- [x] SP9 spatial: affordances — sittable, climbable, cover, breakable (DX),
  walkable, interactable, spawn and choke points: derived from geometry and
  authored in schemas, shown in `scene.brief`. Tier B. Size M. Owns
  `crates/spatial/src/afford.rs`. Accept: 50 gold objects tagged at least
  90% right; a wrong authored tag is flagged against geometry. needs: SP7, RC4.
   Wave 54: PARTIAL — afford.rs applied (50/50 gold in-lane, suite green);
   mod line + scene.brief hook → SP9-WIRE queued (unwired file doesn't tick).
  Wave 54: TICK (declare landed, hook emits `afford <id> <csv>` rows in
  scene_brief within byte budget, 50-gold + 8-mistag green, endpoint
  300+ all green, spatial 147/0, gates + quality PASS).
- [x] SP10 mcp: spatial questions — `scene.query` (near, between, visible,
  reachable, cover near, largest open area, free space) returning IDs with
  reasons, plus a free-space sampler (Poisson disc with capsule clearance,
  grounded, no interpenetration) that SP5 and AB1 use. Tier A. Size M. Owns
  `crates/endpoint/src/scene_query.rs`, `crates/spatial/src/sample.rs`.
  Accept: 100 gold queries at least 95%; sampled placements 0 overlaps, 0
  floating, 100% reachable; answers fit the brief budget. Hunt 2026-09-25: every conflict returns a solver-computed repair hint (SpatialGrammar 2604.27555), and a dry-run graph diff equals the applied result 10 of 10. needs: SP8, MCP7.
- [x] SP14 spatial: production reroute onto index3 — `checks.rs` overlap
  phase, `place.rs` verify and `relation.rs` candidate phase read the
  landed SP6 index (one build per solve, microsecond probes); the dense
  500-object room solves inside the SP6 budget. Tier A. Size S. Owns the
  reroute hunks in `crates/spatial/src/checks.rs`, `place.rs`,
  `relation.rs` (same crate, index3 API frozen). Accept: SP1-SP5 suites
  green; dense-room solve under the SP6 50 ms p95 bar; bytes x2.
  needs: SP6.
- [x] SP15 spatial: incremental index3 — dirty-subtree rebuild for
  the SP6 index (salsa red-green shape, reimplemented std-only): an
  edit rebuilds exactly its subtrees before the next query and
  records it; full-build equality holds 20 of 20. Tier A. Size M.
  Owns `crates/spatial/src/index3.rs` incremental hunks (no new
  index type). Accept: 10k-object scene, 100 sequential edits, every
  probe equals the full-build probe; rebuild p95 under 2 ms; bytes
  x2. needs: SP6, SP14. (PLN25 spatial.md, reviewed 2026-09-25.)
- [x] SP17 mcp: conflict repair hints at the door — ScenePlaceOutput
  carries hints plus dry-run summary mapped from ConflictSet.hints;
  frozen solve signature, search order and SP3 graph untouched.
  Tier A. Size S. Owns the hints field plus conflict_output mapping
  in crates/endpoint/src/scene_tools.rs plus endpoint round-trip
  tests only. Accept: 20 seeded conflicts each hint verifies clean
  through scene.place; 10 of 10 dry-run diffs equal applied diffs
  through the door; golden d57b5c8ae3096614 x2 plus brief digest x2
  unchanged. needs: SP54.
- [x] SP18 spatial: brief-name hardening for duplicate names — the
  SP4 brief path renders every object as `name [id]` (MCP8 pattern at
  the tool layer; this is the spatial render path) so duplicate names
  never mis-place; solver and diff behavior unchanged. Tier B.
  Size S. Owns `crates/spatial/src/brief.rs` render hunks. Accept: a
  duplicate-name scene places correctly through SP5; gold briefs
  re-emit with ids and stay within budget. needs: SP4, MCP3.
  (PLN25 spatial.md, reviewed 2026-09-25.)
- [x] SL4 ops: one schedule — delete `crates/ecs/src/systems.rs` (5-phase
  duplicate of `schedule.rs`, zero users outside its own file per git grep)
  and its tests; `schedule::Phase` stays (engine-app uses it). Size S. Owns
  `crates/ecs/src/systems.rs`, `crates/ecs/src/lib.rs`. Accept: `cargo test
  -p engine-ecs -p engine-app` green; proof golden unchanged. needs: none. Tick audit 2026-10-19: the named ecs `systems.rs` was deleted by this row on purpose (da6b7922); the named path is the row's target, not a loss.
- [x] SL6 arena: zero-alloc tick — `Arena::step` builds a fresh
  `Vec::with_capacity(self.fighters)` for inputs every tick
  (`crates/arena/src/lib.rs` near line 550); use a fixed array or a reused
  buffer, then sweep the rest of the tick path. Tier A. Size S. Owns
  `crates/arena/src/lib.rs`. Accept: a counting allocator reads 0 heap
  allocations per tick after warmup; golden unchanged. Bench 2026-09-25: the full arena run makes 8,839 allocations (about 14.7 per tick) from arena scaffolding while physics and AI are already at zero; add an 8-fighter scaled workload beside the 4-fighter one. needs: none.
- [x] SHOW1 playtest: showcase clips — replay any tape in the window build and
  dump a PNG sequence plus a manifest (seed, tape, board rows shown, build
  SHA) into `target/showcase/<date>/<clip>/`; `tools/showcase.mjs` stitches
  MP4 and GIF with an external ffmpeg (never a crate dependency: GM3 must
  pass on a clean machine). This is the footage the factory's videographer
  cuts (EMPIRE inbox EB-4). Tier A. Size S. Owns
  `crates/playtest/src/showcase.rs`, `tools/showcase.mjs`. Accept: the same
  tape gives byte-identical PNGs twice; a 10 s clip stitches; the manifest
  names real rows. needs: WIN1, CR1. Tick audit 2026-10-19: `tools/showcase.mjs` is present; the named playtest `showcase.rs` was never landed as a file, so the path is stale.
- [x] ADV55a perf: committed-key reseal forgery — widened baseline row +
  `baseline_seal` recomputed with the in-tree `SEAL_KEY` committed as a
  pair passes `verify_seal` (ADV55 wave 54; only the unkeyed recipe is
  refused today). Tier A. Size S. Owns `crates/perf/src/baselines.rs`
  seal hunks (operator-held HMAC or detached signature; key out of
  tree). Accept: keyed-with-committed-key pair FAILs loudly with
  workload named; clean regens PASS; golden unchanged. needs: PF17,
  EV14. (ADV55.)
  Wave 54: TICK (operator-held key via env, in-tree key retired,
  detached receipts, committed-key forgery refuses loudly, perf 66/0,
  gate + quality PASS; HMAC-SHA256 upgrade proposed as follow-up).
  Wave 54 R2 (judge FAIL, reverted c83a9c6): `decode_seal_hex` /
  `parse_seal_key` / `seal_key_from_env` copied verbatim from proof
  `a9.rs:410-457` (proof dup_fns 3→6). R2 must move all three behind one
  owner (reuse the proof parser from perf, or extract to engine-core)
  and delete the `baselines.rs:174-230` copy; keep the forgery-refusal
  tests green.
  Wave 54 R2 LAND: helpers extracted to `engine-core::seal_key`
  (std-only), perf delegates with typed wrappers, perf copy deleted;
  dup_fns back to 3, perf 66/0, core 61/0, gates + quality PASS.
  TICK. Follow-up ADV55c (HMAC upgrade + migrate proof/playtest
  parsers onto the core owner).
  REVIEW-W54R2: PASS (depth 4/5, RUST 4/5 — revert clean, R2 one-owner,
  ED12/ADV55b/RP7/API1/SP42-DOC verified, no weakened tests, golden
  untouched). Wave-54 review CLOSED.

## SPRINT-10-empire-inbox-drain.md

- [x] DEAD2 tools: dead-js confirm-and-close (EB-AP-DG-dead-js-2026-10-02)
  - re-scan the 57 tracked JS/MJS, unexport or delete only confirmed-dead,
  fold copied bodies into one shared helper. Tier A. Size S. Owns
  `tools/board-slice.mjs`, `tools/gh-search.mjs`, `tools/showcase.mjs`,
  `tools/web-search.mjs`, `content/fighters/tools/paths.js`,
  `content/arena/tools/gen.mjs`, `content/maps/tools/gen.mjs`. Accept:
  re-scan lists 5 or fewer dead exports and 0 copied bodies;
  `node tools/workspace-check.mjs` PASS. F2P: re-scan before (dead + dupes
  listed); after (5 or fewer dead, 0 dupes). P2P:
  `node tools/workspace-check.mjs` stays green. needs: none.
  (EB-AP-DG-dead-js-2026-10-02.)
- [x] BLOAT2 content: repo-bloat LFS pass (EB-AP-DG-repo-bloat-2026-10-02)
  - move `banner.gltf` plus `crate.gltf` to LFS or untrack since gen.mjs
  rebuilds them, HDR skies stay in LFS. Tier A. Size S. Owns
  `content/arena/src/banner.gltf`, `content/arena/src/crate.gltf`,
  `.gitattributes`. Accept: `node tools/workspace-check.mjs` PASS including
  the lfs guard, 0 tracked non-LFS blobs over 5 MB, size-pack under
  500 MB. F2P: `git lfs ls-files` before (2 tracked non-LFS blobs over
  5 MB); after (0). P2P: `node tools/workspace-check.mjs` stays green.
  needs: none. (EB-AP-DG-repo-bloat-2026-10-02.)
- [x] E52 assets: schema_compile wire or delete (EB-2026-10-09-S1). Size S. Owns `crates/assets/src/schema_compile.rs` + lib.rs mod row only. F2P `node tools/workspace-check.mjs` prints no schema_compile line; P2P assets lib green. needs: none. (done 2026-10-09: file deleted at HEAD, workspace modules PASS 580, assets lib 128+201 green.) Tick audit 2026-10-19: the row's own purpose was to delete the named assets `schema_compile.rs`; the path is the target, not a loss.

## SPRINT-11-north-star.md

- [x] NS32 rules: deterministic rules sandbox on Rhai — new `crates/rules`
  embeds `rhai` =1.26.1 (MIT OR Apache-2.0) with `only_i64`, `no_float`,
  `no_time`, `no_module`, op/call/size limits, a deny-all registry with a small
  host API (read fighter state, emit actions), the script hash in the tape
  header, `rules.set/test/explain` as Rust functions (door verbs arrive through
  Hands' rules pack) and last good kept on any failure. Size L. Owns
  `crates/rules/**` (off `default-members`, no tick crate depends on it).
  Accept: F2P a 40-line shield bash script applied via `rules.set` in at most
  1 s; 3 of 3 bad edits refused (float op, unbounded loop, unknown host call)
  with typed errors and last good kept; 64 scripted fighters at most 0.2 ms per
  tick p99; 10,000-tick replay equal x2; an old tape with its old script hash
  still replays; P2P golden unchanged and `cargo tree -p engine-game -e normal`
  shows no rhai. needs: none.
  Done 2026-10-04 (commit 9ad1e1e, merged by the tick commit): `crates/rules` (engine-rules, off
  `default-members`; `cargo tree -p engine-game -e normal` shows no rhai). A script is plain
  statements run once per living actor per tick with the actor's numbers as read-only variables
  (`hp`, `blocking`, ...); 20 host functions, no float (`only_i64` + `no_float` + literal scan),
  no clock, no strings, per-actor 2,000-op budget, call/expr/size limits, AST vet (unknown host
  call, unbounded loop, method call, property access). `Rules::set/test/explain` with last good
  kept, per-actor fault isolation with memory rollback; `tape::RulesTape` header carries the ABI
  id and every script hash, `ScriptLibrary` replays old tapes with their old script. Measured:
  `cargo test -p engine-rules` 25 passed (+ budget 2); `rules.set` of the 35-line
  `scripts/shield_bash.rhai` 2.5-4.7 ms (bar 1 s); 3 of 3 bad edits refused with typed errors
  (float `Float{line}`, `loop{}` `UnboundedLoop` plus op-budget `Limit`, `UnknownHostCall`) and
  last good kept, plus 30 more refusals; 64 scripted fighters p99 0.108 ms per tick (release,
  PC loaded by the loops; mean 0.073 ms, bar 0.2 ms; strict gate `cargo test --release -p
  engine-rules --test budget -- --ignored`); 10,000-tick replay equal x2 and re-recorded bytes
  equal; committed tape `crates/rules/tests/data/shield_bash_v1.e2rl` replays after the live
  script moved on; golden `d57b5c8ae3096614` x2 (`proof.mjs --skip-build`); deny licenses ok
  (one exception: rhai's smartstring is MPL-2.0).
- [x] NS32a rules: arena controller hook and rules adapter — `crates/arena/src/controller.rs`
  (trait `SlotController`, `ControlView`, `SlotView`, `drive`), `run_controlled` in
  `crates/arena/src/lib.rs`, `run_spine_controlled` in `crates/arena/src/spine_step.rs`,
  `crates/rules/src/arena.rs` (`RulesController`, feature `arena`, off by default), input host
  calls `walk(mx,mz)` `face(cdeg)` `press(bits)` (additive, ABI id unchanged), script
  `crates/rules/scripts/shield_bash_arena.rhai`. Size M. Owns those files. Accept: F2P
  `cargo test -p engine-rules --features arena --test arena_drive` 6 of 6 (scripted slot differs
  from `run`, same seed twice equal 600 of 600, empty slot list equals `run`, spine equals legacy
  on an empty world, a failing call is counted and the match goes on, explain at tick 30 lists
  host calls); `cargo test -p engine-arena --lib controller` green; P2P golden
  `d57b5c8ae3096614` x2 and `cargo tree -p engine-game -e normal` shows no rhai. needs: NS32.
  Done 2026-10-05: `cargo test -p engine-rules --features arena` 25 lib + 6 of 6 `arena_drive` (scripted slot 0
  differs from `run`, same seed twice 600 of 600 equal, empty slot list equals `run`, memory reset
  between matches, a failing call is counted and the match goes on, explain at tick 30 lists host
  calls, the spine step equals the legacy step on an empty world); `cargo test -p engine-arena
  --features spine --lib controller` 5 of 5; golden `d57b5c8ae3096614` x2 (`proof.mjs
  --skip-build`); `cargo tree -p engine-game -e normal` and `-p engine-endpoint -e normal` show no
  rhai. Notes: the legacy motion loop reads the `slots` array, not the merged row, so the hook
  writes the decided inputs back into `slots`; the arena moves by facing, and wishes only change
  speed (the brain already runs at 100), so the script steers with `face`. Rhai reserves `go`,
  so the input calls are `walk`, `face`, `press`. Stamina in the arena is 0 to 100 points.
- [x] NS32b rules: door verbs `rules.set`, `rules.test`, `rules.explain` — feature `rules` on
  `engine-endpoint` (off by default), `crates/endpoint/src/packs/rules.rs`, `packs/rules/verbs.rs`,
  `packs/rules/worker.rs` (one thread per session owns the `!Send` `Rules`), hunks in
  `crates/endpoint/src/mcp.rs` (`ServerState.rules`, `play_run`) and `crates/endpoint/src/play.rs`
  (`MatchRunner`, `play_run_with`). Size M. Accept: F2P `cargo test -p engine-endpoint --features
  rules --test it rules_door::` 2 of 2 (set makes `play.run` differ from plain, same seed twice 600 of
  600 equal, 3 bad scripts refused `rules_refused` with the last good kept, slot 16 refused
  `bad_slots`, test and explain change nothing); default `cargo test -p engine-endpoint` unchanged;
  `cargo tree -p engine-endpoint -e normal` shows no rhai; golden unchanged. needs: NS32a.
  Done 2026-10-05: `cargo test -p engine-endpoint --features rules --test it rules_door::` 2 of 2 (600 of 600
  checkpoints equal on a replay; 3 of 3 bad scripts refused `rules_refused` naming the kept hash;
  slot 16 refused; test and explain change nothing); the `rules` feature is off by default. Note: the
  job log ring keeps 16 lines, so the "rules script ... drives slots" line shows only on short matches. Owner chat 2026-10-19 (delegated): path updated to the tests/it layout; re-ran `node tools/gh-check.mjs --budget 840 cargo test -p engine-endpoint --features rules --test it rules_door::` 3 passed, run 39230104016.
- [x] NS32f rules: effect actions in the real arena — apply `damage`, `heal`, `stun`,
  `drain_stamina` of a scripted slot through a second trait method in
  `crates/arena/src/controller.rs` that edits `FightState.fighters[i]` (public fields `hp_permille`,
  `react`, `stamina`) before combat resolves; `push` and `shield` stay ignored. Size S. Accept:
  F2P `cargo test -p engine-rules --features arena --test arena_drive` new test
  `effects_apply_in_the_real_arena` (the original `scripts/shield_bash.rhai` now changes the
  match); no controller means the exact legacy path, golden `d57b5c8ae3096614` x2. needs: NS32a. Landed 2026-10-07 ns32f: F2P effects_apply 1/1 + drive 7/7, controller 9/9, arena 110, rules 32, golden x2, arena quality 86 B 0 regress, packet-gate 7 paths 0 findings, review PASS ns32f. Note: wall-clock guard flakes under parallel load only (quiet 0.2 ms); rules casts 11→13 pre-exists on master.
- [x] GN1 assets: generated-mesh intake — new `crates/assets/src/gen_intake.rs` (features `import` +
  `registry`): licence gate (SPDX allow-list from `deny.toml`, restricted-model list, declared
  conditions must be accepted by name), glTF import, repair (weld, degenerate and duplicate
  triangles, floating specks), pose (up axis, size, base on the floor), simplify to at most 5,000
  triangles, LOD chain plus collision hull, E204BNDL bundle, one provenance row (model id, SPDX
  licence, prompt or image digest, seed), and a cache keyed by those inputs. Size M. Owns
  `crates/assets/src/gen_intake.rs` and its `gen_intake/` tests. Accept: F2P a 20,480-triangle
  generated mesh cooks to at most 5,000 triangles with a hull and a row; cook x2 byte-identical;
  a cache hit returns the same bundle hash (also from a new process); a non-commercial model,
  licence or part is refused before any cook; P2P golden `d57b5c8ae3096614` unchanged. needs: none.
  Done 2026-10-05 (lane fable-make): `crates/assets/src/gen_intake.rs` (+ `gen_intake/tests.rs`), feature `import` + `registry`.
  `cargo test -p engine-assets --features import,registry gen_intake` 9 passed: a 20,480-triangle sphere mesh with a duplicate, a
  degenerate triangle and a floating speck repairs to 20,480 in, cooks to at most 5,000 (LODs shrink), hull holds it, size 1 m, base on y=0;
  cook x2 byte-identical; cache hit returns the same bundle hash (also from a new folder cache); changed GLB re-cooks; refused: CC-BY-NC,
  GPL, non-SPDX, briaai RMBG, diso, Hunyuan3D, undeclared condition, all before any cook. Licence list = `deny.toml` allow-list. No sim crate touched
  (golden `d57b5c8ae3096614` path unchanged; `cargo tree -p engine-game -e normal` shows no assets).
- [x] GN2 endpoint: `asset.intake` door verb (feature `intake`, off by default) reads the center
  runner's `kaggle-lane.json`, writes the bundle and the row, and `asset.generate` explains the
  center command (no runner in the engine). Size S. Owns `crates/endpoint/src/packs/asset.rs` and
  `packs/asset/`. Accept: F2P through the door a generated GLB cooks to at most 5,000 triangles,
  a refused model writes nothing, a repeat call returns the same bundle hash with `cache_hit`;
  `cargo test -p engine-endpoint --features intake asset` green; the default build links no
  `engine-assets`. needs: GN1.
  Done 2026-10-05: `crates/endpoint/src/packs/asset.rs` + `packs/asset/intake_verb.rs`; feature `intake` (off by default) pulls
  `engine-assets` (edge documented in `tools/workspace-check.mjs` and `docs/ARCHITECTURE.md`). `asset.generate` explains the center command; `asset.intake`
  reads `kaggle-lane.json`, writes `<out>/<name>.e204bndl` + `<name>.provenance.json`. `cargo test -p engine-endpoint --features intake asset` 7 passed
  (+1 ignored real-props test): refusal writes no bundle, repeat call returns the same bundle hash with `cache_hit`, fresh folder re-cook is byte-identical.
- [x] GN3 make: the free chain on real pictures — center `kaggle.mjs run mesh` (BiRefNet MIT cutout,
  TripoSG MIT shape; RMBG-1.4 and `diso` never loaded) on 2 pictures made by `run image`, then
  `asset.intake` on both. Size S. Owns center `kaggle.mjs` and `kaggle.test.mjs` hunks. Accept:
  F2P 2 generated props cook to at most 5,000 triangles each with a hull and a provenance row; cook
  x2 byte-identical; 0 weight bytes in git; golden unchanged. needs: GN1, GN2.
  Done 2026-10-05: center `kaggle.mjs run mesh` (commits 38be5b4, c2ca457; `node --test kaggle.test.mjs` 30 passed): 2 pictures from `run image` (FLUX.1-schnell, 7.1 GPU min),
  then BiRefNet + TripoSG in one kernel, 7.7 GPU min, 2 GLBs (`target/make-run/mesh`, not in git). Through `asset.intake`: barrel 59,948 -> 4,822 triangles, lantern 59,859 -> 4,852,
  hulls (14 and 12 support points, radius 0.658 and 0.518 m), provenance rows (VAST-AI/TripoSG MIT, seed 4243/4244, image sha256), cook x2 byte-identical, 0 weight bytes in git.
  OPEN for Maxim: TripoSG's transformer code is adapted from Tencent HunyuanDiT (licence excludes the EU, no use of Output outside the Territory); intake needs that condition accepted by name.

## SPRINT-12-tier-2040.md

- [x] RT0 render: tier probe — read the adapter's ray-query and
  acceleration-structure features (wgpu experimental ray tracing), pick
  Base, RT or Ultra, refuse honestly, write the tier into every RA8
  manifest. Tier C. Size S. Owns `crates/render/src/tier.rs`. Accept: the
  reference GPU reports RT; llvmpipe reports Base; a forced tier the
  adapter lacks is refused. needs: GX1.
- [x] RT1 render: acceleration structures — BLAS per cooked mesh from
  simplified CT2 proxies (full detail stays in raster), TLAS rebuilt or
  refit per frame, skinned refit for fighters. Tier C. Size L. Owns
  `crates/render/src/accel.rs`. Accept: 8 skinned fighters plus the arena
  refit within budget on the reference GPU (accel refit 1.5 ms with 8
  skinned fighters); build inputs identical x2. Consumer is the WIN1
  match; CPU parity is the merge gate, device submit is the look gate.
  needs: RT0, CT2.
- [x] ON1 net: QUIC transport — quinn (license read first) behind a
  `Transport` trait: unreliable datagrams for inputs and snapshots,
  reliable streams for lobby and chat, encryption; the LAN UDP path
  stays. Tier C. Size M. Owns `crates/net/src/quic.rs`. Accept: the MP1
  loopback tapes converge over QUIC; kbps per client charted. Deep dive 02 (2026-09-25): QUIC carries session, lobby and reliable streams; tick inputs stay on whichever path measures the best p99, and QUIC must not add p99. needs: MP1.
- [x] ON2 net: internet join — self-hostable relay binary, STUN and ICE
  hole punching with relay fallback, invite codes. Tier C. Size L. Owns
  `crates/net/src/relay.rs`. Accept: two machines on different networks
  join by invite code; relay fallback when punching fails. needs: ON1.
- [x] ON3 net: interest management — relevancy by distance and view,
  per-connection priority, delta budgets (Iris-style shape from Epic's
  public docs) for 64 players plus 500 AI soldiers. Tier C. Size L. Owns
  `crates/net/src/interest.rs`. Accept: a 64-client bot soak holds the
  kbps cap per client; far fighters update at a lower rate without
  desync. Deep dive 02: grid interest plus a per-connection delta baseline and a priority queue; ≤ 256 kbps per client at p95, no entity starved over 500 ms, ≥ 3x bandwidth saving or killed. needs: ON1, MP2.
- [x] ON4 net: WAN fault soak — injected 150 ms RTT, 30 ms jitter, 2%
  loss, reorder, disconnect and reconnect across 64 clients. Tier C.
  Size M. Owns `crates/net/src/wan_soak.rs`. Accept: the soak converges
  on every authoritative checkpoint; a report per fault. Deep dive 02: bounded rollback window ≤ 8 ticks, resim ≤ 2 ms p99, converge ≤ 5 s after a fault, tick IO wait 0 ms. needs: ON2, ON3.
- [x] ON5 net: server-authority cheat checks — input rate and range
  validation, movement sanity, hits only through server lag compensation,
  replay-backed reports; no kernel drivers. Tier C. Size M. Owns
  `crates/net/src/guard.rs`. Accept: seeded speed, rate and hit-claim
  cheats are refused and logged with a replay tape. needs: ON1, MP5.
- [x] ON9 net: siege interest proof — 64 fighters plus 500 AI soldiers through
  ON3 relevancy with per-connection budgets and far-rate LOD. Tier C. Size L.
  Owns `crates/net/src/interest.rs`. Accept: 64-client bot soak holds ≤ 128
  kbps per client at 10–15 Hz; far fighters update at reduced rate with zero
  desync; kbps and rate-LOD charts posted. needs: ON3, MP2.
- [x] WD1 world: cell streaming — async cell load and unload with AS2
  budgets, nav and physics per cell, seamless edges. Tier C. Size L. Owns
  `crates/world/src/cells.rs`. Accept: a walk across 9 cells has zero
  tick hitches; memory stays flat over a 30-minute loop; publish happens
  at frame boundaries through the AS2 queue; the 9-cell walk fixture names
  per-cell load time and peak memory; the 30-minute loop asserts memory
  within 5% of the AS2 budget. needs: AS2, NV1.
- [x] WD6 game: mode framework — arena, siege and district free-roam over
  the GP4 ladder. Tier C. Size M. Owns `crates/game/src/modes_world.rs`.
  Accept: each mode boots from the lobby and replays; no mode edits the
  engine. needs: GP4, WD1.

## SPRINT-13-engine-edge.md

- [x] OPT1 ops: build baselines, observed-only — one clean
  `cargo build --locked -j 2 --timings` per profile (dev, `--profile
  fast-test`, `--release`) plus one touch rebuild of each of
  `engine-core`, `engine-combat`, `engine-render`, `engine-game` per
  profile (touch `crates/<dir>/src/lib.rs`, rebuild with `--timings`);
  per-crate seconds, the critical path from the longest `.timing.json`
  chain and link time under `target/evidence/build/` with the table in
  `docs/SMOOTH-BUILD.md`. (orchestrator) Tier A. Size S. Owns the
  report. Accept: report plus table exist with the critical path named;
  no repro gate in-wave (promotion to baseline is OPT32). needs: none.
  (OPT1 VERDICT wave 84: split orchestrator-observed-only + OPT32 HELD.)
- [x] OPT2 render: backend selector plus headless 600-frame bench — new
  `crates/render/src/backend.rs` (`WGPU_BACKEND` dx12|vulkan|gl|auto,
  compiled-in default, startup log names the chosen backend plus why);
  route the two runtime entries (`gpu.rs` window path, `present.rs`
  `init_offscreen`/`init_windowed`) through it, leaving test-only
  `Backends::all()` sites alone; the headless harness in `backend.rs`
  tests drives the offscreen presenter over the fixed 600-frame arena
  camera orbit with RA8 executor receipts. Tier B. Size S. Owns
  `crates/render/src/backend.rs` plus selector hunks in
  `crates/render/src/gpu.rs` and `crates/render/src/present.rs`. Accept:
  `cargo test --locked -p engine-render --lib backend` green headless
  (adapter legs skip-not-green with no GPU); report under
  `target/evidence/backend/` names adapter, driver and backend with CPU
  per-pass p50/p95/p99, draws, validation errors and VRAM-or-PENDING;
  draws plus validation errors identical x2; GPU per-pass p50/p95/p99
  marked PENDING until Q1, never faked; `WGPU_BACKEND` override honored.
  needs: RA8, WIN1. (OPT2 VERDICT wave 84: split headless S + OPT33 HELD.)
- [x] OPT3 ops: dev profiles that keep iteration fast —
  `[profile.dev] opt-level = 1`, keep the 12 landed per-crate opt-level 2
  lines, keep `[profile.dev.package."*"] opt-level = 3` with the packet's own
  before/after numbers (touch-`engine-combat` within +10% on a quiet box, else
  close with the numbers per Stop clause). Root `Cargo.toml` owns `dev`
  opt-level (`.cargo/config.toml` keeps codegen-units/debug/incremental/linker
  only). (orchestrator) Tier A. Size S. Owns root `Cargo.toml` `[profile.dev]`
  hunks. Accept: workspace-check PASS; `cargo check --locked -p
  engine-game` green; golden test x2 unchanged; touch-`engine-combat`
  rebuild seconds before/after within 10% quiet-box, or filed numbers on a
  loaded box. Proof: `cargo check --locked -p engine-game` green + golden x2.
  needs: none. (OPT3 deep-dive 2026-10-03: root cause — board says 11 lines
  while `Cargo.toml:254-277` lands 12 plus the `"*"` blanket, and
  `.cargo/config.toml:8` pins a second `dev` opt-level 0, so 4 helpers in
  72 h re-decided the `"*"` conditional against incomparable baselines while
  the +10% touch gate cannot hold under shared-pool contention; different
  approach — single-truth config-only close, Stop clause honored, window fps
  stays OPT31; proof: check green + golden x2 + touch numbers.)

## SPRINT-15-platform-live-ai.md

- [x] PLF1 app: platform contract — one trait for achievements, stats,
  cloud save, rich presence, overlay hooks, lobbies and Workshop, with a
  `NullPlatform` default; callbacks pumped once per frame outside the
  tick. Tier B. Size M. Owns `crates/app/src/platform.rs`. Accept: the
  arena runs identically (golden x2) with the null backend; a mock
  backend records every call in order. needs: WIN1.
- [x] PLF2 game: Steamworks backend — steamworks-rs behind a `steam`
  feature; the SDK found from an env path and never committed;
  achievements, stats, cloud saves of SV1 generations, rich presence.
  Tier B. Size M. Owns `crates/game/src/steam.rs`. Accept: with Steam
  running, an achievement unlocks on the public test app; without Steam
  or without the feature the null backend runs; no Steam type outside
  the file. needs: PLF1, SV1.

## SPRINT-16-urban-war.md

- [x] UW1 physics: segment queries — BVH over caller-passed CT2 hulls,
  moving capsules and live chunk boxes; integer-mm segments in, hits
  sorted by (distance, id) with material ids and entry/exit points out.
  Tier B. Size M. Owns `crates/physics/src/query.rs`. Accept: 10k seeded
  segments match brute force, hash x2; 2,048 segments on a 20-building
  fixture ≤ 1 ms (reference CPU); 0 allocations after warm-up. needs:
  CT2.

## SPRINT-17-body-and-break.md

- [x] BX1 combat: anatomy graph and tissue tables — humanoid joint graph, 15
  severable joints, three layers per segment, per-armor coverage, Q16
  toughness, versioned data plus a validator. Tier A. Size M. Owns
  `crates/combat/src/anatomy.rs`. Accept: validator rejects cyclic or
  unarmored-gap tables; table digest x2; the Rome gladiator table covers galea,
  manica, greave, lorica and scutum. needs: CB1.
- [x] BX2 combat: cut resolver — swept blade segment vs segment capsules in
  integers, edge sharpness and wear, one `Cut` (16 bytes) per joint per swing.
  Tier A. Size M. Owns `crates/combat/src/cut.rs`. Accept: 10,000 seeded swings
  byte-equal x2 and under shuffled insertion; cut chance rises monotonically with
  energy x sharpness; a manica stops a swing a bare forearm loses; at most
  0.15 ms per cut. needs: BX1, GP2. (PX20 is advisory physics-lane
  follow-up: cut.rs imports only anatomy + engine-core, GP2 sweep is
  the real bridge.)
- [x] BX3 combat: limb loss and bleeding — stump states (intact, wounded,
  disabled, severed), venous and arterial bleed, shock, exsanguination and
  surrender tables. Tier A. Size M. Owns `crates/combat/src/limbloss.rs`.
  Accept: bleed-out time table hit within 1 tick x2; wound table matrix
  replays x2; extends UW5 without forking it. needs: BX2. (Hit zones and armor dropped from the needs 2026-10-04: the wound table takes the damage they will resolve; they stay open in their own row.)
  Done 2026-10-04: `crates/combat/src/limbloss.rs`: versioned `WoundTable` (blood 5,000 ml in Q16, bleed-out times per class ooze 18,000 /
  venous 5,400 / arterial 1,200 ticks, stump class and sever shock per joint, wound and disable lines, shock decay and gain, surrender
  and faint lines) and `BodyState` (Copy, no heap): stump states intact, wounded, disabled, severed; `apply_cut(table, anatomy, Cut)`
  severs the joint and everything below it and only the cut root bleeds; `apply_wound` steps toward worse only; `step` bleeds, raises and
  decays shock and returns Surrender, Faint or Exsanguinated once each. `cargo test -p engine-combat --lib limbloss` 9 green: bleed-out
  time table hit within 1 tick x2 for venous, arterial (neck and shoulder) and ooze; wound matrix 144 cells replays x2 (digest
  `d9b7824741515931`); a neck cut surrenders, faints and dies in that order; a hand cut yields before it bleeds out; zero allocation over
  600 steps. UW5 (hit zones and armor) is still open: this module takes the damage UW5 will resolve and extends `anatomy.rs` and `cut.rs`
  without forking them. Golden unchanged.
- [x] BX4 physics: severed limb as a body — detach at joint, mass from volume
  x density, inherited velocity plus blade impulse, stump recoil, canonical
  order, capped and in every snapshot. Tier A. Size L. Owns
  `crates/physics/src/limb.rs`. Accept: 1,000 seeded cuts hash x2 and
  snapshot-restore equal; no NaN; body cap holds with debris. needs: BX2, PX1. (Fighter rollback bytes dropped from the needs 2026-10-04: the pool brings its own snapshot bytes.)
  Done 2026-10-04: `crates/physics/src/limb.rs`: `LimbPool` of severed limbs as integer bodies (mass = volume x density, velocity = parent
  bone + impulse / mass, quarter-bounce ground contact, friction, sleep, spin from a seeded stream), `spawn` returns the equal-and-opposite
  stump `Recoil`, `spawn_batch` runs in (swing, victim, joint) order, the pool caps limbs plus the debris the caller reports (oldest sleeper
  first, then oldest; when debris alone fills the cap the report says how many debris bodies to free), `snapshot` / `restore` bytes
  (hostile bytes refused). `cargo test -p engine-physics --lib limb` 9 green: 1,000 seeded cuts hash x2 (`c12d0d46bb5bb40e`, 216 alive of cap 256
  with 40 debris) and snapshot-restore then replay lands on the same hash; no float so no NaN; cap holds with debris on 1,000 cuts; zero allocation per step.
  PX17 (64-fighter snapshot) not needed for the pool. Golden unchanged.
- [x] BX10 combat: gore setting never changes the hash — Off, Reduced, Full as a
  presentation policy. Tier A. Size S. Owns hunks in `crates/combat/src/gore.rs`.
  Accept: one arena tape at three settings gives one golden hash; Off shows
  impact flash and a guard or downed pose, no severed part. needs: BX2, CB2.
- [x] BX12 destruction: material table — 14 materials with toughness,
  brittleness, shard rule, sound, dust, spark, decal, burn rate. Tier A. Size M.
  Owns `crates/destruction/src/material.rs`. Accept: every bond and CSG bake
  names a material id; validator; digest x2; feeds DX9. needs: DX2, DX3.
- [x] BX13 destruction: integer load-leg — BFS-layer loads over the binary
  flood in `crates/destruction/src/bonded.rs` (LADDER L458). Tier A.
  Size S. Owns load-leg hunks beside `crates/destruction/src/bonded.rs`
  (base `:549` flood untouched as oracle). Accept: 1/1 oracle + 8/8
  weakest-wall agreement with digest `194afd8d` x2 + p50 <= 50us;
  consumer sword-vs-wall/siege/BX24. needs: BX12.
  (ACCEPTED research-merge 2026-09-30, L458.)
  Done 2026-10-04: load-leg in `crates/destruction/src/bonded.rs` (BFS layers out of the anchors over intact bonds, smallest-index parent,
  reverse-visit Q16 sums child into parent, sorted adjacency built once, pre-sized scratch, no steady-state allocation; the old
  binary flood stays as the test oracle `support_flood_oracle`) plus `crates/destruction/src/structure.rs` (weakest link,
  predicted collapse, report digest, the 8-chunk oracle wall and the seeded frame building). `cargo test -p engine-destruction
  --lib` 135 passed: `load_oracle_wall_matches_the_hand_loads` 1/1 (loads 14/9/4/10/5/2/2/2, parents and layers equal the
  hand case), `weakest_link_is_a_column_on_eight_seeded_walls` 8/8, the support set equals the oracle flood after every hit of a
  24-piece crate; load digest `5d5b6204f7a409f7` equal x2 and across 8 seeds (the spike's `194afd8d` was a node-side FNV over
  other bytes: this crate pins its own digest). Step p50 0.1 to 0.2 us for the 8-chunk wall, p50 20.7 us and p99 37.5 us for the
  427-chunk building (release, loaded box, ignored test `step_budget_building_600_ticks`). Golden `d57b5c8ae3096614` x2
  unchanged. Consumer wiring (sword-vs-wall, siege, BX24 verb) still open.
- [x] BX35 endpoint: break pack read verbs — `structure.analyze`, `break.fracture` and `hit.simulate` in
  `crates/endpoint/src/packs/break_.rs` (Observe, read-only, loaded on demand by `api.load break`), over the destruction and combat
  crates. Tier B. Size S. Owns `crates/endpoint/src/packs/break_.rs` and `crates/combat/src/hitsim.rs`. Accept: each verb typed,
  deterministic (same request, same bytes) and refusing bad input with the legal range; the pack guard tests stay green.
  (The undoable authoring verbs `anatomy.set`, `material.set`, `wound.table.set` and the BX24 ledger parity stay open.)
  Done 2026-10-04: `structure.analyze` (bays, stories, knockouts, seed, capacity_percent: elements, weakest link and its kind, what falls with it, digests),
  `break.fracture` (wall size, hit, cells 2..=64, radius, seed: piece boxes, bond and freed event counts, digest), `hit.simulate` (energy, sharpness,
  wear, worn bits, samples up to 100,000, seed: per joint protection, sever odds, severed per thousand, bleed-out ticks; the odds and the seeded roll
  are the cut resolver's own). `cargo test -p engine-endpoint --features mcp --lib` 396 passed (5 new break pack tests), `cargo test -p engine-combat --lib
  hitsim` 5 passed. Adds the endpoint -> destruction and combat edges (std-only crates) to the workspace-check table. Golden unchanged.
- [x] BX37 arena: severed limbs from cut events — new `crates/arena/src/limbs.rs` (one `pub mod limbs;` line in `crates/arena/src/lib.rs`,
  behind the `sim` feature next to physics). Tier A. Size S. Owns `limbs.rs`. Build `pub fn spawn_limbs(pool: &mut engine_physics::limb::LimbPool,
  cuts: &[engine_combat::Cut], debris_live: usize) -> Result<Vec<engine_physics::limb::SpawnReport>, engine_physics::limb::LimbError>`: one
  `LimbSpawn` per cut (swing, victim, joint from the cut; volume from a const table `LIMB_VOLUME_CM3: [u32; 16]` in joint order, density 1000,
  radius 60 mm, origin `[0, 1000, 0]` mm, parent velocity zero, impulse `[20000, 5000, 0]` mN*s until the fighter capsules feed real values),
  through `LimbPool::spawn_batch`. Accept: `cargo test -p engine-arena --features sim --lib limbs` green: 20 seeded cuts (joints 1..=15, swings 0..20)
  give `pool.len() == 20` and the same `pool.state_hash()` when the slice is shuffled and when run twice; with `debris_live` equal to the pool cap the
  reports carry `debris_to_free == 1`; golden unchanged. needs: BX4.
- [x] BX38 arena: one rollback snapshot with the limbs — in `crates/arena/src/limbs.rs` (after BX37). Tier A. Size S. Owns that file. Add
  `pub fn capture_with_limbs(collections: &[&BondedCollection], ledger: &DebrisLedger, pool: &LimbPool) -> Vec<u8>` (the destruction `snapshot::capture`
  bytes, then the pool's `snapshot()` bytes, each length-prefixed with a u64) and `restore_with_limbs` (atomic: parse both, then replace). Accept:
  `cargo test -p engine-arena --features sim --lib limbs_snapshot` green: a 427-element building collapse (see `engine_destruction::snapshot` tests,
  `BuildingSpec::three_story(0)`) plus 20 limbs, checkpoint at tick 6, resim to tick 90, restore, resim again: equal bytes; truncated and trailing
  bytes refused with nothing changed; golden unchanged. needs: BX37.
- [x] BX41 proof: break corpus helper — move `seeded_cut_digest` out of the test module of `crates/destruction/src/structure.rs` into a `pub fn
  seeded_wall_cut_digest(index: u64) -> u64` (same body: seed `index * 2`, oracle wall, one seeded sever, 40 ticks of `collapse_digest`) and re-export it from
  `crates/destruction/src/lib.rs`. Tier A. Size S. Owns those two files. BX28 (`crates/proof/src/break_corpus.rs`) then folds it over 10,000 indices.
  Accept: `cargo test -p engine-destruction --lib ten_thousand_seeded_cuts_are_byte_equal_twice -- --nocapture` prints corpus digest `ced4ea909061defd`
  (unchanged) and the test calls the public fn; golden unchanged. needs: BX13.
- [x] BX28 proof: break replay corpus — 10,000 seeded cuts with one golden digest
  (500 collapses and 50 fires join the corpus when BX13 and BX16 land). Tier A.
  Size M. Owns `crates/proof/src/break_corpus.rs`. Accept: digest x2 on the
  reference laptop and on Linux CI. needs: BX2.
- [x] BX42 destruction: fourteen shipped material records roundtrip plus
  BX12 digest lock. Tier B. Size S. Owns test hunks beside
  `crates/material/src/record.rs` (RECORD_VERSION plus MAX_LAYERS plus
  MAX_NAME_BYTES frozen; no new file, no second table). Accept: 14 shipped
  records parse plus emit byte-identical x2; sim_digest equals the BX12
  table digest; golden unchanged. F2P:
  `cargo test -p engine-material --lib fourteen_shipped` (fails before as
  missing). P2P: `cargo test -p engine-material --lib` stays green.
  Consumer: BX12 table, BX24 material.set. needs: BX12. (Planner
  planner-rows 2026-10-07; plan docs/plan/body-and-break.md:415.)
- [x] BX43 destruction: set budget plus param edit lock over the shipped
  kit. Tier B. Size S. Owns test hunks beside
  `crates/material/src/set.rs` (MAX_RECORDS 255 plus
  siege_kit/shipped/reference frozen; no new file, no second set).
  Accept: 256th push refuses with named TooMany; set_param plus inverse
  roundtrips on 7 shipped records; bytes x2; golden unchanged. F2P:
  `cargo test -p engine-material --lib set_budget` (fails before). P2P:
  `cargo test -p engine-material --lib` stays green. Consumer: endpoint
  material pack 7 verbs. needs: BX12. (Planner planner-rows 2026-10-07;
  plan docs/plan/body-and-break.md:424.)
- [x] BX44 destruction: sim/look digest split plus compare lock. Tier B.
  Size S. Owns test hunks beside `crates/material/src/compare.rs`
  (lab_of plus ciede2000 plus compare frozen; no new file, no second
  digest). Accept: roughness edit moves only look_digest and toughness
  edit moves only sim_digest across all 14 records with 0 cross moves;
  bytes x2; golden unchanged. F2P:
  `cargo test -p engine-material --lib separate_digests` (fails before).
  P2P: `cargo test -p engine-material --lib` stays green. Consumer: BX28
  corpus, BX11 sound table. needs: BX12. (Planner planner-rows 2026-10-07;
  plan docs/plan/body-and-break.md:433.)
- [x] HS15 proof: tape-to-dataset export — action-conditioned shards (state,
  action, observation, hash, optional capture) from any replay, license clean,
  any provider can train on it. Tier B. Size M. Owns
  `crates/proof/src/dataset.rs`. Accept: 1 hour of arena tapes exports and
  re-verifies by hash x2; a provider-agnostic manifest. needs: OB1, DT5.
  (HS15.d landed 2026-10-10: JSON-only wire + license-refusal test, dataset
  17/17, golden d57b5c8ae3096614 x2, quality 93 A.)
- [x] GEO2 geo: native mesh booleans and the geo crate seed (decision D4, center note `engine-steal-blender-2026-10-04.md`
  row GEO2) — new cook-time crate `crates/geo` (package `engine-geo`, not a default member, no tick crate depends on it:
  `cargo tree -p engine-game -e normal` shows no geo; every donor behind a feature that is off by default): the shared
  `GeoMesh` (weld, edge census, volume, digest), seeded primitives, and `geo::boolean` (union, difference, intersection) on
  `manifold-rust` 0.16.0 (Apache-2.0, pure Rust). Tier A. Size M. Owns `crates/geo/**`, the `engine-geo` line in
  `tools/workspace-check.mjs`. Accept: `cargo test -p engine-geo --features boolean` — 20 seeded solid pairs (box, sphere,
  cylinder, torus, L prism; seeded scale, quarter turn, offset) give union and difference results with 0 open edges and equal
  digests x2; 6 box pairs match the analytic volume within 1e-4 relative; open or malformed input refused with a typed error;
  closed kit meshes of `content/arena/src` survive a boolean with a cutter; `cargo deny` license list unchanged; golden
  `d57b5c8ae3096614` unchanged. needs: none.
  Done 2026-10-04: `crates/geo` (`GeoMesh`, seeded primitives, `geo::boolean` on manifold-rust 0.16.0); `cargo test -p engine-geo --features boolean` green: 20 seeded solid pairs x union and difference = 40 results, 0 open edges, digests equal x2; 6 box pairs match the analytic volume within 1e-4; open, empty and malformed input refused with typed errors; a 24-vertex seamed box is welded first; on the arena kit 36 of 38 primitives are closed and survive union and difference with a cutter (0 open edges, digest x2); `cargo deny --manifest-path crates/geo/Cargo.toml --features boolean,uv,lod,collision check` ok (one named exception: clipper2-rust BSL-1.0, `deny.toml`; the `kit` test feature pulls the engine importer whose image/rav1e chain has an NCSA crate, libfuzzer-sys, not checked here); `cargo tree -p engine-game -e normal` shows no geo; clippy -D warnings clean; golden unchanged.
- [x] GEO3 geo: UV unwrap and pack (center note row GEO3) — `geo::uv::unwrap` on xatlas (MIT; source vendored unmodified in
  `crates/geo/vendor/` because `xatlas-rs-v2` needs a libclang install for bindgen, plus our own C shim; built by `build.rs`
  with `cc` only when feature `uv` is on, `XA_MULTITHREADED=0`) and `overlapping_chart_pairs`, an exact separating-axis check.
  Tier A. Size S. Owns `crates/geo/src/uv.rs`, `crates/geo/vendor/`, `crates/geo/build.rs`. Accept: `cargo test -p engine-geo
  --features uv` — the 8 arena kit glTF files (every primitive) give 0 overlapping charts, every UV inside 0..1, equal digest
  x2; atlas size, chart count and vertex growth printed; golden unchanged. needs: GEO2.
  Done 2026-10-04: `geo::uv::unwrap` + `overlapping_chart_pairs`; `cargo test -p engine-geo --features all --test kit` green: all 38 primitives of the 8 arena kit files give 0 overlapping charts, every UV inside 0..1, equal digest x2; vertex growth 1.00 on the box-like kit (6 charts), 2.25 to 2.57 on the 5 debris meshes (73 to 98 charts); atlas size is the packer's own (about 1000 to 1800 px at the 1024 target, printed per mesh); xatlas built with 1 thread; golden unchanged.
- [x] GEO5 geo: collision decomposition (center note row GEO5) — `geo::collision::decompose` on the V-HACD in `parry3d`
  0.30.2 (Apache-2.0, already in `Cargo.lock`, `enhanced-determinism` like rapier3d): closed convex parts, at most `max_hulls`
  (16) and `max_hull_vertices` (64) each. Tier A. Size S. Owns `crates/geo/src/collision.rs`. Accept: `cargo test -p engine-geo
  --features collision` — an L prism splits into 2..16 parts whose volume sum is closer to the L than its single hull; the 8
  arena kit files give at most 16 parts each, every part closed and outward, digest equal x2; golden unchanged. needs: GEO2.
  Done 2026-10-04: `geo::collision::decompose` + `single_hull`; `cargo test -p engine-geo --features all --test kit` green: the 38 kit primitives give 1 to 16 parts each (convex ones 1, debris 14 to 16, stands 16 at most), every part closed (0 open edges) and outward, at most 64 vertices, digest equal x2; the L prism splits into 2..16 parts whose volume sum (about 3) beats its single hull (3.5); golden unchanged.
- [x] GEO11 geo: native recipe runner and the door's modeling ops (decision D4; lead's order after the geo pack skeleton
  DV3) — `crates/geo/src/recipe.rs` (`Recipe { seed, ops }`, 7 typed ops over named meshes: primitive, place, jitter, boolean,
  unwrap, lod, collision; limits 64 ops, 200,000 triangles, names a-z 0-9 _ -; `run` returns every mesh and collision part with
  one digest; an op whose donor feature is off refuses `FeatureOff`) and, in `crates/endpoint/src/packs/geo.rs` plus
  `crates/endpoint/src/packs/geo/kernel.rs`, the `GeoOp` variants `boolean` (cutter cube, sphere or cylinder with an
  offset, union, difference or intersection), `unwrap`, `lod` and `collision`, behind engine-endpoint's off-by-default
  feature `geo` (without it each refuses with the typed code `feature_off`; a modeling op after a `select` is refused, selects
  go last). Tier A. Size M. Owns `crates/geo/src/recipe.rs`, `crates/endpoint/src/packs/geo.rs`,
  `crates/endpoint/src/packs/geo/`, the `geo` feature lines of `crates/endpoint/Cargo.toml`.
  Done 2026-10-05: `cargo test -p engine-geo --features all --lib recipe` 3 green (same recipe same digest, seed moves it, 8 typed
  refusals, cube minus cylinder then unwrap, lod, collision with digest x2); `cargo test -p engine-endpoint --features geo --lib
  packs::geo` 12 green (cube minus cylinder, unwrap with 0 overlapping chart pairs, lod, collision, digest x2, a different cutter
  moves the digest, 7 refusals with the legal values, `geo.apply` commits the modeled mesh and `tx.undo` removes it) and the
  same command with `--features mcp` 10 green (modeling ops refuse `feature_off`); `cargo tree -p engine-endpoint -e normal`
  shows no geo or C++ donor without the feature; clippy -D warnings clean on both feature sets; golden unchanged.
