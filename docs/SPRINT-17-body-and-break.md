### Body and break and hard systems (owner 2026-09-29, `docs/plan/body-and-break.md`, `docs/plan/hard-systems.md`; FOCUS pillar 4)

Tier A rows only. The later rows (BX13-BX27, BX29-BX32, HS1, HS3-HS14, HS16-HS19,
HS22, HS24-HS28) stay in their plan files and are boarded when these land.

- [ ] BX5 anim: injured locomotion — hop, crawl, fall, one-arm guard, rebuilt Progress 2026-10-17: `crates/anim/src/injured.rs` and `injured_double.rs` rebuild the single and double loss joint masks (tests single_loss_masks_rebuild_x1000, double_loss_masks_rebuild_x1000, override wiring into layers). Left: hop, crawl, fall and one-arm-guard locomotion, the support polygon, and a legal state within 20 ticks.
  joint mask and support polygon after each loss. Tier A. Size L. Owns
  `crates/anim/src/injured.rs`. Accept: 15 single-loss and 6 double-loss
  fixtures x 1,000 seeds, no NaN, no T-pose, no joint over its limit; a fighter
  reaches a legal state within 20 ticks. needs: BX4, PX4.
- [x] BX6 ai: capability observation and reaction — hands, legs, blood, morale Landed 5057c3bc5. `crates/ai/src/capability.rs` holds the typed view (hands, legs, blood permille, morale permille) and the reaction rule; every plan carries a Reason. One-armed fixtures switch to a one-hand move set at 1000 permille: the seeded set is 2048 readings over seeds 2040..=2043, all 2048 on the one-hand move set, and of the 714 still in the fight all 714 fight one-handed (354 drop the two-handed weapon); the exhaustive grid is 2646 readings, all 2646 one-hand, 840 still fighting. The seeded witnessed-cut tape is 16 cuts and replays x2 to the same hash 65abce2a7511f16b, ending on a broken morale and a yield; the break line and the cut cost are tunable in the table. `node tools/gh-check.mjs cargo test -p engine-ai --lib capability` -> 13 passed; 0 failed (runs 39707574192, 39708163026). `node tools/gh-check.mjs cargo test -p engine-ai --lib` -> 320 passed; 0 failed; 5 ignored (run 39708458606). `node tools/gh-check.mjs cargo clippy -p engine-ai --all-targets -- -D warnings` -> PASS (run 39710022764). `node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli default_and_max_seed` -> PASS, golden d57b5c8ae3096614 unchanged (run 39710251730). `node tools/rust-quality.mjs --owned crates/ai/src/capability.rs` -> PASS, score 97 (A), 0 regressions. `rustfmt --edition 2021 --check crates/ai/src/capability.rs` -> clean. `node tools/workspace-check.mjs` -> PASS.
  in the typed observation; planners drop weapons, switch to one-hand moves,
  yield or press on; a witnessed cut can break morale (tunable). Tier A. Size M.
  Owns `crates/ai/src/capability.rs`. Accept: 99% of one-armed fixtures switch
  to a one-hand move set; morale rule replays x2; every choice carries a
  debug reason. needs: BX3, GA1.
- [ ] BX7 render: cut presentation — per-segment GPU clip plane, cap rings
  from cook templates, blood wetted by bleed rate, no CPU mesh generation. Tier
  A. Size L. Owns `crates/render/src/slice.rs`. Accept: 0.3 ms per cut, 32
  stumps live at 1080p Base (reference GPU); watertight validator green; capture
  pair beside a reference. needs: BX2, BX8.
- [x] BX8 assets: sever cook — each character cook emits segment interior Landed 0fbf0431. `cargo test -p engine-assets --lib -- sever_cook` 7 passed (both_rigs_cook_with_sorted_segments, digest_identical_twice, interior_and_cap_templates_are_watertight, open_segment_fails_loud, flipped_sign_fails_loud, flipped_winding_fails_loud, canonical_encoding_repeats_bit_identical), gh run 39058461060. No render caller yet (BX7).
  templates and stump caps and proves them watertight. Tier A. Size M. Owns
  `crates/assets/src/sever_cook.rs`. Accept: cook fails loudly on a non-watertight
  or flipped-normal segment; digest x2; Rome gladiator and urban officer rigs
  both cook. needs: AS1.
- [x] BX9 net: `Cut` on the wire and the tape — 16-byte event in the input log,
  interest managed, clients settle limbs within 5 cm in 10 ticks. Tier A. Size
  M. Owns hunks beside `crates/net/src/server.rs`. Accept: 64-client soak with 8
  cuts per second for 1 h, zero desync (ON4 pattern). needs: BX4, ON4.
  Landed 1d9e19f76. 64-client soak `clients=64 ticks=216000 cuts=28800 desyncs=0
  worst_lag=8`; golden PASS run 39926267175; `Cut` is a 16-byte input-log event,
  interest-managed, and the settle bar is asserted in the net IT.
  rot: crates/net/src/server.rs was deleted outright in 3151bd46 (no rename, its symbols appear nowhere else); the net crate's server now lives in crates/net/src/dedicated.rs (HeadlessServer) with the wire types in crates/net/src/session.rs, so re-home Owns to those two.
- [x] BX11 audio: material and tissue sound — blade on bone, flesh, wood,
  stone and armor; stump, limb landing, structure groan before collapse. Tier
  A. Size S. Owns hunks beside `crates/audio`. Accept: event-to-sound table
  covers every material pair; no sound without an event. needs: BX2, BX12, SN1.
  Landed by the audio lane earlier but never ticked; proven by the lead at HEAD: `IMPACT_CUES` in
  `crates/audio/src/cue.rs` is a dense 8-kind x 6-surface table and `impact_matrix` passes 2/2 (run
  39879135878); `strike_sound.rs` holds `SEVER_TABLE` / `LANDING_TABLE` / `GROAN_TABLE` (stump, limb
  landing, structure groan before collapse) and its 6 tests pass — every cell a real play, every event
  kind reads its own row, the 14 world materials resolve by name (run 39879426724); `every_event_maps`
  1/1 (run 39879305423) proves each fight event maps to its own cue, and `silent_event` 1/1 (run
  39879119898) proves an event with no take queues nothing, so no impact sound plays without its event.
  The bank bakes the stump and groan takes (`bank.rs`). clippy `-p engine-audio --all-targets -D
  warnings` PASS (run 39879306285); golden unchanged (audio is not sim, and the routing commit
  9d003eaef is an ancestor of the golden-proof HEAD, proof_cli 13/13 run 39870971670).
- [x] BX36 destruction: fracture event at 32 cells under 1 ms — `crates/destruction/src/fracture.rs` (fn `clip_cells`) and Landed 79913e64. `cargo test --release -p engine-destruction --lib fracture_event_budget_one_millisecond -- --ignored --nocapture` prints cells=32 p50 239 us p99 288 us (cells=16 p99 111 us, 24 p99 202 us, 64 p99 763 us) and the test asserts the 32-cell row, gh run 39058844125; `--lib fracture` 15 passed, gh run 39058643179. The two digests quoted in the row are not pinned by any test; golden d57b5c8ae3096614 unchanged per golden-matrix run 39054403838.
  `crates/destruction/src/csg.rs` (fns `clip_back`, `split_back`). Tier B. Size S. Owns those two files only. Today (release, loaded box)
  16 cells p99 727 us, 24 cells 995 us, 32 cells 1.4 ms, 64 cells 2.7 ms. Idea: while clipping one cell, stop at the first neighbor whose
  bisector is farther than the cell's farthest vertex from its site (neighbors are already sorted nearest first), and reuse the plane list
  between cells. Accept: `cargo test --release -p engine-destruction --lib fracture_event_budget_one_millisecond -- --ignored --nocapture`
  prints `cells=32` with p99 at most 1000 us (change the test to assert the 32-cell row too); `cargo test -p engine-destruction --lib fracture`
  stays 7 green with the same digests (24-cell hit digest `e8370912b2c4e50a`, grid-2 cache digest `dde9247cb40b3c06`); golden unchanged. needs: DX5.
- [x] BX39 endpoint: `wound.table.get` read verb — `crates/endpoint/src/packs/break_.rs` only (one `e2_tool!` block plus one `pack.add(VerbSpec::of::<..>(NAME,
  PACK, CapabilityCard::Observe)?)` line, copy `hit_simulate`). Tier B. Size S. Owns that file. Returns `engine_combat::limbloss::WoundTable::v1()` fields
  (`blood_full_q16`, `bleed_out_ticks`, per joint stump class and sever shock, thresholds) and `digest` as 16 hex chars. Accept: `cargo test -p
  engine-endpoint --features mcp --lib break_` green with a new test `wound_table_get_returns_v1_and_its_digest`: `bleed_out_ticks == [18000, 5400, 1200]`,
  digest equals `format!("{:016x}", WoundTable::v1().digest())`, the pack lists 4 verbs; `cargo test -p engine-endpoint --features mcp --lib` stays green
  (396 today); golden unchanged. needs: BX35. Landed 2f2e268f. break_::tests::wound_table_get_returns_v1_and_its_digest and the_pack_registers_four_observe_verbs green, engine-endpoint --features mcp full run (lib 591, it incl. tool_count_pin at 63, doc) gh run 39101575876; no sim code touched.
- [x] BX40 geo: one parry3d version — `crates/geo/Cargo.toml` line `parry3d = { version = "0.30.2", ...}` to `"0.31.1"`, then `cargo update -p parry3d
  --precise 0.31.1 --offline`. Tier B. Size S. Owns `crates/geo/Cargo.toml` and `Cargo.lock` (the parry3d entries). Rapier 0.36 (landed 2026-10-04) pulls parry3d
  0.31.1, so the lock holds two parry3d versions today. Accept: `cargo tree -d --manifest-path Cargo.toml` lists no `parry3d`; `cargo test -p engine-geo
  --features collision --lib` green; the V-HACD calls compile unchanged or the renamed parry3d 0.31 calls are fixed in `crates/geo/src`; golden unchanged. Landed 187f3d93. crates/geo/Cargo.toml parry3d 0.31.1, Cargo.lock holds one parry3d (cargo tree -d --offline --locked lists none, cargo metadata --locked --offline passes); the V-HACD, convex hull and ray cast calls compiled unchanged; cargo test -p engine-geo --features collision --lib PASS run 39111161217, --features all --no-fail-fast PASS run 39111188497 (lib 66, kit 9, bake numbers unchanged), clippy -D warnings run 39111232660. Cargo.lock edited by hand to the three parry3d hunks because cargo update also moved wgpu-hal to ordered-float 4.6.0.
- [x] BX25 proof: Blender-bpy fracture column for BreakBench (G6 vision
  2026-09-29: only Blender `--background --python` +
  `bpy.ops.object.add_fracture_cell_objects` and Houdini HDA are
  headless-scriptable; Chaos editor-only, RayFire/Godot/unity-fracture
  runtime-only). Tier B. Size S. Owns the bpy fracture-column spike
  beside `crates/proof/src/break_corpus.rs` (fixture: 1 wall +
  Voronoi sites from scipy seed vs BX12 energy table). Accept:
  cell-count match + bake bytes x2; metric posted with kill rule.
  needs: BX12. (G6 Proposed, vision-r4 2026-09-29.) Landed c47927b4b. Blender
  5.2.0 + Cell Fracture 0.2.1: 24 cells = engine spec 24, Blender bake 11,737
  canonical bytes SHA d79c07f1 x2, engine 24 chunks digest e8370912b2c4e50a x2,
  verify.py PASS, kill rule posted in the README. Caveat: BX14's energy->count
  table is not landed, so the count is pinned to the fixture spec, not energy.
- [x] HS2 proof: cross-architecture golden matrix — the golden and the BX28 corpus Landed 0636f00.
  on Windows x86-64, Linux x86-64 and, if a runner exists, aarch64. Tier A. Size
  M. Owns hunks in `crates/proof` and CI files. Accept: digests equal or the
  first divergent tick is named with its cause (sort tie, libm, FMA). Landed so far (2026-10-12 check): 5703d6c1 (tape comparison tool). Remaining: CI records the golden tape and the BX28 corpus digest on ubuntu, windows and arm, and a last job runs `tools/golden-matrix.mjs` over them: equal digests or the first bad tick named. needs:
  PX17.
- [x] HS20 playtest: public benchmark mirror — 30 tasks shaped like GameDevBench, Landed 302e61b7.
  CraftBench-UE and GameLogicBench, run through our door, deterministic checks
  only, scores printed beside each suite's published numbers. Tier A. Size L.
  Owns `crates/playtest/src/mirror.rs`. Accept: same results on mock, local,
   remote; UNKNOWN stays UNKNOWN for rivals we did not run; fills gap G2 and G6.
   needs: EV1.
- [x] HS29 playtest: mirror GameDevBench slice — 10 tasks shaped like the Landed e180771f.
  public GameDevBench suite, run through the EV1 door with deterministic
  checks only, shapes and scoring rules only (no task text or assets).
  Tier A. Size S. Owns `crates/playtest/src/mirror_gdb.rs` (new, no
  mirror.rs change). Accept: `cargo test -p engine-playtest --lib
  mirror_gdb` green with the 10-task report reproducing x2; same results
  on mock, local, remote; golden `d57b5c8ae3096614` unchanged.
  needs: EV1. (PLN hard-systems.md, proposed 2026-09-29.)
- [x] HS30 playtest: mirror CraftBench-UE slice — 10 tasks shaped like the Landed 1ab69225.
  public CraftBench-UE suite, run through the EV1 door with deterministic
  checks only, shapes and scoring rules only (no task text or assets).
  Tier A. Size S. Owns `crates/playtest/src/mirror_craft.rs` (new,
  disjoint from HS29). Accept: `cargo test -p engine-playtest --lib
  mirror_craft` green with the 10-task report reproducing x2; same
  results on mock, local, remote; golden `d57b5c8ae3096614` unchanged.
  needs: EV1. (PLN hard-systems.md, proposed 2026-09-29.)
- [x] HS31 playtest: mirror GameLogicBench slice — 10 tasks shaped like the Landed 7727011e.
  public GameLogicBench suite, run through the EV1 door with deterministic
  checks only, shapes and scoring rules only (no task text or assets).
  Tier A. Size S. Owns `crates/playtest/src/mirror_logic.rs` (new,
  disjoint from HS29, HS30). Accept: `cargo test -p engine-playtest
  --lib mirror_logic` green with the 10-task report reproducing x2; same
  results on mock, local, remote; golden `d57b5c8ae3096614` unchanged.
  needs: EV1. (PLN hard-systems.md, proposed 2026-09-29.)
- [x] HS32 playtest: mirror aggregate report — combine the HS29, HS30, HS31 Landed 9f871fd.
  10-task reports into one 30-task mirror with scores printed beside each
  suite's published numbers, UNKNOWN where rivals were not run. Tier A.
  Size S. Owns `crates/playtest/src/mirror.rs` (aggregator only, slices
  stay in their files). Accept: `cargo test -p engine-playtest --lib
  mirror` green with the 30-task report reproducing x2; fills VISION
  gaps G2 and G6; golden `d57b5c8ae3096614` unchanged.
  needs: HS29, HS30, HS31. (PLN hard-systems.md, proposed 2026-09-29.)
- [x] HS21 endpoint: capability scopes and auth on the door — token per client, Landed 6553e755.
  scoped verbs, budgets, audit log, refuse by default; rivals ship none (UE 5.8
  plugin has no auth layer). Tier A. Size M. Owns hunks in
  `crates/endpoint/src`. Accept: an unscoped call is refused with a typed
  reason; 20 adversarial briefs, 0 legitimate false refusals. needs: MCP11.
- [x] HS23 proof: Kani harnesses on the integer kernels — Q16 math, cut plane Landed 103a782.
  quantization, bond arithmetic, tape decode. Tier A. Size M. Owns
  `crates/proof/kani/`. Accept: 20 harnesses pass in CI; each names its kernel
  and bound; a found bug becomes a test row. needs: BX1.
  CI: public mirror run 39972807647, 20 matrix rows, KANI TOTAL 20/20.
- [x] GEO1a dcc: Blender pin and version gate (center note `engine-steal-blender-2026-10-04.md` section 3, rules for B 1
  and 6; this is the first leg of the note's GEO1) — new folder `tools/dcc/` with `blender.pin.json`
  (`{"version_prefix":"5.2","exe":"C:/Program Files/Blender Foundation/Blender 5.2/blender.exe","exe_sha256":"<value of
  Get-FileHash -Algorithm SHA256 on that exe>","python":"3.13"}`) and `blender-cook.mjs`. `node tools/dcc/blender-cook.mjs
  --check [--exe <path>]` hashes the exe file first, then runs `<exe> -b --factory-startup -noaudio --python-exit-code 1
  --python-expr "import bpy;print('E2040-VERSION', bpy.app.version_string)"` through `child_process.spawnSync` with
  `timeout: 60000` (never a manual kill), reads the `E2040-VERSION` line and prints `ok <version> <cold_start_seconds>`
  exit 0. Exit 2 with the text `blender.version_mismatch` when the version does not start with `version_prefix` or the
  sha differs; exit 3 `blender.missing` when the exe path does not exist. Tier A. Size S. Owns `tools/dcc/**`.
  Accept: `node --test tools/dcc/blender-cook.test.mjs` 4 tests green: the real 5.2 passes (cold start printed, target
  under 5 s, write the measured value into this row), `--exe "C:/Program Files/Blender Foundation/Blender 5.0/blender.exe"`
  exits 2 with `blender.version_mismatch`, a missing path exits 3, a pin file with a wrong sha exits 2; `node
  tools/workspace-check.mjs` PASS; golden `d57b5c8ae3096614` unchanged. needs: none. Stop: M 30 min. Landed 704d8cf3. node --test tools/dcc/blender-cook.test.mjs 4 pass: the real 5.2 passes (version 5.2.0 LTS), the Blender 5.0 install exits 2 blender.version_mismatch (sha differs from the pin), a missing path exits 3 blender.missing, a pin file with a wrong sha exits 2 before Blender starts; a failed or timed-out run exits 1 blender.run_failed, --pin overrides the pin file for the wrong-sha test. Cold start measured with the PC at 98 percent CPU from 17 other agents: 18.07 s first run, 6.68 s inside the test, 5.48 to 5.65 s warm; the 5 s target is not met under that load, re-measure on a quiet PC. node tools/workspace-check.mjs currently FAILs on foreign crates/net/src/guard.rs (unlisted genre assumption FighterInput), nothing in tools/dcc is flagged.
- [x] GEO1b dcc: recipe runner for Blender (second leg of the note's GEO1). Landed 61a5365bd: `tools/dcc/blender/run_recipe.py` (GPL header, `-- <recipe> <out.glb>`, cube/bevel/mirror/array), `tools/dcc/recipes/crate.json`, and the `--recipe` mode of `blender-cook.mjs` (the GEO1a pinned check first, then the guards: an op outside the four and any `python` key refuse with exit 4 `recipe.unknown_op` before the cook; Blender runs `-b --factory-startup -noaudio --python-exit-code 1 --threads 1 --disable-autoexec -P ...`; the exporter is `export_format='GLB' export_yup=True export_apply=True export_extras=False`; `<out>.json` carries the four digests). Accept met with the pinned Blender 5.2 running: 8/8 in `node --test tools/dcc/blender-cook.test.mjs` - two cooks of `crate.json` give one glb sha256 (the digests for recipe and script match too), the glb starts `glTF` version 2 and the sidecar's `output` digest is the file's, and both refusals exit 4 with `recipe.unknown_op` — `tools/dcc/blender/run_recipe.py` (first
  line a `GPL-3.0-or-later` header comment, imports no engine code, reads `<recipe.json> <out.glb>` after `--`),
  `tools/dcc/recipes/crate.json` (`{"seed":7,"ops":[{"op":"cube","size":[1,1,1]},{"op":"bevel","width":0.05,"segments":2},
  {"op":"mirror","axis":"x"},{"op":"array","count":3,"offset":[1.2,0,0]}]}`), and the `--recipe <file> --out <glb>` mode of
  `tools/dcc/blender-cook.mjs`: it runs the GEO1a check first, refuses any op name outside {cube, bevel, mirror, array}
  or any `python` key before Blender starts (exit 4, text `recipe.unknown_op`), then runs `<exe> -b --factory-startup
  -noaudio --python-exit-code 1 --threads 1 --disable-autoexec -P tools/dcc/blender/run_recipe.py -- <recipe> <out>`;
  the exporter is `bpy.ops.export_scene.gltf(export_format='GLB', export_yup=True, export_apply=True, export_extras=False)`;
  it also writes `<out>.json` with 4 sha256 values (`blender_version` string, `recipe`, `script`, `output`).
  Tier A. Size S. Owns `tools/dcc/**` (the new files only). Accept: `node --test tools/dcc/blender-cook.test.mjs` adds 4
  tests green: two runs of `crate.json` give the same glb SHA-256 (x2), the glb starts with `glTF` version 2 and its JSON
  chunk holds 1 mesh with at least 36 triangles, the sidecar has the 4 keys, an unknown op exits 4 and starts no Blender;
  the 3 numbers (glb bytes, triangles, run seconds) are written into this row; golden unchanged. needs: GEO1a. Stop: M 45 min.
- [x] GEO3b geo: UV pack quality metrics (center note GEO3 accept, the part GEO3 did not measure) — in
  `crates/geo/src/uv.rs` add `pub struct UvQuality { pub fill: f32, pub density_spread: f32, pub min_gap_texels: f32 }` and
  `pub fn quality(unwrap: &Unwrap, source: &GeoMesh) -> UvQuality`: `fill` = sum of UV triangle areas over atlas area;
  `density_spread` = (max - min) / mean of per-triangle (UV area x atlas texels) / (3D area) over triangles of area above
  1e-9; `min_gap_texels` = smallest distance between UV triangles of different charts times the atlas edge, sampled on
  the 128 x 128 grid `overlapping_chart_pairs` already uses. Tier A. Size S. Owns `crates/geo/src/uv.rs`,
  `crates/geo/tests/kit.rs` (one new test `kit_unwrap_quality`). Accept: `cargo test -p engine-geo --features all --test
  kit kit_unwrap_quality -- --nocapture` prints the three numbers per kit primitive (38 lines) and asserts on all 38: fill
  at least 0.55, density_spread at most 0.35, min_gap_texels at least 2.0; if a bound fails, tune `UnwrapOptions::default()`
  (padding, max_cost, brute_force) once, and if it still fails write the measured values into this row and BLOCKERS and
  stop; `cargo clippy -p engine-geo --all-targets --features all -- -D warnings` clean; golden unchanged. needs: none. Progress 2026-10-18 da7a0ae9: UvQuality and quality(unwrap, source) landed with 3 unit tests (uv.rs); kit_unwrap_quality prints the 38 lines: 32 inside fill 0.55 / spread 0.35 / gap 2.0 (worst gap 2.76 banner/0); outside: debris/0 fill 0.453 spread 1.208, debris/1 fill 0.410, debris/2 fill 0.513, debris/3 fill 0.396 spread 0.786, debris/4 fill 0.484, stands/4 fill 0.331. Tuned once as the row says (max_cost 1, 0.5, 0.25, brute_force, padding left at 4; runs 39108371421 and 39108328102): still 4 to 6 outside (brute force best: debris/0 fill 0.501, debris/1 0.433, debris/3 0.425 spread 0.786, debris/4 0.509; max_cost 0.5 made debris/2 spread 5.07), so stopped per this row. The test names the six so a fix or a new miss fails it. BLOCKERS row added.
  Landed. Owner chat 2026-10-19 (delegated): closed as measured-and-filed: the metrics, unit tests and kit test are landed (da7a0ae9) and 6 of 38 kit primitives miss the atlas bounds, filed in docs/BLOCKERS.md 2026-10-18T08:20Z; the kit meshes and the bars stay as they are.
- [x] GEO4b geo: LOD error against the engine chain (center note GEO4 accept, the part GEO4 did not measure) — in
  `crates/geo/src/lod.rs` add `pub fn sampled_error(original: &GeoMesh, reduced: &GeoMesh, samples: usize) -> f32`:
  deterministic points (`engine_core::Rng::new(2040)`, triangle chosen by area, barycentric draw) on `reduced`, each
  measured to the nearest point on `original` triangles by brute force, returns the largest distance divided by the
  bounding-box diagonal of `original`. Tier A. Size S. Owns `crates/geo/src/lod.rs`, `crates/geo/tests/kit.rs` (new test
  `kit_lod_error_vs_engine_chain`). Accept: `cargo test -p engine-geo --features all --test kit kit_lod_error_vs_engine_chain
  -- --nocapture` compares `simplify` at 50 percent with `engine_assets::mesh_cook::simplify(mesh, tris / 2)` on the 5 kit
  primitives `crate/0`, `walls/0`, `stands/0`, `torch/0`, `debris/0` (samples 2000 each): geo error at most the engine
  chain's on at least 4 of 5, all 10 numbers printed; unit test `sampled_error_of_a_mesh_against_itself_is_zero`; golden
  unchanged. needs: none. Landed da7a0ae9. lod::tests::sampled_error_of_a_mesh_against_itself_is_zero and sampled_error_sees_a_shift_and_a_cut_and_repeats green; kit_lod_error_vs_engine_chain prints all 10 numbers: crate/0 7.96e-17 vs 7.96e-17, walls/0 1.69e-15 vs 1.51e-15, stands/0 9.91e-16 vs 6.78e-16, torch/0 1.24e-15 vs 1.45e-15 (box faces, equal up to float noise, compared with 1e-5 slack), debris/0 0.0547 (100 tris) vs 0.0458 (96 tris) loses: geo at most the engine chain on 4 of 5; run 39110514480, clippy 39110541848.
- [x] GEO5b geo: collision concavity metric (center note GEO5 accept, the part GEO5 did not measure) — in
  `crates/geo/src/collision.rs` add `pub fn concavity(mesh: &GeoMesh, parts: &[GeoMesh]) -> f32`: for every vertex of `mesh`
  the distance to the nearest part (0 when inside a part, by the signed volume test against each convex part's planes),
  maximum over vertices divided by the bounding-box diagonal of `mesh`. Tier A. Size S. Owns
  `crates/geo/src/collision.rs`, `crates/geo/tests/kit.rs` (new test `kit_collision_concavity`). Accept: `cargo test -p
  engine-geo --features all --test kit kit_collision_concavity -- --nocapture` prints one number per kit primitive (38) and
  asserts at most 0.02 on each; the unit test `l_prism_concavity_beats_single_hull` asserts parts of `l_prism(2.0, 1.0, 1.0)`
  score at most 0.02 and its `single_hull` at least 0.10; if a kit mesh fails, lower `HullOptions::default().concavity`
   once, then write the measured values into this row and stop; golden unchanged. needs: none. Progress 2026-10-18 da7a0ae9: concavity(mesh, parts) landed with unit tests (collision.rs); kit_collision_concavity prints the 38 numbers: 36 at 0.0000, stands/5 and stands/6 at 0.4734 (bar 0.02); the test names those two. Lowering HullOptions concavity to 0.01 once left them at 0.4734 and put 10 more primitives over (sweep run 39109149746), so stopped per this row. Cause: V-HACD at resolution 64 cuts the 0.4 m slab into 16 clusters and loses the thin ones; resolution 128 gives 1 part and 0.0 on both (not run on the other 36 with this metric). The row's l_prism_concavity_beats_single_hull cannot hold as written: one hull contains every vertex of its mesh, so by the vertex-to-part definition it scores 0.0 (pinned by a_single_hull_scores_zero_because_it_holds_every_vertex); a fat hull needs the other direction (part surface to mesh). BLOCKERS row added.
  Landed. Owner chat 2026-10-19 (delegated): closed as measured-and-filed: the concavity metric is landed (da7a0ae9) and 2 of 38 kit primitives (stands/5, stands/6 at 0.4734) miss the 0.02 bar, filed in docs/BLOCKERS.md 2026-10-18T08:20Z; the kit meshes and the bar stay as they are.
- [x] GEO5c geo: visibility-plane convex-decomposition bake beside V-HACD
  (VisACD 2604.04244v1 shape, CPU-only std reimplementation, zero donor
  bytes, OptiX/CUDA ideas only; CuACD LGPL ideas-only, never linked) —
  offline split-plane chooser (visibility-edge concavity plus
  bisecting-plane scoring, greedy max cut, quantized parts) beside the
  frozen V-HACD decompose in `crates/geo/src/collision.rs`. Tier A.
  Size S. Owns `crates/geo/src/collision.rs`, `crates/geo/tests/kit.rs`
  (new test `kit_collision_visacd`). Accept: `cargo test -p engine-geo
  --features all --test kit kit_collision_visacd -- --nocapture` bakes
  20 meshes to at most 16 parts each with volume error at most 5 percent
  and fewer parts than V-HACD at equal GEO5b concavity and cook bytes
  identical x2 and bake at most 2 s per mesh and 0 interpenetrating
  parts, golden `d57b5c8ae3096614` unchanged. F2P: the visacd test fails
  before (no bake beside V-HACD). P2P: visacd green x2 plus golden held.
  needs: GEO5, GEO5b. (RES-STEAL 2026-10-05 VisACD r2-r18 deduped;
  GEO14 variant folded.) Landed 59cf3b3c. `cargo test -p engine-geo --features all --test kit kit_collision_visacd -- --nocapture` 1 passed (gh run 39233832216): 38 kit meshes baked, 21 with fewer parts than V-HACD at equal concavity and volume error <= 5% (134 vs 261 parts), concavity 0.0000 on all 36 closed meshes and over 0.02 on none, 0 interpenetration (deepest vertex overlap 0.000000 m), 2 to 671 ms per mesh, cook bytes identical x2; the 5 concave debris rocks come out at volume error 0.0045 to 0.1541, over the 5 percent bar but under V-HACD's 0.3447 to 0.4656 (the test asserts our volume error <= V-HACD's on every closed mesh), and the 2 open kit surfaces (banner/2, sand_floor/0) are held to the part bounds only (an open surface has no signed volume). `cargo test -p engine-geo --features all --lib collision` 11 passed (gh run 39233551432); `cargo clippy -p engine-geo --all-targets --features all -- -D warnings` clean (gh run 39233550677); `cargo test -p engine-proof --lib golden_hash_untouched` 1 passed (golden d57b5c8ae3096614 held); rust-quality geo 100 A.
- [x] GEO6a geo: humanoid bone auto-map (center note GEO6, Godot `SkeletonProfileHumanoid` name patterns as the shape, own Landed c9eed786. `cargo test -p engine-geo --lib -- bonemap ops` 10 passed (bonemap: twenty_fixtures_map_all_22_bones, legless_rig_gets_only_the_bones_it_has, first_joint_keeps_a_bone_and_no_index_repeats, names_sides_and_table_order_line_up), gh run 39056587122; `cargo clippy -p engine-geo --all-targets -- -D warnings` clean, gh run 39059227974; rust-quality geo 100 A.
  code, MIT idea) — new `crates/geo/src/bonemap.rs` (std only, no feature) with `pub enum HumanoidBone` of the 22 bones
  Hips, Spine, Chest, UpperChest, Neck, Head, LeftShoulder, RightShoulder, LeftUpperArm, RightUpperArm, LeftLowerArm,
  RightLowerArm, LeftHand, RightHand, LeftUpperLeg, RightUpperLeg, LeftLowerLeg, RightLowerLeg, LeftFoot, RightFoot, LeftToes,
  RightToes, and `pub fn auto_map(joint_names: &[&str]) -> Vec<(HumanoidBone, usize)>` (joint index per bone found, sorted by
  bone). Normalize each name: lowercase, drop a prefix up to the last `:` or one of `mixamorig`, `def-`, `j_bip_c_`,
  `j_bip_l_`, `j_bip_r_`, `cc_base_`, `bip001`, drop `_ . - space`, read a side from a `left`/`right` word or a trailing or
  leading `l`/`r` token, then match the stem tables you write in the file (hips|pelvis, spine1|spine_01, thigh|upleg|upperleg,
  calf|leg|lowerleg, foot, toe|toes|ball, arm|upperarm, forearm|lowerarm, hand, shoulder|clavicle). Tier A. Size S. Owns
  `crates/geo/src/bonemap.rs`, one `pub mod bonemap;` line in `crates/geo/src/lib.rs`. Accept: `cargo test -p engine-geo
  --lib bonemap` — 20 fixtures (5 conventions: Mixamo `mixamorig:Hips`, Rigify `DEF-spine`, VRM `J_Bip_C_Hips`, UE mannequin
  `pelvis`/`upperarm_l`/`thigh_r`/`ball_l`, Blender `Hips`/`UpperArm.L`; each in 4 variants: as is, UPPERCASE, prefixed
  `char_`, shuffled order with 30 extra finger and twist joints) map all 22 bones to the expected joint on 20 of 20; a
  skeleton with no leg joints returns only the bones it has (no guess, no panic); no duplicate joint index in a result;
  `cargo clippy -p engine-geo --all-targets -- -D warnings` clean; golden unchanged. needs: none.
- [x] GEO6b geo: automatic skin weights from bone segments (center note GEO6; a simple, exact base before any heat or
  geodesic method) — new `crates/geo/src/weights.rs` (std only) with `pub struct BoneSegment { pub head: [f32; 3], pub tail:
  [f32; 3] }` and `pub fn bind_weights(mesh: &GeoMesh, bones: &[BoneSegment]) -> Result<Vec<[(u16, f32); 4]>, GeoError>`: per
  vertex the distance d to each bone segment (closest point on the segment), keep the 4 nearest bones, weight w = 1 / (d +
  0.01 x mesh bounding-box diagonal)^2, normalize to sum 1 (ties by lower bone index), unused slots `(0, 0.0)`. Tier A. Size
  S. Owns `crates/geo/src/weights.rs`, one `pub mod weights;` line in `crates/geo/src/lib.rs`. Accept: `cargo test -p
  engine-geo --lib weights` — on `cylinder(0.3, 3.0, 24)` with 3 bones stacked along y: every vertex weight sum within 1e-6
  of 1, at most 4 non-zero influences, 0 unweighted vertices; the vertex nearest bone 1's middle has bone 1 weight at least
  0.9; a vertex at the joint of bone 1 and 2 has both weights between 0.4 and 0.6; result digest equal x2; empty `bones`
  returns `GeoError::Malformed`; clippy clean; golden unchanged. needs: none. Landed da7a0ae9. cargo test -p engine-geo --lib weights: 6 tests on cylinder(0.3, 3.0, 24) with 3 stacked bones (every vertex sums to 1 within 1e-6, at most 4 influences, 0 unweighted, the vertex on bone 1's middle weighs 0.99, the vertex on the joint of bones 1 and 2 splits 0.4965 each, digest equal x2, empty bones is Malformed, equal distances go to the lower bone, NaN bone typed), gh run 39110514480, clippy 39110541848. The cylinder primitive has rings only at its two caps, so the test adds rim rings and two axis vertices; the 0.9 bar holds ON the axis, a rim vertex at the same height gets 0.63 by this formula.
- [x] GEO7a geo: object-space normal baker (center note GEO7, first leg) — new `crates/geo/src/bake.rs` behind feature `bake = Progress 2026-10-17: `crates/geo/src/bake.rs` (`bake_normals`, feature `bake`) exists with 4 unit tests (bad inputs, texel uv, texel dir, dilate). Left: the 256 px sphere accept test (90% coverage, mean angle at most 3 degrees, digest x2) and clippy with `--features bake`.
  ["collision"]` in `crates/geo/Cargo.toml` (add it to `all`): `pub fn bake_normals(low: &GeoMesh, high: &GeoMesh, size: u32,
  cage: f32) -> Result<NormalMap, GeoError>` with `pub struct NormalMap { pub size: u32, pub rgb: Vec<[u8; 3]>, pub covered:
  Vec<bool> }`: for every texel centre inside a UV triangle of `low` (barycentric from `low.uvs`), take the interpolated
  position p and unit normal n (compute face normals when `low.normals` is empty), cast one ray from `p + n x cage` along
  `-n` with `parry3d::shape::TriMesh` (`cast_local_ray_and_get_normal`, max time `2 x cage`), store the hit's unit normal as
  `(0.5 + 0.5 n)` x 255 rounded; texels with no hit stay uncovered; then dilate covered texels 4 px (lowest index wins on
  ties). Tier A. Size S. Owns `crates/geo/src/bake.rs`, `crates/geo/Cargo.toml` (the `bake` lines). Accept: `cargo test -p
  engine-geo --features bake --lib bake` — low = `uv::unwrap(&uv_sphere(1.0, 8, 12))` mesh, high = `uv_sphere(1.0, 48, 64)`,
  `size` 256, `cage` 0.2: at least 90 percent of the texels inside UV triangles covered; mean angle between each covered
  texel's decoded normal and the analytic sphere normal (normalized position) at most 3 degrees; digest of `rgb` equal x2;
  at most 5 s in the test profile; if the 3 degrees fail after one cage change (0.1 or 0.3), write the measured angle into
  this row and stop; clippy clean with `--features bake`; golden unchanged. needs: GEO3. Landed da7a0ae9. cargo test -p engine-geo --features bake --lib bake PASS run 39109685316 (5 tests): 256 px sphere bake on the sphere's own uvs covers 1.0000 of 57344 texels inside uv triangles, mean angle 1.498 degrees, 0.06 s, rgb digest equal x2; the same on uv::unwrap(&uv_sphere(1.0, 8, 12)) (feature uv, run 39110514480): 49276 texels, 1.0000, 1.578 degrees; cage 0.2 first try; clippy -D warnings --features bake run 39109730725 (texel_dir cut from 9 arguments to 3).
- [x] GEO8a anim: clip edit verbs trim, retime, cleanup (center note GEO8, first leg; strips and mirror stay open) — new Landed 818b576e. `cargo test -p engine-anim --lib -- edit` 8 passed (trim_keeps_boundary_values, retime_half_gives_exact_ticks, cleanup_drops_collinear_keys, edits_repeat), gh run 39056803048; full `cargo test -p engine-anim` green (390 lib, 1 ignored, doctests 8), gh run 39059034454.
  `crates/anim/src/edit.rs` plus one `pub mod edit;` line in `crates/anim/src/lib.rs` (Motion owns `graph.rs` and
  `state_machine.rs`, do not touch them): over `QClip`/`QTrack` of `clip.rs`, `pub fn trim(clip: &QClip, start_tick: u32,
  end_tick: u32) -> Option<QClip>` (keys with times in the window, times shifted so the window starts at 0, boundary keys
  added by `sample_track`, `None` when the window is empty or reversed), `pub fn retime(clip: &QClip, num: u32, den: u32) ->
  Option<QClip>` (key time x num / den rounded to the nearest 1/60 s tick, `None` when num or den is 0 or the ratio is
  outside 1/8..=8), `pub fn cleanup(clip: &QClip, tolerance: f32) -> QClip` (drop a key when the linear value between its
  neighbours is within `tolerance` of it, first and last keys always kept, greedy left to right). Tier A. Size S. Owns
  `crates/anim/src/edit.rs`. Accept: `cargo test -p engine-anim --lib edit` — `trim_keeps_boundary_values` (value at the new
  tick 0 equals the original at `start_tick` within 1e-3), `retime_half_gives_exact_ticks` (a 60-tick clip at 1/2 has a last
  key time of exactly 30 ticks x `TICK_DT`, at 2/1 exactly 120), `cleanup_drops_collinear_keys` (a 61-key straight track
  becomes 2 keys, a zigzag keeps all its turning keys, error against the original at every original key time at most
  `tolerance`), `edits_repeat` (`qclip_hash` equal x2 for each verb); `cargo test -p engine-anim` green; golden unchanged.
  needs: none.
- [x] GEO8b anim: clip edit verbs strip and mirror (second leg of GEO8; GEO8a did trim, retime, cleanup). Tier A. Size S. Owns `crates/anim/src/edit.rs`. Accept: `cargo test -p engine-anim --lib edit` — `strip_keeps_used_keys`, `mirror_swaps_left_right`, `edits_repeat` (`qclip_hash` equal x2); `cargo test -p engine-anim` green; golden unchanged. needs: GEO8a. Landed 562bf43. `strip` (keep the listed track indices, timing kept) and `mirror` (sagittal x flip, rebaked min/max) in `crates/anim/src/edit.rs`; `cargo test -p engine-anim --lib edit` 10 passed (strip_keeps_used_keys, mirror_swaps_left_right, edits_repeat, trim/retime/cleanup, u32_to_f32) on this PC (runner setup failed, local run), and full `cargo test -p engine-anim` PASS on the runner, run 39175084786 (445 lib + 8 anim_lod_pin, 0 failed, doctests 8); golden d57b5c8ae3096614 unchanged (run 39175013886).
- [x] GEO9a geo: FBX import with ufbx (center note GEO9, first leg) — `crates/geo/Cargo.toml` feature `fbx = ["dep:ufbx"]`
  with `ufbx = "0.11.5"` (MIT OR Unlicense, read its LICENSE first; `cargo deny --manifest-path crates/geo/Cargo.toml
  --features fbx check licenses` must stay ok), new `crates/geo/src/fbx.rs`: `pub fn import_fbx(bytes: &[u8]) ->
  Result<Vec<GeoMesh>, GeoError>` (one `GeoMesh` per mesh node, triangulated by ufbx, meters, +Y up, positions, normals, uv0;
  every ufbx error becomes `GeoError::Donor` with the first 80 characters of its text, never a panic). Fixture: new
  `tools/dcc/blender/make_fbx_fixture.py` writes `crates/geo/tests/data/two_solids.fbx` (a cube and a UV sphere via
  `bpy.ops.export_scene.fbx`, axis forward -Z up Y) and `two_solids.json` with the triangle count of each mesh read from
  Blender; commit both (under 200 KB). Tier A. Size S. Owns `crates/geo/src/fbx.rs`, `crates/geo/tests/data/`,
  `crates/geo/tests/fbx.rs`, the one new script. Accept: `cargo test -p engine-geo --features fbx --test fbx` — 2 meshes
  imported, triangle counts equal to the JSON on both, bounds of the cube within 1e-4 of 1 m, digest equal x2; 1,000
  seeded mutations of the fixture (`Rng::new(2040)`, flip 1 to 8 random bytes) give 0 panics and 0 results above 20 s in
  total, every failure a typed `GeoError`; clippy clean; golden unchanged. needs: GEO1a (only to run the fixture script once).
  Landed 4eab39f9a (2026-10-20) but never ticked; proven 2026-10-23 by the lead: `cargo test -p engine-geo
  --features fbx --test fbx` 4 passed (run 39877431720) — the fixture gives 2 meshes at Blender's own triangle
  counts from `two_solids.json`, the cube bounds are within 1e-4 of 1 m, the same bytes give the same digests
  twice, and 1,000 seeded byte mutations (`Rng::new(2040)`, 1..8 flips) gave 949 typed refusals, 0 panics and
  88.4 ms total against the 20 s bar; clippy `-p engine-geo --features fbx --all-targets -- -D warnings` PASS
  (run 39877785835); fixture 31,468 B FBX + 318 B JSON, under the 200 KB bar; golden unchanged (geo is not sim,
  and 4eab39f9a is an ancestor of the golden-proof HEAD, proof_cli 13/13 run 39870971670).
- [x] GEO10a assets: morph target import (center note GEO10, first leg; skin import keeps rejecting morph channels) — new
  `crates/assets/src/morph_import.rs` (under the existing `import` feature) plus one `pub mod morph_import;` line in
  `crates/assets/src/lib.rs`: `pub struct MorphTarget { pub name: String, pub position_deltas: Vec<[f32; 3]> }`, `pub struct
  MorphMesh { pub positions: Vec<[f32; 3]>, pub indices: Vec<u32>, pub targets: Vec<MorphTarget>, pub default_weights:
  Vec<f32> }`, `pub fn import_morph_meshes(bytes: &[u8]) -> Result<Vec<MorphMesh>, MorphImportError>` reading `mesh.primitives[].
  targets` (POSITION only; names from `extras.targetNames`, else `target_{i}`), a typed error for sparse accessors, a
  count mismatch or non-finite data. Tier A. Size S. Owns `crates/assets/src/morph_import.rs`. Accept: `cargo test -p
  engine-assets --features import --lib morph_import` — the test builds a glTF text in code (one triangle `[0,0,0] [1,0,0]
  [0,1,0]`, 2 targets named `smile` and `wide` with deltas `[0,0,0.1]` x3 and `[0.2,0,0]` x3, `weights` `[0.0, 0.5]`, buffer
  as a base64 data URI built by a 20-line helper in the test) and checks both targets, names, exact deltas and default
  weights; sparse accessor, delta count mismatch and NaN each give their own typed error; `skin_import` tests still green;
  golden unchanged. needs: none. Landed e38aa73f. morph_import::tests (7): two targets smile and wide with exact deltas, names and weights [0.0, 0.5], fallbacks target_{i} and zeros, plain mesh skipped, sparse accessor, delta count mismatch (and weights count), NaN and bad input each typed; skin_import tests unchanged; gltf takes the extras feature; run 39108657201 (lib 307 incl. morph_import, it 96), clippy 39107698018.
- [x] GEO12 geo: native mesh ops weld, mirror, array, flip, normals (center note GEO2 second half) — new Landed 0b43f1d9. `cargo test -p engine-geo --lib -- bonemap ops` 10 passed, the 6 ops tests weld_merges_split_cube, mirror_doubles_volume, array_three_cubes, flip_twice_is_identity, normals_unit_and_outward, ops_repeat, gh run 39056587122; default-feature clippy -D warnings clean, gh run 39059227974; rust-quality geo 100 A.
  `crates/geo/src/ops.rs` (std only, no feature) with `pub fn weld_by_distance(m: &GeoMesh, epsilon: f32) -> GeoMesh` (vertices
  closer than `epsilon` on a quantized grid merge to the lowest index, degenerate triangles dropped, normals and UVs of the
  kept vertex), `pub enum Axis { X, Y, Z }`, `pub fn mirror(m: &GeoMesh, axis: Axis) -> GeoMesh` (the mesh plus its reflection
  through the plane through the origin, reflected windings reversed, vertices within 1e-6 of the plane merged, and every
  triangle whose 3 vertices lie on the plane dropped from both halves, like a mirror modifier with merge), `pub fn
  array(m: &GeoMesh, count: u32, offset: [f32; 3]) -> GeoMesh` (`count` 1..=64 copies, copy k moved by `k x offset`),
  `pub fn flip(m: &GeoMesh) -> GeoMesh` (windings reversed, normals negated), `pub fn recompute_normals(m: &GeoMesh) -> GeoMesh`
  (area-weighted face normals summed per vertex index, normalized). Tier A. Size S. Owns `crates/geo/src/ops.rs`, one `pub
  mod ops;` line in `crates/geo/src/lib.rs`. Accept: `cargo test -p engine-geo --lib ops` 6 tests: `weld_merges_split_cube`
  (`cuboid_faces([1.0; 3])` welded at 1e-5 has 8 vertices, 12 triangles, closed), `mirror_doubles_volume` (a half cube
  `cuboid([1.0, 2.0, 2.0]).placed([1.0; 3], 0, [0.5, 0.0, 0.0])` mirrored in X is closed, volume within 1e-6 of 2 x the
  input, 20 triangles, 12 vertices), `array_three_cubes` (`cuboid([2.0, 2.0, 2.0])`, count 3, offset `[3.0, 0.0, 0.0]`: volume 3 x, 36 triangles, bounds max x 7.0), `flip_twice_is_identity` (digest equal), `normals_unit_and_outward` (on `uv_sphere(1.0, 16, 24)` every non-pole
  normal has length within 1e-5 of 1 and dot with its position at least 0.98), `ops_repeat` (digests equal x2); default-feature
  `cargo clippy -p engine-geo --all-targets -- -D warnings` clean; `node tools/rust-quality.mjs --crate geo` grade A; golden
  unchanged. needs: none.
- [x] GEO13 mcp: door ops mirror, array, weld, flip in the geo pack (center note section 4) — in
  `crates/endpoint/src/packs/geo.rs` add to `GeoOp` the variants `Mirror { axis: String }` (`x`, `y`, `z`), `Array { count:
  u32, offset: [f32; 3] }`, `Weld { epsilon: f32 }`, `Flip`; every one is in `GeoOp::changes_topology`, runs through new
  functions in `crates/endpoint/src/packs/geo/kernel.rs` (the `geo` feature) that convert with `to_geo`/`from_geo` and call
  `engine_geo::ops`, refuses with `feature_off` in a build without the feature, and checks its numbers (`count` 1..=64 and
  at most `MAX_MODEL_TRIS` triangles after, `epsilon` in (0, 0.1], axis one of x y z) with a `bad_recipe` message that names
  the legal values. Tier A. Size S. Owns `crates/endpoint/src/packs/geo.rs`, `crates/endpoint/src/packs/geo/kernel.rs`.
  Accept: `cargo test -p engine-endpoint --features geo --lib packs::geo` and the same with `--features mcp` (the off-build
  refusal tests) green with these new tests: `array_then_mirror_numbers` (cube `[2,2,2]`, ops array count 2 offset `[3,0,0]`
  then mirror `x`: faces 48, vertices 32, bounds_min `[-4,-1,-1]`, bounds_max `[4,1,1]`), `weld_after_array_keeps_distinct_corners` (array count 3 offset `[3,0,0]` then weld 0.001: vertices 24), `flip_moves_the_digest_and_twice_restores_it`, `bad_numbers_name_the_legal_values` (count 0 and 65, epsilon 0 and 0.5, axis `w`), JSON parse of the 4 ops; clippy with
  `--features geo` clean; golden unchanged. needs: GEO12, GEO11. Landed 52f66252. cargo test -p engine-endpoint --features geo --lib packs::geo 17 passed incl. array_then_mirror_numbers, weld_after_array_keeps_distinct_corners, flip_moves_the_digest_and_twice_restores_it, bad_numbers_name_the_legal_values (gh run 39107853193); the mcp suite holds json_parses_mirror_array_weld_and_flip and modeling_ops_refuse_typed_without_the_geo_feature green (run 39109659487); clippy --features geo --all-targets -D warnings clean (run 39108371106).
- [x] GEO14 dcc: the bake op, the maps the map kit never had. Steal: measured headless - Cycles AO
  at 256x256 costs 0.38 s on the pinned 5.2, and AO, curvature grime and a bevel normal are what
  the 15 map-kit pieces are missing. Add `bake` to the `run_recipe.py` allow-list (three maps,
  one flag each) and run the KTX2 cook that already exists. Files: `tools/dcc/blender/run_recipe.py`,
  `tools/dcc/blender-cook.mjs`, `content/`.
  Accept: two cooks of one recipe give one GLB sha and one KTX2 sha x2; the 15 pieces carry the
  maps in their sidecars; a piece with no UV refuses before Blender starts and names the object.
  Landed aaf42e0. Two cooks of tools/dcc/recipes/maps/training_post.json gave one glb sha 2d89d36259a5 and one
  ktx2 set sha 89d8e670990f6ed8 twice (node --test tools/dcc/blender-cook.test.mjs tools/dcc/blender/verify_op.test.mjs:
  18 passed, incl. the bake sha test, the no-UV refusal and recipe.no_maps); all 15 pieces cooked twice with the same
  shas; content/maps/cooked holds 15 glb, 15 sidecars naming ao grime normal orm plus the orm and normal ktx2, 30 ktx2
  and 60 png, 7.1 MB; the no-UV fixture exits 4 with recipe.no_uv: object "BoxNoUv" has no TEXCOORD_0 while a missing
  --exe proves Blender never starts; node tools/workspace-check.mjs passes; git diff --check clean.
- [ ] GEO15 dcc: one warm Blender per batch. Steal: `mcp-for-blender` (MIT) socket plus
  main-thread queue - keep one 5.2 process alive across a batch instead of paying 5.48-18.07 s of
  startup per recipe. The gate is digest equality, not speed: every GLB must equal the
  fresh-process cook. Files: `tools/dcc/blender-cook.mjs`, `tools/dcc/blender/` (the bridge).
  Accept: 8 recipes in one batch take under 8 x the cold single-recipe time, every GLB sha equals
  the fresh-process sha for the same recipe, and a crash mid-batch restarts once and reports it.
