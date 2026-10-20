# Proposed rows — lane N2 (auto making), 2026-10-20

From `12-auto-making-content.md` (same folder). One row per find. Ids are `AMKX-` so the lead can
merge them into `docs/SPRINT*.md` without a clash. No board file, plan file or focus file was
touched. Nothing was committed, staged or pushed in either repo.

Evidence grades: OBSERVED (read today, 2026-10-20), DERIVED (reasoned from what I read), INFERRED
(likely, not proven), UNKNOWN. Every donor licence was read live today; the board text was not
trusted.

Two rows are already landed as code by this lane (`AMKX-02`, `AMKX-08`); the rest are proposals or
patch-ready specs. Owned paths marked HELD belong to another lane and are not touched.

---

### AMKX-01 — ORPHAN: the generation lane has no runner file any more

- **what it is:** The center file every generator row names is gone. `C:\empire\center\kaggle.mjs`
  does not exist on disk and is not in the git history of `C:\empire` (OBSERVED: file search over
  `C:\empire` and `git log --all -- center/kaggle.mjs` return nothing). The board says so itself:
  `docs/SPRINT-10-empire-inbox-drain.md:232` "Landed obsolete. Owner chat 2026-10-19 (delegated):
  the target or premise is gone — target `kaggle.mjs` (center mesh step) is gone; the center
  pipeline was retired by the standalone-repo decision 2026-10-17."
- **donor:** no donor. The gap is ours: `crates/endpoint/src/packs/asset.rs:32-33` still tells the
  agent to run `node kaggle.mjs run mesh --repo engine2040 --in <image> --out <folder>`, and
  `crates/endpoint/src/packs/asset.rs:74-79` returns `runs_here: false`, so the door names a
  command that cannot be run. Owner text: `C:\empire\center\accounts.json:752` still calls the
  center "Center's free GPU lane (kaggle.mjs, one job at a time)".
- **closes:** GN4, GN5, GN7, GN8 (they all name the missing file).
- **owned paths:** `C:\empire\center\make.mjs` (new), `C:\empire\center\AGENTS.md` (read only),
  `docs/SPRINT-11-north-star.md` (the four rows, via the lead).
- **accept:** `node C:\empire\center\make.mjs selftest` prints
  `make selftest: 11/11 checks ok` (exit 0), and `node C:\empire\center\make.mjs models --verify`
  prints one line per model and exits 1 while any model is refused.
- **kill line:** if the owner does not want a generation lane in the center at all, this row dies
  and GN4, GN5, GN7, GN8 are re-homed to a runner inside `engine2040` (a crate the lead owns).
- **cost:** S.
- **step:** 2 (the runner). The new home must be: one file in the center that (a) starts a job
  and returns a run id, (b) polls a job and reports state and files, (c) refuses any model whose
  card licence is not commercial use. The owner of the file is the center seat (it is the free GPU
  lane); the engine2040 side of the contract is the endpoint pack (HELD, `AMKX-02`).

### AMKX-02 — The runner and the licence gate land in one center file

- **what it is:** One new file, `C:\empire\center\make.mjs`, with four commands: `models --verify`
  (reads every model card and fails closed), `run <kind> --spec <file>` (starts a job, prints
  `started run <id>`), `job <id>` (prints the state and the files) and `selftest` (no network).
  The run id is the hash of the job spec, so the same spec always names the same run.
- **donor:** the shape of a free GPU lane, taken from the empire's own working example:
  `C:\empire\lab\research\research\cards\kimodo-trial-2026-10-20\KAGGLE.md` and
  `kaggle\encode_kernel.py` (start a kernel, poll it, fetch the output; OBSERVED 2026-10-20), plus
  the queue shape of `C:\empire\center\heavy.mjs` (read 2026-10-20). The Kaggle CLI itself is
  Apache-2.0: https://github.com/Kaggle/kaggle-cli, `LICENSE.txt` (read 2026-10-20). No donor
  bytes are copied.
- **closes:** GN7 (the runner half), GN8 (the gate half).
- **owned paths:** `C:\empire\center\make.mjs` only. Nothing else in the center changed.
- **accept:** `node make.mjs selftest` prints `make selftest: 11/11 checks ok` (exit 0);
  `node make.mjs models --verify` prints `VAST-AI/TripoSG mit ok`, `stabilityai/TripoSR mit ok`,
  `wgsxm/PartCrafter mit REFUSED ...`, `stabilityai/stable-fast-3d other REFUSED the card is gated`,
  `tencent/Hunyuan3D-2 other REFUSED licence other is not commercial use`, `models 3 refused`
  (exit 1); `node make.mjs run mesh --spec <file>` prints `started run run-bfd790cedddd`.
- **kill line:** if the center rules ever forbid a generation lane there, this file moves to
  `engine2040` as `tools/make.mjs` with the same four commands; the contract does not change.
- **cost:** S.
- **step:** 2 and 1 (the runner and the gate).
- **engine side (HELD, `crates/endpoint`):** `crates/endpoint/src/packs/asset/generate_run.rs`
  (the file GN7 names) starts `node <center>/make.mjs run <kind> --spec <file>`, parses the
  `started run <id>` line, and polls `node <center>/make.mjs job <id> --json` for
  `{state, files}`; when the state is done it calls `asset.intake` on every returned GLB. The
  center folder comes from `E2040_CENTER_DIR`; unset is refused with the code `no_center`.

### AMKX-03 — The licence gate reads the model card, not the caller's word

- **what it is:** The engine's gate today accepts the SPDX string the caller typed
  (`crates/assets/src/provenance.rs:19-32` is the allow-list, `:258 check_record` checks the
  record). Nothing compares that string with the model card, so a caller can call a
  non-commercial model MIT. The gate must read the card and fail closed.
- **donor:** the card itself: https://huggingface.co/api/models/<id> gives `cardData.license` and
  `gated` (read live 2026-10-20 on ten models). The empire already ships this shape in its own
  vault: `C:\empire\studio\asset-vault\tools\provenance.py`, `tools\vault-license-gate.mjs`,
  `tools\gltf-gate.mjs`, `tools\stamp-manifest.mjs` (read 2026-10-20; our own code, no donor
  bytes). The engine side to copy: `crates/assets/src/provenance.rs:19-32`, `:258`, `:355`.
- **closes:** GN8, and it is the gate NS17 and NS20 lean on.
- **owned paths:** HELD `crates/assets/src/provenance.rs` (WEB6). The centre half is landed in
  `AMKX-02`.
- **accept:** `node make.mjs models --verify` exits 1 and names every refused model; the engine
  side must add `crates/assets/src/provenance.rs::check_model_card(hf_id, expected_spdx)` with a
  test that a card saying `cc-by-nc-4.0` refuses with `license_refused`.
- **kill line:** a gated card must refuse by default (`stabilityai/stable-fast-3d` answered HTTP
  401 without a token today); if any gated or untagged model can pass, the row dies.
- **cost:** S.
- **step:** 1.

### AMKX-04 — LICENCE RULING: PartCrafter's transformer file carries the Tencent terms

- **what it is:** `wgsxm/PartCrafter` has an MIT LICENSE and an MIT model card, but one file in it
  is adapted from Tencent HunyuanDiT under the Tencent Hunyuan Community License, which does not
  apply in the EU. A repo-wide MIT read is not enough: the gate must read the file that is copied.
- **donor:** https://raw.githubusercontent.com/wgsxm/PartCrafter/main/LICENSE (MIT, read 2026-10-20)
  and https://huggingface.co/api/models/wgsxm/PartCrafter (`license: mit`, not gated, read
  2026-10-20), against
  `https://raw.githubusercontent.com/wgsxm/PartCrafter/main/src/models/transformers/partcrafter_transformer.py`
  lines 3-14 (read 2026-10-20): "This code is based on TripoSG ... which is based on Tencent
  HunyuanDiT ... licensed under the Tencent Hunyuan Community License Agreement ... THIS LICENSE
  AGREEMENT DOES NOT APPLY IN THE EUROPEAN UNION". TripoSG itself is MIT:
  https://raw.githubusercontent.com/VAST-AI-Research/TripoSG/main/LICENSE and
  https://huggingface.co/api/models/VAST-AI/TripoSG (`mit`, read 2026-10-20).
- **closes:** GN5 (the `parts` job), GN8 (the gate). The ruling is what the gate must refuse.
- **owned paths:** the centre table in `C:\empire\center\make.mjs` (landed: PartCrafter is
  refused with the reason) and the engine gate (HELD).
- **accept:** `node make.mjs models --verify` prints
  `wgsxm/PartCrafter mit REFUSED card says mit but the transformer file carries the Tencent
  Hunyuan Community terms (no EU): research only` and exits 1.
- **kill line:** the gate must refuse PartCrafter as a default generator. Whether a mesh produced
  by those weights is a Tencent "Output" that may ship is a legal reading (INFERRED, not proven),
  so it is an owner decision: the inbox item must name it.
- **cost:** S.
- **step:** 3.

### AMKX-05 — LICENCE RULING: Hunyuan3D-2 excludes the EU, the UK and South Korea

- **what it is:** The second mesh source NS20 names cannot be a shipping generator. Its licence
  says so in the first line.
- **donor:** https://raw.githubusercontent.com/Tencent-Hunyuan/Hunyuan3D-2/main/LICENSE (read
  2026-10-20): "TENCENT HUNYUAN 3D 2.0 COMMUNITY LICENSE AGREEMENT ... THIS LICENSE AGREEMENT
  DOES NOT APPLY IN THE EUROPEAN UNION, UNITED KINGDOM AND SOUTH KOREA AND IS EXPRESSLY LIMITED
  TO THE TERRORY". Card: https://huggingface.co/api/models/tencent/Hunyuan3D-2 (`license: other`,
  read 2026-10-20).
- **closes:** NS20 (the gate must refuse it), GN8.
- **owned paths:** the centre table (landed: refused) and the engine gate (HELD).
- **accept:** `node make.mjs models --verify` prints `tencent/Hunyuan3D-2 other REFUSED licence
  other is not commercial use` and exits 1.
- **kill line:** research only. It never becomes a default generator and never enters `content/`.
- **cost:** S.
- **step:** 7.

### AMKX-06 — LICENCE RULING: stable-fast-3d is gated and its licence reads "other"

- **what it is:** The other second mesh source. The card is gated (HTTP 401 without a token) and
  the licence is not a plain SPDX id.
- **donor:** https://api.github.com/repos/Stability-AI/stable-fast-3d (`license.spdx_id:
  NOASSERTION`, read 2026-10-20) and https://huggingface.co/api/models/stabilityai/stable-fast-3d
  (`license: other`, `gated: "auto"`, read 2026-10-20); the card README answers 401 without a
  token.
- **closes:** NS20 (the gate must refuse it), GN8.
- **owned paths:** the centre table (landed: refused) and the engine gate (HELD).
- **accept:** `node make.mjs models --verify` prints `stabilityai/stable-fast-3d other REFUSED the
  card is gated` and exits 1.
- **kill line:** research only, and a gated card refuses by default even when the owner has an
  account.
- **cost:** S.
- **step:** 7.

### AMKX-07 — The `parts` job kind: a prop that breaks before it breaks

- **what it is:** A generator job that writes N part meshes for one prop, so the destruction
  system has real chunks to break. The job writes `parts-NN-<stem>/part_00.glb ... part_NN.glb`
  plus `object.glb`.
- **donor:** `wgsxm/PartCrafter`, MIT repo LICENSE
  (https://raw.githubusercontent.com/wgsxm/PartCrafter/main/LICENSE) with the file-level Tencent
  terms of `AMKX-04`; the pipeline file
  `https://raw.githubusercontent.com/wgsxm/PartCrafter/main/src/pipelines/pipeline_partcrafter.py`
  (the part set, read by the board row, not re-read by me today, so treat the file as UNKNOWN
  until a builder reads it). The paper: arXiv 2506.05573.
- **closes:** GN5.
- **owned paths:** HELD `C:\empire\center\make.mjs` job kinds (the runner is landed; the `parts`
  kernel itself needs the Kaggle account the owner holds), plus the engine door (HELD).
- **accept:** one real run with `--parts 4` returns four part GLBs, each with more than 500
  faces, and the licence gate still refuses the model (the parts are research-only until the
  owner decides), so the proof is "the job runs and the gate refuses" — not "the props ship".
- **kill line:** if the parts cannot be produced without the non-commercial chain, the job kind
  never lands and GN5 closes as rejected.
- **cost:** S.
- **step:** 3.

### AMKX-08 — The part-set geometry lands in crates/geo (LANDED)

- **what it is:** The geometry half of GN6: one union box over the part set, one shared
  translation every part gets, and one triangle budget per part. It is pure cook-time geometry,
  so it belongs beside the other mesh tools (decision D4), not inside the asset intake.
- **donor:** no donor code. The shape comes from the GN6 row text and from
  `crates/geo/src/ops.rs` (the crate's own native mesh ops: weld, mirror, array, flip, normals).
  Licence: our own code.
- **closes:** GN6 (the geometry half; the intake half is `AMKX-09`).
- **owned paths:** `crates/geo/src/parts.rs` (new), `crates/geo/src/lib.rs` (one `pub mod parts;`
  line, one root-path line in the wiring test, one line in the crate header).
- **accept:** `node tools/gh-check.mjs cargo test -p engine-geo --lib parts` prints
  `test result: ok. 6 passed; 0 failed; 0 ignored; 33 filtered out` (run 39497089479) and
  `node tools/gh-check.mjs cargo clippy -p engine-geo --all-targets -- -D warnings` prints
  `PASS on the runner` (run 39496477977).
- **kill line:** if the pose cannot be one translation for the whole set, or if the budget split
  can exceed the cap, the row dies. The tests hold both.
- **cost:** S.
- **step:** 4.

### AMKX-09 — PATCH-READY: `intake_parts` in crates/assets (HELD)

- **what it is:** The cook half of GN6: N part GLBs plus one `Provenance` become one bundle with
  one `FRAC_<name>_<ii>` mesh per part, each part simplified to at most `max_tris / N` triangles,
  all parts sharing the one pose that `AMKX-08` computes.
- **donor:** our own code. `crates/assets/src/gen_intake.rs` (the existing intake, the cache and
  the provenance row) and `crates/assets/src/mesh_cook.rs:979 is_frac_name`, `:993
  validate_frac_name`, `:58 MAX_FRAC_CHUNKS` (the legal chunk names).
- **closes:** GN6 (the intake half).
- **owned paths:** HELD `crates/assets/src/gen_intake.rs` (WEB6). Nothing was written there.
- **accept:** `cargo test -p engine-assets --features import,registry gen_intake` prints a new test
  `four_parts_cook_to_frac_chunks_that_fit_together` green, with a 4-part fixture (four
  icospheres side by side) whose summed triangles are at most 5,000, whose union hull contains
  every part, cook x2 byte-identical, one cache hit returning the same bundle hash, and a refused
  model writing nothing. The golden `d57b5c8ae3096614` must not move.
- **kill line:** if the parts cannot share one pose and still fit, the row dies.
- **cost:** S.
- **step:** 4.

### AMKX-10 — The texture bake with no NVIDIA and no BRIA in the chain

- **what it is:** A deterministic texture bake: a seeded graph of noise and material nodes baked
  to a base-colour map (plus normal and ORM when asked), then through the existing texture cook.
  It replaces the MV-Adapter spike, whose texture path pulls in the NVIDIA licence.
- **donor:** the node-graph shape of `RodZill4/material-maker` (MIT,
  https://raw.githubusercontent.com/RodZill4/material-maker/master/LICENSE.md, branch master, read
  2026-10-20). The "text becomes a material" papers are ideas only: arXiv 2407.06489 (T2MAT) and
  arXiv 2509.09952 (Chord, PBR from a generated texture image). REJECTED: `huanngzh/MV-Adapter`
  (Apache-2.0, https://raw.githubusercontent.com/huanngzh/MV-Adapter/main/LICENSE, read 2026-10-20)
  whose texture path imports `nvdiffrast.torch` (NVIDIA), plus `pymeshlab` (GPL-3.0) and
  `briaai/RMBG-1.4` (non-commercial).
- **closes:** GN4, and it feeds NS17.
- **owned paths:** the bake beside `crates/assets/src/texture.rs` (HELD, WEB6) or a new
  `crates/geo` module (this lane) if the bake is pure geometry. FU6 already names the home:
  "spike hunks beside `crates/assets/src/texture.rs`".
- **accept:** `cargo test --locked -p engine-assets matgen` prints a 256x256 bake green with bytes
  and ms posted, cook x2 identical, golden unchanged.
- **kill line:** if a deterministic bake cannot produce a base-colour PNG over 100 KB with bytes
  identical x2 inside the A2 cook bounds, write the BLOCKERS row and stop; no AI texture path
  lands.
- **cost:** M.
- **step:** 5.

### AMKX-11 — The texture and material intake (NS17) on the A2 cook

- **what it is:** The intake that takes a baked texture set (albedo, normal, ORM), cooks it with
  mips and the sRGB rules, writes the provenance row and refuses a set whose licence is not
  commercial use.
- **donor:** our own code: `crates/assets/src/texture.rs` (the cook, with `MAX_TEXTURE_BYTES`,
  `MAX_TEXTURE_DIM`, `MAX_MIPS`), `crates/assets/src/ktx2_cook.rs` (the KTX2 cook that VR2
  landed) and `crates/assets/src/provenance.rs:355 provenance_row`. The CC0 texture path that
  already works: `content/arena/sources/PROVENANCE.md` and the five ambientCG sets (OBSERVED).
- **closes:** NS17, and A2 (A2 is archived and done: "texture cook with mips and sRGB rules", so
  the intake must use A2's cook bounds, not a second cook).
- **owned paths:** HELD `crates/assets/src/texture.rs` (WEB6).
- **accept:** `cargo test -p engine-assets --lib texture` prints one new test
  `an_intake_texture_cooks_with_mips_x2_hashed` green, and the golden `d57b5c8ae3096614` does not
  move.
- **kill line:** if the intake needs a second cook path, it is refused: A2 owns the cook.
- **cost:** M.
- **step:** 6.

### AMKX-12 — A second mesh source whose weights we may ship, hashed by the prompt bytes

- **what it is:** NS20 asks for a second mesh source and a content-addressed cache. The cache is
  already ours (`crates/assets/src/gen_intake.rs:589 cache_key`, `:796 IntakeCache::lookup`), but
  the key hashes the input GLB, not the prompt, so the same prompt cannot hit the cache before the
  GPU run. The source must be a model whose card is plain MIT.
- **donor:** `openai/shap-e` (MIT code and MIT card, not gated,
  https://huggingface.co/api/models/openai/shap-e, read 2026-10-20) and
  `VAST-AI-Research/TripoSR` (MIT, https://raw.githubusercontent.com/VAST-AI-Research/TripoSR/main/LICENSE,
  card `mit` at https://huggingface.co/api/models/VAST-AI/TripoSG, read 2026-10-20). Refused:
  Hunyuan3D-2 and stable-fast-3d (`AMKX-05`, `AMLX-06`).
- **closes:** NS20.
- **owned paths:** HELD `crates/assets/src/cooked.rs` and `crates/assets/src/gen_intake.rs` (WEB6).
- **accept:** `cargo test -p engine-assets --lib gen_intake` prints a new test
  `the_same_prompt_bytes_hit_the_cache_with_one_hash` green, and `license_gate` refuses a card
  whose terms cannot be read.
- **kill line:** a source whose card licence is not on `crates/assets/src/provenance.rs:19-32`
  stays out of `content/`; the gate is the kill line, not a person.
- **cost:** M.
- **step:** 7.

### AMKX-13 — WD14 correction: what the node-constraint collapse still owes

- **what it is:** The row text lies about the shape of the work. The tree has a WFC tile bake, but
  not the solver WD14 asks for. What exists: `crates/world/src/wfc.rs:43 Adjacency` (per-direction
  neighbour masks), `:192 bake(width, height, adjacency, seed)`, `:283 next_cell` (fewest-kinds
  pick), `:344 propagate` (arc consistency), `:229 is_connected`. What is missing: (a) authored node
  restrictions (wall, rubble, gate, door tiles) — `bake` takes no such input, (b) room-adjacency
  graphs, (c) a fixed backtrack cap — the doc at `:190-191` says "A contradiction is refused,
  never retried" and `WfcError::Contradiction` at `:141` ends the run, (d) the proof: 20 seeds
  solving 18 or more within the cap at p50 100 ms or less. The existing tests
  (`:446 bake_is_deterministic_and_seed_sensitive`, `:458 baked_neighbours_agree_with_the_table`,
  `:482 impossible_table_dies_in_bake_not_in_new`, `:501 new_refuses_bad_tables`, `:534
  bake_refuses_a_side_above_the_cap`, `:554 is_connected_finds_walls_and_open_ground`) cover the
  tile bake, not the node-constraint solver.
- **donor:** `BorisTheBrave/DeBroglie` (MIT, read 2026-10-20) for the AC4 support counts and the
  non-local constraints, and `mxgmn/WaveFunctionCollapse` (MIT,
  https://raw.githubusercontent.com/mxgmn/WaveFunctionCollapse/master/LICENSE, read 2026-10-20) for
  the propagate-and-collapse shape. `krychu/wfc` has no LICENSE file at its master root today
  (raw fetch 404), so it stays ideas only.
- **closes:** WD14 (the board row needs correcting, not re-implementing).
- **owned paths:** `crates/world/src/wfc.rs` (the row names `crates/world/src/gen/wfc.rs`, a path
  that does not exist).
- **accept:** `node tools/gh-check.mjs cargo test -p engine-world --lib wfc` prints a new test
  `twenty_seeds_solve_at_least_eighteen_within_the_backtrack_cap` green, with the solve count and
  the p50 bake time posted, bytes identical x2.
- **kill line:** if 20 seeds do not solve 18 or more within the cap at p50 100 ms or less, or if
  any float enters the solver, the row dies.
- **cost:** S.
- **step:** 8.

### AMKX-14 — WD15 correction: the staged bake is in, the two proof legs are not

- **what it is:** The staged bake is code: `crates/world/src/ground/bake.rs:11 GROUND_MAGIC`
  (`WGND`), `:137 bake_ground(seed, size)`, `:160 bake_ground_bytes`, `:182 parse_ground`, the
  three passes (`:67 apply_wear`, `:91 scatter_props`, `:114 apply_rubble`), `:249
  blocked_tiles`, and the tests `:321 layout_parse_roundtrip`, `:346
  staged_has_wear_and_rubble_over_uniform_dirt`, `:362 no_blocked_paths_and_connected`, `:374
  refuses_before_allocating`, `:388 adjacent_seeds_differ_without_aliasing`, `:396
  props_sorted_bounded_and_kinded`. What is missing: the 100 ms bake leg is marked
  `#[ignore = "timing: bake budget 100ms, wall clock"]` at `:410-411`, so it never runs, and the
  blind A/B leg (staged against a single-pass bake) has no record at all.
- **donor:** `bones-ai/rust-procedural-world` (MIT, read 2026-10-20) for the staged bake
  discipline. Our own code otherwise.
- **closes:** WD15 (the two proof legs only; the bake itself is done).
- **owned paths:** `crates/world/src/ground/bake.rs` (this lane) and the board row text.
- **accept:** `node tools/gh-check.mjs cargo test -p engine-world --lib ground` prints
  `bake_fits_time_budget` green (the ignore is gone) and a new test
  `a_blind_pair_prefers_the_staged_bake` green, with the recorded pair.
- **kill line:** if the bake needs more than 100 ms at a 64 side, or if the blind pair prefers the
  single pass, the row stays open.
- **cost:** S.
- **step:** 8.

### AMKX-15 — The city grammar and the map import (WD13)

- **what it is:** The rule interpreter, the road graph and the map importer, so a district is
  made by a grammar and a map is imported from the real world. Today `crates/world/src/city.rs`
  does not exist and nothing imports a map.
- **donor:** the shapes of `flax-mcp` (MIT per our own `docs/EVIDENCE.md:38` and
  `docs/RESEARCH-VERDICT.md:495-505`), but the repo is gone: the live check
  https://api.github.com/repos/maximkone/flax-mcp answers 404 (read 2026-10-20), so no
  fresh read is possible and no bytes may be ported. Data source: OpenStreetMap through Overpass,
  ODbL (https://www.openstreetmap.org/copyright, read 2026-10-20: "OpenStreetMap provides open
  map data ... licensed under t..." the ODbL), so every imported cell needs an attribution row.
- **closes:** WD13 (and it feeds WD3).
- **owned paths:** `crates/world/src/city.rs` (this lane).
- **accept:** `node tools/gh-check.mjs cargo test -p engine-world --lib city` prints a new test
  `a_two_cell_grammar_walks_its_rule_bodies` green, plus `an_overpass_json_parses_into_a_road
  _graph` green, with the height cascade resolving and no float in the module.
- **kill line:** any float or any tick code in the city modules kills it; if the OSM attribution
  cannot become a provenance row, the import stops at the road graph.
- **cost:** M.
- **step:** 9.

### AMKX-16 — WD12 correction: the manifest is code and content, the assembler is missing

- **what it is:** The versioned cell manifest bytes exist: `crates/world/src/manifest.rs:28
  MANIFEST_MAGIC` (`WCM1`), `:30 MANIFEST_VERSION`, `:149 to_bytes`, `:193 parse`, `:174 gate`
  (the provenance gate over the EVIDENCE ledger), `:307 ManifestError::Unprovenanced` with the
  message "manifest {coord} asset {asset} has no EVIDENCE.md row" at `:331`, `:410 LUDUS_LEDGER`
  reading `docs/EVIDENCE.md`, and the test `:502
  one_unprovenanced_asset_refuses_the_cell_and_names_its_path`. The content exists too:
  `content/maps/manifest.json` (per-file SHA-256) and `content/maps/district/manifest.json` (four
  cells, seed 20260925, nav links, placement hash, CC0-1.0 provenance). What is missing: the
  assembler that hands one manifest per map to the cell stream, and the proof that the manifest
  bytes are identical x2.
- **donor:** our own code. No donor.
- **closes:** WD12 (the assembler and the proof only; the bytes and the gate are done).
- **owned paths:** `crates/world/src/manifest.rs` (this lane) and `crates/world/src/cells.rs`.
- **accept:** `node tools/gh-check.mjs cargo test -p engine-world --lib manifest` prints a new test
  `the_district_manifest_round_trips_and_is_identical_twice` green.
- **kill line:** if one asset without a provenance row does not refuse the whole cell with its
  path named, the gate is not done and the row stays open.
- **cost:** S.
- **step:** 9.

### AMKX-17 — WD3 correction: the district content is written, the generator is not

- **what it is:** The Rome district exists as content: `content/maps/district/manifest.json` (four
  cells: insula_a, insula_b, stall_fish, stall_bread, city_gate, city_wall, fountain; seed
  20260925; `nav_links` pointing at the route file; placement SHA-256; provenance CC0-1.0) and
  `content/maps/district/route_tv1.json` (west_approach to fountain_plaza with one vault, one
  mantle and one climb, the TV1 bounds inlined, and an `ai_chase` block that follows `nav_links`
  with `replay_exact: true`). The kit meshes are in `content/maps/src/` (insula.gltf,
  market_stall.gltf, fountain.gltf, street_wall.gltf, arch_gate.gltf). The row's accept ("a
  traversal route crosses the map with vault, mantle and climb; AI chases follow nav links") is
  met in the content. What is missing: nothing can make a second district (no grammar, `AMKX-15`),
  and the district is not wired into the streaming map by an assembler (`AMKX-16`).
- **donor:** our own content, made by `content/maps/tools/gen.mjs` (the CT9 lane) and the WD3
  lane; CC0-1.0, recorded in `docs/EVIDENCE.md:138`.
- **closes:** WD3 (correction and the generator half).
- **owned paths:** `content/maps/district/` (this lane) and the board row text.
- **accept:** `node tools/workspace-check.mjs` stays clean, and the district manifest is read by
  the assembler test of `AMKX-16` with the route file linked.
- **kill line:** if the route does not cross the map in the built cells, the content is a claim
  and the row stays open.
- **cost:** L for a generated second district.
- **step:** 9.

### AMKX-18 — The one-call project door (C12): `project.new`, `project.build`, `project.run`

- **what it is:** The CLI half of C12 is landed: `src/cli.rs:53-59` lists `new <template> <dir>`,
  `run --headless` and `build --target windows|web`; `src/project.rs` holds the embedded
  templates (arena-duel, siege) and the manifest load; `src/build.rs` runs the two targets. The
  door half is missing, and the run ignores the project: `src/project.rs:417 run_headless` uses
  only the seed from `content/seed.txt` and ignores `quality`, `gore`, `physics_layers` and
  `content` from `game.toml`, so every project plays the default arena proof.
- **donor:** our own code. The pack shape to copy: `crates/endpoint/src/packs/asset.rs` (a pack
  with verb consts, Args and Output structs, `crate::e2_tool!`, and one `pack.add(VerbSpec::of::<
  Tool>(NAME, PACK, card)?)` line) and `crates/endpoint/src/packs/mod.rs:78` `PACK_NAMES`
  (a fixed-size array, so a new pack is a 17).
- **closes:** C12.
- **owned paths:** HELD `crates/endpoint/src/packs/project.rs` (new), `crates/endpoint/src/packs/mod.rs`
  (one `pub mod project;` line, one `project::PACK` entry in both `PACK_NAMES` arrays, one line in
  the module doc), and `src/project.rs` plus `src/build.rs` (the run must use the manifest).
- **accept:** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib project`
  prints three new tests green: `project_new_writes_a_template_and_returns_its_path`,
  `project_build_reports_the_artifact`, `project_run_plays_the_project_scene_not_the_default_proof`
  (the receipt names the scene that ran). The golden `d57b5c8ae3096614` must not move.
- **kill line:** if a project with its own content and tier still plays the default proof, C12 is
  not done and the row stays open.
- **cost:** M.
- **step:** 10.

### AMKX-19 — The scene diff and merge window (FU2)

- **what it is:** An AI in the editor must see what it changed and merge with the human's change.
  Today the editor undo is whole-batch, so an agent's edit cannot be compared with the human's
  edit cell by cell.
- **donor:** the merge shape of `automerge/automerge` (MIT per the GitHub API, read 2026-10-20)
  and our own `crates/endpoint/src/scene_tools/spatial_delta.rs` (the scene graph delta the door
  already writes). No CRDT engine is ported: the merge is three-way over our own scene graph.
- **closes:** FU2.
- **owned paths:** HELD `crates/editor/src/commands.rs` (the row's own home; the editor is not
  this lane's crate) and `crates/endpoint/src/packs/scene.rs` if the diff is served over the door.
- **accept:** `cargo test --locked -p engine-editor commands` prints a new test green:
  `a_two_thousand_object_diff_posts_ms_and_bytes` (the ms and the byte count posted) and
  `a_conflicting_edit_is_refused_with_both_sides_named`. Undo stays whole-batch x2 and the golden
  does not move.
- **kill line:** if a merge cannot refuse a conflict with both sides named, it dies; a silent
  merge is worse than no merge.
- **cost:** S.
- **step:** 10.

### AMKX-20 — The open-prompt eval on the A6 tasks (FU3)

- **what it is:** The agent's work has to be scored without an LLM judge: a task set with open
  prompts, a programmatic referee per task and a pass-rate table per model.
- **donor:** the harness shape of `EleutherAI/lm-evaluation-harness` (MIT, read 2026-10-20): a
  task set, a scorer and a per-model table. `promptfoo` is already mined, so it is not repeated.
  `openai/evals` was checked and refused: its GitHub licence reads `NOASSERTION` today.
- **closes:** FU3.
- **owned paths:** HELD `crates/endpoint/src/discover.rs` (the row's own home) and a new
  `crates/endpoint/src/insight/eval_tasks.rs` or a `tools/` folder for the task set.
- **accept:** `cargo test --locked -p engine-endpoint discover` prints a new test green:
  `the_open_prompt_pack_posts_a_completion_and_token_delta` (the delta posted), the manifest
  bytes identical x2, and the golden unchanged.
- **kill line:** if the eval needs an LLM judge, it dies; the rule is programmatic referees only.
- **cost:** S.
- **step:** 10.

### AMKX-21 — The motion intake as BVH (NS19)

- **what it is:** Offline text-to-motion lands as clips the anim crate can read: BVH parse,
  retarget, quantize, clip, provenance row, bytes identical x2.
- **donor:** `nv-tlabs/kimodo` (Apache-2.0, https://raw.githubusercontent.com/nv-tlabs/kimodo/main/LICENSE,
  read 2026-10-20); its README table names the weights licence "NVIDIA Open Model License" for
  the SOMA models, and those terms are not parsed here (UNKNOWN). `NVlabs/SOMA-X` (Apache-2.0,
  GitHub API, read 2026-10-20). The empire already runs this on a free GPU:
  `C:\empire\lab\research\research\cards\kimodo-trial-2026-10-20\kaggle\encode_kernel.py` bakes
  16 SOMA BVH clips (read 2026-10-20).
- **closes:** NS19.
- **owned paths:** HELD `crates/anim/src/clip.rs` (the row's own home). The clip machinery is
  already there: `crates/assets/src/clip_intake.rs`.
- **accept:** `cargo test -p engine-anim --lib clip` prints a new test
  `an_intake_bvh_clip_retargets_and_plays_x2_hashed` green, and the golden does not move.
- **kill line:** if the retargeted clip is not byte-identical x2, or if the weights licence cannot
  be named, the clip stays in a test folder and never enters `content/`.
- **cost:** M.
- **step:** not on the ten-step path (a later lane).

### AMKX-22 — The sound intake through the CC0 vault (NS18)

- **what it is:** Event-to-sound tables need banked PCM with a provenance row. Today every
  sample in `crates/audio/src/bank.rs` is a placeholder synth (the header at lines 1-10 says so).
- **donor:** `facebookresearch/audiocraft` has MIT code
  (https://raw.githubusercontent.com/facebookresearch/audiocraft/main/LICENSE, read 2026-10-20) but
  its weights are CC-BY-NC, so under D3 they cannot ship: REJECT the weights, keep the code idea.
  MIT alternative for nonverbal audio: `suno/bark` (MIT LICENSE and MIT model card, read
  2026-10-20). The free path that already works is the empire's own CC0 vault:
  `C:\empire\studio\asset-vault\records\sfx-footstep-*.json` with `tools\provenance.py` and
  `tools\vault-license-gate.mjs` (read 2026-10-20).
- **closes:** NS18.
- **owned paths:** HELD `crates/audio/src/bank.rs` (the row's own home).
- **accept:** `cargo test -p engine-audio --lib bank` prints a new test green:
  `an_intake_sound_plays_from_an_event_with_its_provenance_row`, and the golden does not move.
- **kill line:** if the only source is a non-commercial weights set, the row does not land in
  `content/`; the CC0 path is free and already proven.
- **cost:** S.
- **step:** not on the ten-step path (a later lane).

---

## Patch-ready specs (no code written; the crates are HELD)

### S1 — `crates/assets/src/gen_intake.rs` (GN6 intake half, AMKX-09)

- New function `pub fn intake_parts(request: &PartSetRequest<'_>, cache: &mut IntakeCache) -> Result<IntakeOutput, IntakeError>`
  in `crates/assets/src/gen_intake.rs`; new types `PartSetRequest { parts: Vec<&[u8]>, name: String,
  provenance: Provenance, settings: IntakeSettings, max_tris: u32 }` and `PartSetOutput { bundle_hash,
  bundle_path, row_path, cache_hit, meshes: Vec<String>, tris: u32 }`.
- The pose comes from `engine_geo::parts::plan(&meshes, max_tris)` (landed, `AMKX-08`): one union
  box, one translation, one budget per part. `engine-assets` gains the optional dependency
  `engine-geo` behind the same off-by-default feature as the kit import (the `kit` feature), so
  the default build stays donor-free.
- Each part runs the existing steps (`weld`, `clean_tris`, `drop_specks`, `compact`), then
  `mesh_cook::simplify` to `plan.budget_per_part`, and the bundle holds one mesh named
  `FRAC_<name>_<ii>` per part (`mesh_cook::validate_frac_name` at `mesh_cook.rs:993`, the cap
  `MAX_FRAC_CHUNKS` at `mesh_cook.rs:58`).
- Tests in `crates/assets/src/gen_intake/tests.rs`: `four_parts_cook_to_frac_chunks_that_fit_together`
  (four icospheres side by side, summed triangles at most 5,000, the union hull contains every
  part, cook x2 byte-identical, one cache hit with the same bundle hash, a refused model writes
  nothing) and `a_part_set_over_the_part_cap_is_refused`.
- Proof: `node tools/gh-check.mjs cargo test -p engine-assets --features import,registry gen_intake`
  (the golden `d57b5c8ae3096614` must not move).

### S2 — `crates/endpoint/src/packs/project.rs` (C12, AMKX-18)

- New pack `project` with `pub const PACK: &str = "project";` and three verbs:
  `project.new`, `project.build`, `project.run`.
- `project.new` args `{ template: String, dir: String }`, output `{ project: String, manifest_path:
  String, files: Vec<String> }`. Refusals: `bad_template` (names the valid templates), `not_empty`
  (the directory is not empty), `bad_directory`.
- `project.build` args `{ manifest_path: Option<String>, target: String }`, output `{ target: String,
  artifact_path: String, receipt: String }`. Refusals: `bad_target` (names `windows` and `web`),
  `build_failed` (the tool's own message).
- `project.run` args `{ manifest_path: Option<String>, ticks: Option<u32> }`, output `{ receipt:
  String, final_hash: String, scene: String, ticks: u32 }`. Refusals: `bad_manifest` (names the
  key and the line), `build_failed`.
- Every verb is `readonly = false` except `project.new`? No: all three write, so all three are
  undoable and refuse by default (D6). The `CapabilityCard` for each is `CapabilityCard::Mutate`
  where the crate has that card, else `Observe` for the read-only parts; the builder picks the
  card that exists in `crates/endpoint/src/tool_capabilities.rs`.
- Registration: `crates/endpoint/src/packs/mod.rs` gets `pub mod project;`, `project::PACK` in
  both `PACK_NAMES` arrays (16 becomes 17), and the module doc line updated. The pack loads on
  demand through `api.load`, so the tool-list budget only grows when the pack is loaded.
- Proof: `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib project`
  (three new tests green, golden unchanged).

### S3 — `crates/endpoint/src/packs/asset/generate_run.rs` (GN7, AMKX-02)

- The file GN7 names, in the asset pack, behind the existing `intake` feature.
- `asset.generate` keeps its name and its explainer role, and gains a runner mode: args
  `{ image_path: Option<String>, name: String, kind: Option<String>, n: Option<u32>, faces:
  Option<u32> }` start `node <center>/make.mjs run <kind> --spec <file>` and return `{ run_id:
  String }` parsed from the `started run <id>` line; args `{ run_id: String }` run `node <center>/
  make.mjs job <run_id> --json` and return `{ state: String, files: Vec<String> }`. When the state
  is done, the verb calls `asset.intake` on every returned GLB with `lane_json` set.
- Before starting, `IntakeCache::lookup` is asked with the same generation inputs; a hit returns
  the cached bundle hash with no GPU job.
- The center folder comes from `E2040_CENTER_DIR`; unset is refused with the code `no_center` and
  one next step.
- Proof: `cargo test -p engine-endpoint --features intake asset` with a fake `make.mjs` (a node
  script in a temp folder that prints the two lines and writes a GLB plus `kaggle-lane.json`): the
  start returns a run id, the poll returns done and an intake bundle, a second start with the same
  inputs returns `cache_hit: true` and the fake script does not run again.

## Stale row corrections (the board text, with the proof)

| Row | What the board says | What the tree holds (file and line) | The correction |
|---|---|---|---|
| WD12 | "versioned cell manifest bytes (own magic, seed, per-cell asset list) with a provenance gate refusing cells whose assets lack EVIDENCE.md rows" — reads as unwritten | `crates/world/src/manifest.rs:28` `MANIFEST_MAGIC = *b"WCM1"`, `:30` `MANIFEST_VERSION`, `:149` `to_bytes`, `:174` `gate`, `:193` `parse`, `:307` `ManifestError::Unprovenanced`, `:331` the message "manifest {coord} asset {asset} has no EVIDENCE.md row", `:410` the ledger read, `:502` the test `one_unprovenanced_asset_refuses_the_cell_and_names_its_path`; content `content/maps/manifest.json` and `content/maps/district/manifest.json` | The bytes, the gate and the test exist. Keep the row, shrink it to the assembler plus the bytes-identical-twice proof (AMKX-16). |
| WD3 | "Rome district map — streets, rooftops and a market laid out for TV1 traversal fights" — reads as unwritten | `content/maps/district/manifest.json` (four cells, seed 20260925, `nav_links`, placement SHA-256, CC0-1.0 provenance) and `content/maps/district/route_tv1.json` (west_approach to fountain_plaza, links `kind` vault, mantle and climb with the TV1 bounds, an `ai_chase` block that follows `nav_links` with `replay_exact: true`); kit meshes `content/maps/src/insula.gltf`, `market_stall.gltf`, `fountain.gltf`, `street_wall.gltf`, `arch_gate.gltf` | The content and the route exist and the row's accept is met in the content. Keep the row, shrink it to "a district a grammar can make" (AMKX-15, AMKX-17). |
| WD14 | "propagate-and-collapse over authored node restrictions (wall, rubble, gate, door tiles plus room-adjacency graphs) with a fixed backtrack cap" — reads as unwritten | `crates/world/src/wfc.rs:43` `Adjacency`, `:192` `bake(width, height, adjacency, seed)` (no authored restrictions in the signature), `:190-191` the doc "A contradiction is refused, never retried", `:141` `WfcError::Contradiction`, `:283` `next_cell`, `:344` `propagate`, `:229` `is_connected`; tests `:446`, `:458`, `:482`, `:501`, `:534`, `:554` | A WFC tile bake exists, but not the node-constraint solver. The row stands; it must name `crates/world/src/wfc.rs` (not `crates/world/src/gen/wfc.rs`, a path that does not exist) and list the four open legs (AMKX-13). |
| WD15 | "staged arena-ground bake — ordered seeded passes (base → wear → scatter) ... to deterministic tile/prop tables with a preview capture" — the board already carries the progress note | `crates/world/src/ground/bake.rs:11` `GROUND_MAGIC = *b"WGND"`, `:137` `bake_ground`, `:160` `bake_ground_bytes`, `:182` `parse_ground`, the passes `:67 apply_wear`, `:91 scatter_props`, `:114 apply_rubble`, `:249` `blocked_tiles`, tests `:321`, `:346`, `:362`, `:374`, `:388`, `:396`; and `:410-411` `#[ignore = "timing: bake budget 100ms, wall clock"]` on `bake_fits_time_budget` | The bake is code and its behaviour is tested; two legs are open: the ignored 100 ms test and the blind A/B leg, which has no record. Keep the row, shrink it to the two legs (AMKX-14). |

## The ten-step path with the row ids

1. GN8 — read the licence off the model card and fail closed. (AMKX-03, AMKX-04, AMKX-05, AMLX-06)
2. GN7 — a job runner that starts a GPU job, returns a run id and can be polled. (AMKX-01, AMKX-02)
3. GN5 — the `parts` job writes N part GLBs for one prop. (AMKX-07)
4. GN6 — `intake_parts` cooks the part set into FRAC_ chunks. (AMKX-08, AMKX-09)
5. GN4 — a deterministic texture bake writes the base-colour map. (AMKX-10)
6. NS17 — the texture and material intake writes the provenance row. (AMKX-11)
7. NS20 — a second mesh source hashed into the cache by prompt bytes. (AMKX-12)
8. WD14 then WD15 — the tiling collapses from node restrictions, the ground bakes in stages. (AMKX-13, AMKX-14)
9. WD13 then WD12 then WD3 — city grammar, manifest bytes, then the district. (AMKX-15, AMKX-16, AMKX-17)
10. C12 with FU2 and FU3 — the project verbs in the door, the diff window, the open-prompt eval. (AMKX-18, AMKX-19, AMKX-20)
