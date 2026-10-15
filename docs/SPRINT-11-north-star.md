### North star 2026 (S44) (owner 2026-10-03, `docs/plan/frontier.md`; S47: Bevy is the body, engine2040 the brain — pillars 3 and 5 name the Bevy cover per row)

Six pillars, 24 rows, gaps only — every row names the existing ids it
extends (verified by `node tools/plan-atlas.mjs --find` and board-slice at
write; NS prefix verified free). Each Accept: F2P test + P2P golden
`d57b5c8ae3096614` unchanged.

Physics (extends PX1, BX4, BX12, DX5, BX13):

- [ ] NS1 physics: articulated chains as one constraint graph — winch plus
  rope plus load (chariot trace, portcullis lift) with integer servo targets
  and canonical solve order. Size M. Owns hunks beside
  `crates/physics/src/powered.rs`. Accept: F2P `cargo test -p
  engine-physics --lib articulated` green (3-chain lift, same ticks x2
  shuffled); P2P golden `d57b5c8ae3096614` unchanged. needs: PX1.
- [ ] NS3 physics: ragdoll-vs-ragdoll clinch contacts — two clinching
  fighters separate without interpenetration under canonical (fighter, joint)
  order. Size M. Owns hunks beside `crates/physics/src/ragdoll.rs`. Accept:
  F2P clinch tape max penetration under 1 cm x2 shuffled; P2P golden
  unchanged. needs: PX1, AR2.
- [ ] NS4 proof: joint and contact conformance vs Jolt/Rapier oracles —
  offline oracles only (ideas reimplemented, no bytes), our servos and limits
  inside tolerance. Size S. Owns hunks beside `crates/physics/src/verify.rs`.
  Accept: F2P 50-pose oracle report green with max deviation posted; P2P
  golden unchanged. needs: PX1.

Animation (extends AN1, AM2, TV1, PX4, LOC2, LMM1, PX1):

- [ ] NS6 anim: full-body IK solve for carry and climb — spine plus limbs over
  TV1 edges with balance (PX4) as the veto. Size M. Owns hunks beside
  `crates/anim/src/fbik.rs`. Accept: F2P carry fixture: hands on load, no
  joint over its limit, x2; P2P golden unchanged. needs: TV1, PX4.
- [ ] NS7 anim: motion-matching style and terrain dials — LM1 integer search
  over style-tagged and slope-tagged clips from the LOC2 bake. Size M. Owns
  hunks beside `crates/anim/src/motion_match.rs`. Accept: F2P style query
  returns style clips 99% of 1,000 seeded searches, byte-equal x2; P2P golden
  unchanged. needs: LOC2, LMM1.
- [ ] NS8 anim: live motion tracker driving PX1 servos — games-style tracker
  from mocap clips to servo targets (LMM1 nets stay the offline path). Size
  L. Owns hunks beside `crates/physics/src/powered.rs`. Accept: F2P tracked
  walk follows the clip within 5 cm RMSE for 600 ticks x2; P2P golden
  unchanged. needs: PX1, LMM1.

Look — Bevy is the body here (extends RA14, VS5b, BX7, RT14):

- [ ] NS9 proof: look-rubric auto-score over captures — L1-L9 scoring harness
  beside `docs/VISUAL-TARGET.md` over RA14/VS9 frames (judge still opens the
  PNGs). Size S. Owns `tools/look-rubric.mjs`. Accept: F2P harness scores
  today's cube capture under 2/9 and refuses the cube SHA; P2P golden
  unchanged. Bevy covers: nothing (pixel scores, renderer-agnostic). needs:
  RA14.
- [ ] NS10 render: skin, eye and hair tier — subsurface skin, wet eye, strand
  hair on the hero at 1080p Base budgets. Size L. Owns hunks in
  `crates/render/src`. Accept: F2P hero close-up passes the look-rubric skin
  row beside the reference; P2P golden unchanged. Bevy covers: PBR skin/eye
  path at the S47 render-swap step; this row proves the look. needs: VS5b.
- [ ] NS11 render: BreakEvent to GPU particle feed — blood and dust emitters
  from event data with per-event budgets (the BX22 Tier-C look rides this
  feed). Size M. Owns hunks beside `crates/render/src/particles_gpu.rs`.
  Accept: F2P 250 events/s at 60 fps on the reference GPU, 0 CPU mesh builds;
  P2P golden unchanged. Bevy covers: bevy_hanabi 0.19 as the emitter (S47
  order); events and budgets stay ours. needs: RA4.
- [ ] NS12 render: probe-GI decision spike — one measured scene, probe vs
  baked cost and look delta, adopt or kill with numbers. Size S. Owns spike
  hunks beside `crates/render/src/gi.rs`. Accept: F2P report posts ms and
  rubric delta with a dated adopt/kill; P2P golden unchanged. Bevy covers:
  bevy_render/solari as the candidate path; the call stays ours. needs:
  RT14.

AI-native (extends EV1, MCP5, EA1e, LW5, HS21, CT4):

- [ ] NS13 endpoint: editor-plus-game verb conformance — the same 20
  authoring tasks through mock, local and remote with identical results
  (spatial-only SP35 parity stays as is). Size M. Owns hunks beside
  `crates/endpoint/src/mcp.rs`. Accept: F2P 20/20 identical across the three
  providers with the ledger diff empty; P2P golden unchanged. needs: EV1,
  MCP5.
- [ ] NS14 ai: LLM NPC brain with provider-off replay — recorded talk over a
  deterministic tick brain; provider off replays byte-equal. Size L. Owns
  hunks beside `crates/ai/src/mind.rs`. Accept: F2P Roman visit replays x2
  with the provider off, talk log identical; P2P golden unchanged. needs:
  EA1e, LW5.
- [ ] NS15 endpoint: refusal catalog growth loop — HS21's 20 adversarial
  briefs grow from every real refusal with a dated reason; false refusals
  stay 0. Size S. Owns hunks beside `crates/endpoint/src/tool_capabilities.rs`.
  Accept: F2P catalog grows 20 to 30 with 0 legitimate false refusals; P2P
  golden unchanged. needs: MCP8.
- [ ] NS16 playtest: brief-to-game bundle that fails a seeded bad build — one
  brief, one seeded-bad variant, the bundle FAILs loud on the bad one and
  PASSes on the good one. Size M. Owns hunks beside
  `crates/playtest/src/brief_eval.rs`. Accept: F2P bad build refused with a
  typed reason, good build passes, x2; P2P golden unchanged. needs: CT4,
  EV1.

Asset generation, intake only (extends CT40, CT28, LOC2, AS1):

- [ ] NS17 assets: texture and material intake — offline-painted albedo sets
  through the CT40 pattern with CT28 provenance and A2 cook budgets. Size M.
  Owns intake hunks beside `crates/assets/src/texture.rs`. Accept: F2P one
  intake texture cooks with mips x2 hashed; P2P golden unchanged. Bevy
  covers: asset-processor import of baked textures; cook and hash stay ours.
  needs: CT40, CT28.
- [ ] NS18 assets: sound intake for event-to-sound tables — offline-generated
  hits and groans land as banked PCM with provenance (AudioCraft code MIT,
  weights CC-BY-NC: weights never in repo). Size S. Owns intake hunks beside
  `crates/audio/src/bank.rs`. Accept: F2P one intake sound plays from an
  event with its provenance row; P2P golden unchanged. Bevy covers: nothing.
  needs: CT40.
- [ ] NS19 anim: motion intake as BVH through the CT4 loop — offline
  text-to-motion outputs land as clips readable by LOC2 (donor repos
  MIT-class only, LICENSE verified at intake). Size M. Owns intake hunks
  beside `crates/anim/src/clip.rs`. Accept: F2P one intake clip retargets and
  plays x2 hashed; P2P golden unchanged. Bevy covers: bevy_gltf skin-order
  patterns at import only. needs: CT40, LOC2.
- [ ] NS20 assets: second mesh source plus content-addressed cache — Hunyuan3D
  and Stable-Fast-3D both under community licenses (offline intake only),
  outputs hashed into the cache with CT28 provenance. Size M. Owns cache
  hunks beside `crates/assets/src/cooked.rs`. Accept: F2P same prompt bytes
  hit the cache with one hash, license_gate refuses unreadable terms; P2P
  golden unchanged. Bevy covers: asset processor as the cache consumer;
  provenance, cook and hash stay ours. needs: CT40, CT28, AS1.

Crowds (extends CT64, AU2, NV2, CT25, GA2, AM18, PX16):

- [ ] NS21 ai: reacting-crowd behavior model as data — wave, hush-on-kill,
  groan-on-foul driving the AU2 layers from CT64 staging. Size M. Owns hunks
  beside `crates/audio/src/crowd.rs`. Accept: F2P kill event hushes the bowl
  within 30 ticks x2; P2P golden unchanged. needs: CT64, AU2.
- [ ] NS22 nav: vomitoria ingress/egress flow proof — 20k spectators enter
  and leave through the gates on flow fields without deadlock. Size M. Owns
  hunks beside `crates/nav/src/flow.rs`. Accept: F2P full-bowl empty-out tape
  replays x2, 0 stuck agents; P2P golden unchanged. needs: NV2, CT25.
- [ ] NS23 anim: formation locomotion LOD at legion scale — GA2 shield-wall
  movement under AM18 tiers, nearest-8 equality across tiers. Size M. Owns
  hunks beside `crates/anim/src/locomotion.rs`. Accept: F2P 1,000-agent march
  holds formation at far LOD x2; P2P golden unchanged. needs: GA2, AM18.
- [ ] NS24 proof: crowd determinism harness — 20k agents, one hash, two PCs.
  Size S. Owns hunks beside `crates/proof`. Accept: F2P 20k-agent 600-tick
  tape hashes x2; P2P golden unchanged. needs: PX16.

Rules (extends RC5, decision D2 in `FOCUS.md`):

- [ ] NS32c rules: quiet-box p99 for the arena adapter — new ignored test `arena_drive_p99` in
  `crates/rules/tests/arena_budget.rs` (feature `arena`): 16 fighters, all scripted with
  `shield_bash_arena.rhai`, 600 ticks, time `RulesController::control` per tick. Size S.
  Accept: F2P `cargo test --release -p engine-rules --features arena --test arena_budget --
  --ignored` p99 at most 0.2 ms per tick on a quiet box (loops off or heavy queue idle); numbers
  in the row. needs: NS32a.
- [ ] NS32d rules: second-PC replay of the scripted match — new test `arena_drive_hashes` in
  `crates/rules/tests/arena_drive.rs` plus `crates/rules/tests/data/arena_drive_hashes.txt`
  (seed 2040 and 7, 600 ticks, 2 fighters, half 12 m, slot 0 scripted: tick 599 hash per line;
  written once by an `#[ignore]` test `bless_arena_hashes`). Size S. Accept: F2P `cargo test -p
  engine-rules --features arena --test arena_drive arena_drive_hashes` equal on this PC and on a
  second PC; floats enter the arena hash, so a mismatch is a finding to write into
  `docs/BLOCKERS.md`, not a test to loosen. needs: NS32a.
- [ ] NS32e rules: scripted slot on a walled scene through the door — new test
  `rules_drive_on_a_walled_scene` in `crates/endpoint/tests/rules_door.rs` (copy `insert_wall`
  from `tests/scene_drives_match.rs`): `rules.set`, one 2 m wall, `play.run` with `scene_rev`.
  Size S. Accept: F2P `cargo test -p engine-endpoint --features rules --test rules_door
  rules_drive_on_a_walled_scene` the scripted walled run differs from the plain walled run, equal
  x2, no fighter inside the wall in 18 of 20 seeds. needs: NS32b.

Make (extends HS26, decision D3 in `FOCUS.md`): generated props the AI can ask for.

- [ ] GN4 make: texture spike, license first (MV-Adapter, `huanngzh/MV-Adapter`, Apache-2.0, commit
  `4277e0018232bac82bb2c103caf0893cedb711be`, `python -m scripts.texture_i2tex ... --variant sd21`
  writes `<save_dir>/<save_name>_shaded.glb`). TRAP read 2026-10-05: its texture path imports
  `nvdiffrast.torch` (NVIDIA licence, not commercial for us) in `mvadapter/utils/mesh_utils/{mesh,render,utils,warp}.py`
  with 7 calls (`RasterizeCudaContext`, `RasterizeGLContext`, `rasterize` x3, `interpolate`, `antialias`,
  `texture`), plus `pymeshlab` (GPL-3.0) and `open3d` in `mesh_process.py`, `cvcuda` in `cv_ops.py`, and an
  `RMBGModel` (BRIA, non-commercial) in `seg.py`. Size S. Owns center `kaggle.mjs` and `kaggle.test.mjs`
  (edit only these two; commit by path). Steps: (1) in `kaggle.mjs` add `"nvdiffrast"`,
  `"NVlabs/nvdiffrast"` to `MESH_EXCLUDED`; (2) write a stub-and-swap spike as a Python string
  `TEXTURE_SPIKE` that tries `facebookresearch/DRTK` (MIT) behind a module named `nvdiffrast.torch` for
  the 7 calls and runs one texture on `target/make-run/mesh/*.glb` with `--variant sd21`; (3) run it with
  `node kaggle.mjs run mesh ...` style kernel by hand (one job at a time, token stays in the env). KILL: if
  more than 7 call sites need new code, or `pymeshlab`/`open3d` cannot be replaced by `trimesh` plus
  `fast-simplification`, write the reason under `docs/BLOCKERS.md` and stop; no `texture` job kind lands.
  Accept: F2P either a `texture` kind exists with `kaggle.test.mjs` asserting no `nvdiffrast`, `pymeshlab`,
  `rembg`, `briaai` in any `pip(` or `from_pretrained` line (same shape as the test named
  "mesh: licenses read from the model cards"), and one real run textures both GN3 props with a
  non-empty base-colour map each (PNG over 100 KB); or a BLOCKERS entry naming the call sites that fail.
  Run: `node --test kaggle.test.mjs` in the center folder, then `node kaggle.mjs status` (engine2040's
  share is 200 GPU min a week; GN3 used about 8).
  needs: GN3.
- [ ] GN5 make: `parts` job kind for pre-broken props (PartCrafter, `wgsxm/PartCrafter`, MIT on the card
  and LICENSE, commit `3d773bf02fad51c7ab31a5615573fec93b287b30`; `src/pipelines/pipeline_partcrafter.py`
  uses scikit-image marching cubes, no `diso`). Same Tencent condition string as TripoSG because its
  transformer file `src/models/transformers/partcrafter_transformer.py` is adapted from TripoSG/HunyuanDiT.
  TRAPS: `scripts/inference_partcrafter.py` imports `src.models.briarmbg.BriaRMBG` and downloads
  `briaai/RMBG-1.4` (non-commercial): do not run that script, import `PartCrafterPipeline` from
  `src.pipelines.pipeline_partcrafter` and cut the picture out with BiRefNet exactly as `job_mesh` does
  (functions `birefnet_masks`, `frame_for_triposg`); `--part_suggest` and `--style_transfer` call Gemini (an
  API key, not free): never use them, take `--n-parts` from the caller. Size S. Owns center `kaggle.mjs`
  and `kaggle.test.mjs`. Steps: add `JOBS` entry `parts`, `MODELS["partcrafter"]` (`hf: "wgsxm/PartCrafter"`,
  `code_commit` above, `conditions` copied from `MODELS.triposg`), `JOB_MODELS.parts = ["birefnet",
  "partcrafter", "dinov2-large"]`, `JOB_COST.parts = {load: 14, per: 3}`, flag `--parts N` (2 to 8, in
  `VALUE_FLAGS` and `buildSpec`, default 4), Python `job_parts()` beside `job_mesh()` writing
  `parts-NN-<stem>/part_00.glb ... part_NN.glb` plus `object.glb`, `JOBS` dict entry. Accept: F2P
  `node --test kaggle.test.mjs` adds 3 tests (spec bounds for `--parts`, licence lines as for GN4, a fake-CLI
  run copies every part file listed in the manifest `extra` array) and passes; one real run on
  `target/make-run/images/image-01-*.png` with `--parts 4` returns 4 part GLBs, each a closed-enough mesh
  (manifest `faces` > 500), GPU minutes under 15. needs: GN3.
- [ ] GN6 assets: intake of a part set into destruction chunks — new function `intake_parts` in
  `crates/assets/src/gen_intake.rs` (same file, new tests in `crates/assets/src/gen_intake/tests.rs`):
  input is N part GLB byte slices plus one `Provenance`; each part runs the existing steps (`weld`,
  `clean_tris`, `drop_specks`, `compact`), all parts share ONE pose transform computed from the union
  bounding box (so they still fit together), each part simplifies to at most `max_tris / N` triangles, and
  the bundle holds one mesh named `FRAC_<name>_<ii>` per part (the existing `mesh_cook::is_frac_name`
  and `validate_frac_name` define the legal names, `MAX_FRAC_CHUNKS` the cap). Size S. Owns
  `gen_intake.rs` and its tests only. Run: `cargo test -p engine-assets --features import,registry
  gen_intake` through the heavy queue (command in the wave file, rule 4). Accept: F2P a 4-part test
  fixture (4 icospheres side by side, built like `icosphere` in the tests) cooks to a bundle of 4 `FRAC_`
  meshes whose summed triangles are at most 5,000, union hull contains every part, cook x2 byte-identical,
  one cache hit returns the same bundle hash, a refused model writes nothing; P2P golden
  `d57b5c8ae3096614` unchanged. needs: GN1.
- [ ] GN7 endpoint: `asset.generate` becomes a runner that starts the center job without blocking —
  new file `crates/endpoint/src/packs/asset/generate_run.rs` (feature `intake`), `GenerateTool` in
  `packs/asset.rs` keeps its name. Args `{image_path, name, n?, faces?}` start
  `node <center>/kaggle.mjs run mesh --repo engine2040 --in <image_path> --out <out_dir>/<name> --detach`
  (center folder from env `E2040_CENTER_DIR`; refuse with code `no_center` and one next step when unset)
  and return `{run_id}` parsed from the line `started run <id>`; args `{run_id}` run `kaggle.mjs job <id>`
  and return `{state, files}`; when `state` is done the verb calls `asset.intake` on every returned GLB
  with `lane_json` set. Before starting, `IntakeCache::lookup` is asked with the same generation inputs and
  a hit returns the cached bundle hash with no GPU job. Size S. Owns `generate_run.rs` and
  `packs/asset.rs` hunks. Accept: F2P `cargo test -p engine-endpoint --features intake asset` has 3 new
  tests using a fake `kaggle.mjs` (a node script written to a temp folder that prints the two lines above
  and writes a GLB plus `kaggle-lane.json`): start returns a run id, poll returns done and an intake
  bundle, a second start with identical inputs returns `cache_hit: true` and the fake script is not run
  again; unset `E2040_CENTER_DIR` is refused with `no_center`; the default build (no `intake`) still
  lists `asset.generate` as the explainer. needs: GN2.
- [ ] GN8 make: licence recheck for the generator chain — new center command `node kaggle.mjs models
  --verify` in `kaggle.mjs` (function `verifyModels`, dependency-free `fetch` of
  `https://huggingface.co/api/models/<hf>` for every `MODELS` entry with an `hf` field) that compares
  the card's `cardData.license` to the table and prints one line per model; a mismatch or a gated or
  missing model exits 1. Size S. Owns center `kaggle.mjs` and `kaggle.test.mjs`. Accept: F2P
  `node --test kaggle.test.mjs` has 2 new tests with an injected fetch (all equal exits 0; one card
  changed to `cc-by-nc-4.0` exits 1 and names the model); a real `node kaggle.mjs models --verify` prints
  `triposg MIT ok`, `birefnet MIT ok`, `dinov2-large apache-2.0 ok`, `flux-schnell apache-2.0 ok`
  (HF lowercases ids and some licences; compare case-insensitively). It also refuses, whatever the
  table says, a card whose licence is non-commercial or restricted (research 2026-10-12, AI art scout
  F7): `cc-by-nc-*`, `cc-by-nc-sa-*`, `tencent-hunyuan-community` (excludes EU, UK, South Korea),
  `coqui-public-model-license`, `openrail++` with a research-only card, `stability-ai-community`
  unless the revenue flag is set, or no licence tag; TripoSG's HF tag says MIT while our note says a
  Tencent condition string, so read its LICENSE file before any TripoSG asset ships. needs: GN3.
