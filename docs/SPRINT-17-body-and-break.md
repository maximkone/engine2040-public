### Body and break and hard systems (owner 2026-09-29, `docs/plan/body-and-break.md`, `docs/plan/hard-systems.md`; FOCUS pillar 4)

Tier A rows only. The later rows (BX13-BX27, BX29-BX32, HS1, HS3-HS14, HS16-HS19,
HS22, HS24-HS28) stay in their plan files and are boarded when these land.

- [ ] BX5 anim: injured locomotion — hop, crawl, fall, one-arm guard, rebuilt
  joint mask and support polygon after each loss. Tier A. Size L. Owns
  `crates/anim/src/injured.rs`. Accept: 15 single-loss and 6 double-loss
  fixtures x 1,000 seeds, no NaN, no T-pose, no joint over its limit; a fighter
  reaches a legal state within 20 ticks. needs: BX4, PX4.
- [ ] BX6 ai: capability observation and reaction — hands, legs, blood, morale
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
- [ ] BX8 assets: sever cook — each character cook emits segment interior
  templates and stump caps and proves them watertight. Tier A. Size M. Owns
  `crates/assets/src/sever_cook.rs`. Accept: cook fails loudly on a non-watertight
  or flipped-normal segment; digest x2; Rome gladiator and urban officer rigs
  both cook. needs: AS1.
- [ ] BX9 net: `Cut` on the wire and the tape — 16-byte event in the input log,
  interest managed, clients settle limbs within 5 cm in 10 ticks. Tier A. Size
  M. Owns hunks beside `crates/net/src/server.rs`. Accept: 64-client soak with 8
  cuts per second for 1 h, zero desync (ON4 pattern). needs: BX4, ON4.
- [ ] BX11 audio: material and tissue sound — blade on bone, flesh, wood,
  stone and armor; stump, limb landing, structure groan before collapse. Tier
  A. Size S. Owns hunks beside `crates/audio`. Accept: event-to-sound table
  covers every material pair; no sound without an event. needs: BX2, BX12, SN1.
- [ ] BX36 destruction: fracture event at 32 cells under 1 ms — `crates/destruction/src/fracture.rs` (fn `clip_cells`) and
  `crates/destruction/src/csg.rs` (fns `clip_back`, `split_back`). Tier B. Size S. Owns those two files only. Today (release, loaded box)
  16 cells p99 727 us, 24 cells 995 us, 32 cells 1.4 ms, 64 cells 2.7 ms. Idea: while clipping one cell, stop at the first neighbor whose
  bisector is farther than the cell's farthest vertex from its site (neighbors are already sorted nearest first), and reuse the plane list
  between cells. Accept: `cargo test --release -p engine-destruction --lib fracture_event_budget_one_millisecond -- --ignored --nocapture`
  prints `cells=32` with p99 at most 1000 us (change the test to assert the 32-cell row too); `cargo test -p engine-destruction --lib fracture`
  stays 7 green with the same digests (24-cell hit digest `e8370912b2c4e50a`, grid-2 cache digest `dde9247cb40b3c06`); golden unchanged. needs: DX5.
- [ ] BX39 endpoint: `wound.table.get` read verb — `crates/endpoint/src/packs/break_.rs` only (one `e2_tool!` block plus one `pack.add(VerbSpec::of::<..>(NAME,
  PACK, CapabilityCard::Observe)?)` line, copy `hit_simulate`). Tier B. Size S. Owns that file. Returns `engine_combat::limbloss::WoundTable::v1()` fields
  (`blood_full_q16`, `bleed_out_ticks`, per joint stump class and sever shock, thresholds) and `digest` as 16 hex chars. Accept: `cargo test -p
  engine-endpoint --features mcp --lib break_` green with a new test `wound_table_get_returns_v1_and_its_digest`: `bleed_out_ticks == [18000, 5400, 1200]`,
  digest equals `format!("{:016x}", WoundTable::v1().digest())`, the pack lists 4 verbs; `cargo test -p engine-endpoint --features mcp --lib` stays green
  (396 today); golden unchanged. needs: BX35.
- [ ] BX40 geo: one parry3d version — `crates/geo/Cargo.toml` line `parry3d = { version = "0.30.2", ...}` to `"0.31.1"`, then `cargo update -p parry3d
  --precise 0.31.1 --offline`. Tier B. Size S. Owns `crates/geo/Cargo.toml` and `Cargo.lock` (the parry3d entries). Rapier 0.36 (landed 2026-10-04) pulls parry3d
  0.31.1, so the lock holds two parry3d versions today. Accept: `cargo tree -d --manifest-path Cargo.toml` lists no `parry3d`; `cargo test -p engine-geo
  --features collision --lib` green; the V-HACD calls compile unchanged or the renamed parry3d 0.31 calls are fixed in `crates/geo/src`; golden unchanged.
- [ ] BX25 proof: Blender-bpy fracture column for BreakBench (G6 vision
  2026-09-29: only Blender `--background --python` +
  `bpy.ops.object.add_fracture_cell_objects` and Houdini HDA are
  headless-scriptable; Chaos editor-only, RayFire/Godot/unity-fracture
  runtime-only). Tier B. Size S. Owns the bpy fracture-column spike
  beside `crates/proof/src/break_corpus.rs` (fixture: 1 wall +
  Voronoi sites from scipy seed vs BX12 energy table). Accept:
  cell-count match + bake bytes x2; metric posted with kill rule.
  needs: BX12. (G6 Proposed, vision-r4 2026-09-29.)
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
- [ ] HS23 proof: Kani harnesses on the integer kernels — Q16 math, cut plane
  quantization, bond arithmetic, tape decode. Tier A. Size M. Owns
  `crates/proof/kani/`. Accept: 20 harnesses pass in CI; each names its kernel
  and bound; a found bug becomes a test row. needs: BX1.
- [ ] GEO1a dcc: Blender pin and version gate (center note `engine-steal-blender-2026-10-04.md` section 3, rules for B 1
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
  tools/workspace-check.mjs` PASS; golden `d57b5c8ae3096614` unchanged. needs: none. Stop: M 30 min.
- [ ] GEO1b dcc: recipe runner for Blender (second leg of the note's GEO1) — `tools/dcc/blender/run_recipe.py` (first
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
- [ ] GEO3b geo: UV pack quality metrics (center note GEO3 accept, the part GEO3 did not measure) — in
  `crates/geo/src/uv.rs` add `pub struct UvQuality { pub fill: f32, pub density_spread: f32, pub min_gap_texels: f32 }` and
  `pub fn quality(unwrap: &Unwrap, source: &GeoMesh) -> UvQuality`: `fill` = sum of UV triangle areas over atlas area;
  `density_spread` = (max - min) / mean of per-triangle (UV area x atlas texels) / (3D area) over triangles of area above
  1e-9; `min_gap_texels` = smallest distance between UV triangles of different charts times the atlas edge, sampled on
  the 128 x 128 grid `overlapping_chart_pairs` already uses. Tier A. Size S. Owns `crates/geo/src/uv.rs`,
  `crates/geo/tests/kit.rs` (one new test `kit_unwrap_quality`). Accept: `cargo test -p engine-geo --features all --test
  kit kit_unwrap_quality -- --nocapture` prints the three numbers per kit primitive (38 lines) and asserts on all 38: fill
  at least 0.55, density_spread at most 0.35, min_gap_texels at least 2.0; if a bound fails, tune `UnwrapOptions::default()`
  (padding, max_cost, brute_force) once, and if it still fails write the measured values into this row and BLOCKERS and
  stop; `cargo clippy -p engine-geo --all-targets --features all -- -D warnings` clean; golden unchanged. needs: none.
- [ ] GEO4b geo: LOD error against the engine chain (center note GEO4 accept, the part GEO4 did not measure) — in
  `crates/geo/src/lod.rs` add `pub fn sampled_error(original: &GeoMesh, reduced: &GeoMesh, samples: usize) -> f32`:
  deterministic points (`engine_core::Rng::new(2040)`, triangle chosen by area, barycentric draw) on `reduced`, each
  measured to the nearest point on `original` triangles by brute force, returns the largest distance divided by the
  bounding-box diagonal of `original`. Tier A. Size S. Owns `crates/geo/src/lod.rs`, `crates/geo/tests/kit.rs` (new test
  `kit_lod_error_vs_engine_chain`). Accept: `cargo test -p engine-geo --features all --test kit kit_lod_error_vs_engine_chain
  -- --nocapture` compares `simplify` at 50 percent with `engine_assets::mesh_cook::simplify(mesh, tris / 2)` on the 5 kit
  primitives `crate/0`, `walls/0`, `stands/0`, `torch/0`, `debris/0` (samples 2000 each): geo error at most the engine
  chain's on at least 4 of 5, all 10 numbers printed; unit test `sampled_error_of_a_mesh_against_itself_is_zero`; golden
  unchanged. needs: none.
- [ ] GEO5b geo: collision concavity metric (center note GEO5 accept, the part GEO5 did not measure) — in
  `crates/geo/src/collision.rs` add `pub fn concavity(mesh: &GeoMesh, parts: &[GeoMesh]) -> f32`: for every vertex of `mesh`
  the distance to the nearest part (0 when inside a part, by the signed volume test against each convex part's planes),
  maximum over vertices divided by the bounding-box diagonal of `mesh`. Tier A. Size S. Owns
  `crates/geo/src/collision.rs`, `crates/geo/tests/kit.rs` (new test `kit_collision_concavity`). Accept: `cargo test -p
  engine-geo --features all --test kit kit_collision_concavity -- --nocapture` prints one number per kit primitive (38) and
  asserts at most 0.02 on each; the unit test `l_prism_concavity_beats_single_hull` asserts parts of `l_prism(2.0, 1.0, 1.0)`
  score at most 0.02 and its `single_hull` at least 0.10; if a kit mesh fails, lower `HullOptions::default().concavity`
   once, then write the measured values into this row and stop; golden unchanged. needs: none.
- [ ] GEO5c geo: visibility-plane convex-decomposition bake beside V-HACD
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
  GEO14 variant folded.)
- [ ] GEO6a geo: humanoid bone auto-map (center note GEO6, Godot `SkeletonProfileHumanoid` name patterns as the shape, own
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
- [ ] GEO6b geo: automatic skin weights from bone segments (center note GEO6; a simple, exact base before any heat or
  geodesic method) — new `crates/geo/src/weights.rs` (std only) with `pub struct BoneSegment { pub head: [f32; 3], pub tail:
  [f32; 3] }` and `pub fn bind_weights(mesh: &GeoMesh, bones: &[BoneSegment]) -> Result<Vec<[(u16, f32); 4]>, GeoError>`: per
  vertex the distance d to each bone segment (closest point on the segment), keep the 4 nearest bones, weight w = 1 / (d +
  0.01 x mesh bounding-box diagonal)^2, normalize to sum 1 (ties by lower bone index), unused slots `(0, 0.0)`. Tier A. Size
  S. Owns `crates/geo/src/weights.rs`, one `pub mod weights;` line in `crates/geo/src/lib.rs`. Accept: `cargo test -p
  engine-geo --lib weights` — on `cylinder(0.3, 3.0, 24)` with 3 bones stacked along y: every vertex weight sum within 1e-6
  of 1, at most 4 non-zero influences, 0 unweighted vertices; the vertex nearest bone 1's middle has bone 1 weight at least
  0.9; a vertex at the joint of bone 1 and 2 has both weights between 0.4 and 0.6; result digest equal x2; empty `bones`
  returns `GeoError::Malformed`; clippy clean; golden unchanged. needs: none.
- [ ] GEO7a geo: object-space normal baker (center note GEO7, first leg) — new `crates/geo/src/bake.rs` behind feature `bake =
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
  this row and stop; clippy clean with `--features bake`; golden unchanged. needs: GEO3.
- [ ] GEO8a anim: clip edit verbs trim, retime, cleanup (center note GEO8, first leg; strips and mirror stay open) — new
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
- [ ] GEO8b anim: clip edit verbs strip and mirror (second leg of GEO8; GEO8a did trim, retime, cleanup). Tier A. Size S. Owns `crates/anim/src/edit.rs`. Accept: `cargo test -p engine-anim --lib edit` — `strip_keeps_used_keys`, `mirror_swaps_left_right`, `edits_repeat` (`qclip_hash` equal x2); `cargo test -p engine-anim` green; golden unchanged. needs: GEO8a.
- [ ] GEO9a geo: FBX import with ufbx (center note GEO9, first leg) — `crates/geo/Cargo.toml` feature `fbx = ["dep:ufbx"]`
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
- [ ] GEO10a assets: morph target import (center note GEO10, first leg; skin import keeps rejecting morph channels) — new
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
  golden unchanged. needs: none.
- [ ] GEO12 geo: native mesh ops weld, mirror, array, flip, normals (center note GEO2 second half) — new
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
- [ ] GEO13 mcp: door ops mirror, array, weld, flip in the geo pack (center note section 4) — in
  `crates/endpoint/src/packs/geo.rs` add to `GeoOp` the variants `Mirror { axis: String }` (`x`, `y`, `z`), `Array { count:
  u32, offset: [f32; 3] }`, `Weld { epsilon: f32 }`, `Flip`; every one is in `GeoOp::changes_topology`, runs through new
  functions in `crates/endpoint/src/packs/geo/kernel.rs` (the `geo` feature) that convert with `to_geo`/`from_geo` and call
  `engine_geo::ops`, refuses with `feature_off` in a build without the feature, and checks its numbers (`count` 1..=64 and
  at most `MAX_MODEL_TRIS` triangles after, `epsilon` in (0, 0.1], axis one of x y z) with a `bad_recipe` message that names
  the legal values. Tier A. Size S. Owns `crates/endpoint/src/packs/geo.rs`, `crates/endpoint/src/packs/geo/kernel.rs`.
  Accept: `cargo test -p engine-endpoint --features geo --lib packs::geo` and the same with `--features mcp` (the off-build
  refusal tests) green with these new tests: `array_then_mirror_numbers` (cube `[2,2,2]`, ops array count 2 offset `[3,0,0]`
  then mirror `x`: faces 48, vertices 32, bounds_min `[-4,-1,-1]`, bounds_max `[4,1,1]`), `weld_after_array_keeps_distinct_corners` (array count 3 offset `[3,0,0]` then weld 0.001: vertices 24), `flip_moves_the_digest_and_twice_restores_it`, `bad_numbers_name_the_legal_values` (count 0 and 65, epsilon 0 and 0.5, axis `w`), JSON parse of the 4 ops; clippy with
  `--features geo` clean; golden unchanged. needs: GEO12, GEO11.
