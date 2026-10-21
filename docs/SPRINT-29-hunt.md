# Hunt 2026-10-21: steals from outside the engine set (owner 2026-10-21)

Boarded from the research dive in `docs/research/hunt-2026-10-21/`. Every donor was
verified live on 2026-10-21 with `gh api repos/<owner>/<repo>` and the donor file
path was read, not guessed. Evidence and the full list: `docs/research/hunt-2026-10-21/`.
Rules v1 and the golden `d57b5c8ae3096614` never move for these rows.

Donor license does not matter (owner 2026-10-10: any donor code may be ported), but
the donor path, the commit and the idea are written down, so the next reader does
not re-mine the same repo.

- [ ] HX1 render: the GPU radix sort. `crates/render` gains a WGSL radix sort
  (histogram, prefix, scatter) lifted from `KeKsBoTer/web-splat`
  (`src/shaders/radix_sort.wgsl`, Apache-2.0, 307 stars, pushed 2026-10-13), used to
  order the **crowd's** instances and bones and the GPU-driven indirect instance
  list. The particle pool stays CPU-sorted: 4096 keys is under 0.1 ms on the CPU
  and the CPU sort keeps the draw deterministic, so the sort is bought only where
  the count hurts. The shader constants are pinned by a Rust test, the way the
  donor pins them, so a shader and a host constant cannot drift.
  Tier A. Size S. Owns `crates/render/src/radix_sort.rs`,
  `crates/render/src/shaders/radix_sort.wgsl`, `crates/render/src/lib.rs`.
  Accept: `cargo test -p engine-render --lib radix` passes: 20000 random u32 keys
  sort to the same order twice, the result equals a CPU sort of the same keys, and
  the shader constants match the Rust constants (the test names the mismatch).
  F2P: no sort in `crates/render`; the crowd draw list is built on the CPU.
  P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-render --lib` stays green.
  needs: none.
- [ ] HX2 provider: the model on our own GPU. A second local provider runs a Llama
  forward pass in WGSL compute on the same wgpu device the renderer uses, so the
  in-game model is a compute pass in our process, not a sidecar HTTP server. Donor:
  `Beledarian/wgpu-llm` (`crates/core/shaders/gemm.wgsl`, `softmax.wgsl`,
  `rmsnorm.wgsl`, `rope.wgsl`, `crates/core/src/kv_cache.rs`, `crates/core/src/sampling.rs`;
  Apache-2.0, 13 stars, pushed 2026-08-30). It runs TinyLlama 1.1B in f16 on 2.05 GiB
  of weights and in INT8 block-64 on 1.27 GiB, and it already runs on a phone GPU.
  Tier A. Size M. Owns `crates/provider/src/wgpu_model.rs`,
  `crates/provider/src/lib.rs`, `crates/provider/src/local.rs`.
  Accept: `cargo test -p engine-provider --lib wgpu_model` passes: a 2-token prompt
  on a fixed seed returns the same tokens twice, the KV cache grows by one entry per
  token, and the request path never opens a socket (the test refuses a remote host).
  F2P: `crates/provider` has no in-process model.
  P2P: golden `d57b5c8ae3096614` x2; the existing mock, local and remote adapters
  keep their parity test green.
  needs: none.
- [ ] HX3 endpoint: the AI bill of materials. Every door edit records who made it
  (human or agent id), which model, the prompt fingerprint, the tape hash before and
  after, and which acceptance check passed; the record is served as a door resource
  and printed in the receipt. Donor: `Open-fab-ai/openfab`
  (`docs/generation-predicate-v0.1.md`, `src/core/provenance.rs`; Apache-2.0, 6 stars,
  pushed 2026-10-02). The record shape and the verification rules are stolen; the
  Sigstore and in-toto crypto are not.
  Tier A. Size S-M. Owns `crates/endpoint/src/attest.rs`,
  `crates/endpoint/src/resources.rs`, `crates/endpoint/src/lib.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib attest` passes: an edit
  made by an agent id and one made by a human carry different attributions, a record
  with a prompt hash and a tape hash is refused when either is missing, and the
  receipt line prints the agent id, the model and the tape hash.
  F2P: no attestation in the receipt.
  P2P: golden `d57b5c8ae3096614` x2; the 33 shipped prompt skills still list and get.
  needs: none.
- [ ] HX4 tools: the WebGPU conformance gate. The strict flag already exists
  (`crates/render/src/backend.rs`, `E2040_STRICT_WEBGPU=1` in CI), but nothing
  *measures* against the official suite. CI runs a slice of the WebGPU conformance
  suite against the strict instance, with a per-case subprocess isolation and an
  expectations file so a known backend defect is marked expected instead of
  blocking. Donors: `gpuweb/cts` (BSD-3, 180 stars) and `infosia/webgpu-native-cts`
  (the harness, `expectations/wgpu-native-vulkan.txt` and `docs/FINDINGS.md`, which
  shows the wgpu-native backend aborting where Dawn and yawgpu do not).
  Tier A. Size S-M. Owns `tools/webgpu-cts.mjs`, `tools/expectations/wgpu.txt`,
  `.github/workflows/`.
  Accept: `node tools/webgpu-cts.mjs --list` prints the chosen cases and their
  expectations; a run on the PC prints `webgpu_cts: pass=N fail=0 crash=0` and
  exits 0; a case marked expected is reported as expected, never as pass; the tool
  fails if the instance was not strict.
  F2P: no conformance run, only the flag.
  P2P: the golden matrix workflow is untouched and still runs five targets.
  needs: none.
- [ ] HX5 rome: the real map. `content/rome/geo.toml` carries the Roman roads,
  aqueducts, city walls and urban areas of the empire, and the empire extent under
  Hadrian, from `AWMC/geodata` (ODbL-1.0, 68 stars: `Cultural-Data/roads/roads.geojson`,
  `aqueducts/aqueducts.geojson`, `walls/walls.geojson`, `urban_areas/urban_areas.geojson`,
  `political_shading/roman_empire_ce_117_extent`), with the settlement gazetteer of
  Hadrian from `Diasito/urbesetorbis` (BSD-2, `public/downloads/301023_AD130.csv`,
  7,891 places with coordinates, province and dates) and the public-domain British
  road set from Edinburgh DataShare (PDDL-1.0). Every entry names its source in
  `ref`, and the ODbL notice ships with the table.
  Tier A. Size S-M. Owns `content/rome/geo.toml`, `crates/rome/src/geo.rs`,
  `crates/rome/src/lib.rs`, `docs/plan/rome-codex.md`.
  Accept: `cargo test -p engine-rome --lib geo` passes: the table loads with every
  entry carrying a `ref`, a road count and a settlement count print and are stable
  across two loads, a point in the Danube region resolves to the nearest settlement
  within 5 km, and the ODbL notice is in the file header.
  F2P: no `crates/rome` and no `content/rome`.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX6 rome: the codex pipeline. A tool reads the two public-domain sources  Landed aaca5ad5. `node tools/codex-check.mjs` prints `codex: numbers=25 unmatched=0` and exits 0 (two runs byte-equal); a table value no passage supports prints `unmatched` and exits 1; a passage whose text hash moved prints `changed` and exits 1; the ditch reads 1480 mm wide and 888 mm deep with `ref = "PHG 49"`; `node --test tools/codex-check.test.mjs` 6 tests pass; `cargo test -p engine-rome --lib` 18 passed; the golden d57b5c8ae3096614 x2 (`proof_cli default_and_max_seed` 1 passed, the release binary printed final_hash=d57b5c8ae3096614); `node tools/workspace-check.mjs` PASS.
  (Vegetius *De re militari* I, Pseudo-Hyginus *De munitionibus castrorum*) from
  the machine-readable corpus `cltk/lat_text_latin_library` (Public Domain Mark
  1.0: `hyginus/hyginus6.txt` paragraphs `1.`-`58.`, `vegetius1.txt` chapters
  in Roman numerals) and prints the numbers they carry next to
  `content/rome/camp.toml` and `content/rome/line.toml`, so a changed number in
  the table shows as a diff against the source. **The passage's text hash is
  stored beside the ref**, so a re-numbered edition fails the lint instead of
  silently changing a ditch. The passages and the conversions are already
  written down in
  `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md` (25 numbers with
  chapter refs, 1 Roman foot = 296 mm).
  Tier A. Size S. Owns `tools/codex-check.mjs`, `content/rome/camp.toml`,
  `content/rome/line.toml`, `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md`.
  Accept: `node tools/codex-check.mjs` prints `codex: numbers=25 unmatched=0` and
  exits 0; a table value that no passage supports is printed as `unmatched` and
  exits 1; a passage whose text hash changed is printed as `changed` and exits 1;
  the ditch row reads 1480 mm wide and 888 mm deep with `ref = "PHG 49"`.
  F2P: no codex tool and no `content/rome` tables.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [ ] HX7 render: GPU-driven meshlet culling. The cull pass gains normal-cone
  filtering and an indirect multi-draw, and the draw path reads the indirect args
  (the row GS18b waits for that hook). Donor: `nff747/meshlet-culling-wgsl`
  (Apache-2.0, 2 stars, pushed 2026-10-19: `src/shaders/meshlet_cull.wgsl.ts`,
  `src/math/cone_cull.ts`, `src/math/frustum.ts`, `src/runtime/culling_pipeline.ts`).
  The WGSL is the payload; the donor's own tests are the idea we copy: the cull
  efficiency is a number we can print.
  Tier A. Size S. Owns `crates/render/src/shaders/cull.wgsl`,
  `crates/render/src/gpu_cull.rs`, `crates/render/src/draw.rs`.
  Accept: `cargo test -p engine-render --lib gtao` and the cull tests pass; a mesh
  whose cluster normals all point away from the camera is culled by the cone test
  and a mesh with mixed normals is not; the cull report prints `culled=N visible=M`
  and the visible set matches the CPU oracle.
  F2P: the cull has no cone test.
  P2P: golden `d57b5c8ae3096614` x2; the CPU oracle in `draw.rs` stays green.
  needs: none.
  Progress 2026-10-21 (hunt wave 2, `docs/research/hunt2-2026-10-21/`): a second
  donor for the same row, `Scthe/nanite-webgpu` (MIT, 1124 stars, pushed
  2026-05-09) — an independent WebGPU Nanite, **not** the paper's own code
  (GitHub code search for the paper id `2404.06359` returns only paper-digest
  repos, so the paper's codec was never released). What it adds to this row is
  the **per-meshlet** half: it culls per instance *and* per meshlet, builds the
  meshlet LOD hierarchy in the browser with meshoptimizer + METIS, and packs
  depth `u16` + octahedron normals `2xu8` into 32 bits because WebGPU has no
  `atomic<u64>`. Its numbers to hold us to: 16:1 index-buffer compression, and
  15.5 M triangles decompressed and rendered in **0.59 ms** on an RX 7900 XTX;
  its Jinx scene is 640 M triangles with nearly 98 % of them software
  rasterized. Our gap is named: `crates/render/src/shaders/cull.wgsl:64` has one
  Hi-Z wall test and `:25` a sphere-vs-6-planes reject, so nothing culls a
  single cluster. Landing: `crates/render/src/shaders/cull.wgsl`, `:25`, `:64`;
  `crates/render/src/gpu_cull.rs:237` (`cull`), `:366` (`verify`);
  `crates/render/src/meshlet.rs:39` (`Cluster`), `:47` (`cluster_cut`). The
  row's Accept is not lowered; it gains the cluster count the LOD cut chose in
  the same report, and the 16:1 ratio as a target line on the cooked meshlet
  index payload. Golden: does not threaten — the cull is presentation.
- [ ] HX8 physfx: cloth for capes and banners. The float physics layer gains a
  cloth solver in wgpu compute, with a headless mode and a throughput example, so
  capes and banners move in the arena. Donor:
  `DeharengOlivier/gpu-cloth-simulation` (`src/compute.wgsl`, `src/simulation.rs`,
  `src/timestep.rs`, `src/headless.rs`; MIT, 1 star, pushed 2026-08-28). The layer
  is float and never hashed.
  Tier B. Size S-M. Owns `crates/physfx/src/cloth.rs`, `crates/physfx/src/lib.rs`.
  Accept: `cargo test -p engine-physfx --lib cloth` passes: a 8 x 8 sheet pinned at
  its top row sags under gravity and settles, two runs of 200 steps agree within
  1e-3, and the cloth never enters the hashed tick.
  F2P: no cloth in `crates/physfx`.
  P2P: golden `d57b5c8ae3096614` x2; the ragdoll and debris budgets stay inside
  their caps.
  needs: none.
- [ ] HX9 endpoint: the project memory the door serves. The door keeps a project
  memory (decisions, failed fixes, tuned values, each with the tape hash it was
  proved on) with a lifecycle: reinforced, marked outdated, restored, exported,
  forgotten; recall is keyword first with a vector second. Donor:
  `Beledarian/mcp-local-memory` (MIT, 60 stars, pushed 2026-09-14). The design is
  stolen; the TypeScript and the SQLite FTS5 server are not: the store is ours.
  Tier A. Size M. Owns `crates/endpoint/src/memory.rs`, `crates/endpoint/src/lib.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib memory` passes: a
  stored decision is recalled by keyword, a marked-outdated memory is not returned
  by a plain recall but is still in the export, and every stored record carries the
  tape hash it was proved on.
  F2P: no memory in the door.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: none.
- [x] HX10 tools: the ship build is reproducible. A CI job builds the Windows ship  Landed 8303e625d. `node tools/ship-check.mjs --print` prints one SHA-256 content hash per file and a tree hash over `path\0content-hash` pairs; the path rides inside the hash, so a renamed file moves the tree hash. `node --test tools/ship-check.test.mjs` 7 tests pass: the same tree prints the same tree hash, one changed byte changes the tree hash and only that file's line, a renamed file changes it, the lines are sorted by path without the locale, a missing folder exits non-zero and names the folder, an empty folder is refused, and the hash flag prints only the tree hash. `node tools/workspace-check.mjs` PASS. The Windows job in `.github/workflows/ship-check.yml` builds the ship profile twice from a clean folder (`--profile ship -p engine-game --features app`, the same profile `tools/ship.mjs` packs) and compares the two tree hashes; equal hashes are the receipt, a mismatch names the file that moved. The tool reads bytes only: no clock, locale or timezone enters the hash. The mirror job then went red and the red was true: two cold ship builds of one commit differed in twenty bytes per executable, the COFF time stamp and the CodeView debug GUID (the PDB GUID and its age), which the MSVC linker writes fresh in every link. `tools/ship-check.mjs` now reads a PE image and zeroes exactly those fields before it hashes it, and it leaves the PDB out of the tree and names it in the receipt (the PDB is debug data and is not in the shipped bundle). Landed 67dbcb126. The job also no longer lets a newer push cancel it: two cold fat-LTO builds take longer than the gap between two pushes to the mirror, so every run was cancelled before it could compare the two trees. Landed 54dc8ae7f. The mirror job now passes: `SHIP-CHECK PASS: the two ship builds have the same tree hash 4a5eae7f0f78404405cf7b15a2b6d4c89c0757c9445336b53fd5c1159165bede` (run 39642426754, head a3c4b33cc). `node --test tools/ship-check.test.mjs` 12 tests pass.
  profile twice from a clean folder and compares the file trees by content hash, and
  the receipt names the tree hash. Donors: `rust-secure-code/cargo-repro`
  (Apache-2.0, 58 stars) and `omnibor/omnibor-rs` (Apache-2.0, 30 stars) for
  fine-grained content-addressed file naming.
  Tier B. Size S-M. Owns `tools/ship-check.mjs`, `.github/workflows/`.
  Accept: `node tools/ship-check.mjs --print` prints the file tree with one content
  hash per file and a tree hash over them; two builds of the same commit print the
  same tree hash, and a tree hash that changes when one byte changes is the F2P.
  F2P: no ship check.
  P2P: the golden matrix workflow is untouched.
  needs: none.
- [ ] HX11 render: the real TAA resolve. The temporal resolve grows from a plain
  min/max box clamp into a real TAA and TAAU: Karis reversible tonemap working
  space, YCoCg, the Playdead clip-to-box variance clip, a Catmull-Rom history
  from five bilinear taps, closest-depth velocity dilation, disocclusion by
  history view-depth mismatch (the history's alpha carries last frame's view
  depth), jitter-aware neighbourhood reconstruction and temporal upsampling. Donor:
  `Siroko/kansei` (`rust/kansei-core/src/shaders/taa_resolve.wgsl`, 231 lines, plus
  `motion_vectors.wgsl`; MIT, 28 stars, pushed 2026-10-18), the only donor in the
  hunt that is a complete TAA and TAAU resolve in plain WGSL inside a Rust engine.
  The ghosting axes come from `vga-group/taa-kdop-optimizer` (MIT-0, 4 stars).
  Rows LF5c (resolve), LF5b (jitter), LF5 (motion vectors), PW01 (velocity target);
  gaps B7 and B9.
  Tier A. Size M. Owns `crates/render/src/shaders/post/taa_resolve.wgsl`,
  `crates/render/src/post/taa.rs`, `crates/render/src/shaders/motion_vectors.wgsl`.
  Accept: `cargo test -p engine-render --lib temporal` passes: a frozen camera and
  a moving light produce a stable history, the neighbourhood clamp never reads
  outside the clip box, the resolve output differs from the plain box-clamp resolve
  on a ghosting fixture (a pass that changes no pixel does not count), and the CPU
  twin of the resolve agrees with the GPU answer inside the tolerance the twin
  already pins.
  F2P: the temporal resolve has no neighbourhood clamp and no disocclusion.
  P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-render --lib` stays green.
  needs: none.
- [x] HX12 ci: the test report a stranger can read. The CI publishes a structured
  test report (the CTRF JSON shape) for the door and playtest suites, so a run's
  result is read without opening the log. Donor: `ctrf-io/github-test-reporter`
  (MIT, 378 stars, pushed 2026-10-11). Landed c879adfaa; fixed in 2af474f85 (the tool
  finds the receipt itself, the CI log keeps stderr, and the log is checked per status:
  `node --test tools/ctrf-report.test.mjs tools/ci.test.mjs` 23 pass; a real nextest
  0.9.149 run agrees with its receipt).
  Tier C. Size S. Owns `tools/ctrf-report.mjs`, `.github/workflows/`.
  Accept: `node tools/ctrf-report.mjs --out target/reports/ctrf.json` writes a
  report whose `summary.tests` counts equal the number of tests the run printed,
  and the file is byte-equal across two runs of the same tree.
  F2P: no structured report.
  P2P: the golden matrix workflow is untouched.
  needs: none.
- [ ] HX13 web: the browser proof under stress. The browser net proof injects
  faults (a dropped frame, a slow frame, a stalled load) and the page recovers
  without a stuck fight, with the recovery printed as a line. Donor:
  `mizchi/chaosbringer` (MIT, 48 stars, pushed 2026-10-19) for the pattern of
  chaos inside a browser test, and `WithSecureOpenSource/flaky-tests-detection`
  (Apache-2.0, 26 stars) for the idea that a red that repeats is not a red.
  Tier B. Size S. Owns `tools/web-chaos.mjs`, `crates/web/src/recovery.rs`.
  Accept: `node tools/web-chaos.mjs --faults=drop-frame,slow-frame,stall-load`
  prints `web_chaos: faults=3 recovered=3 stuck=0` and exits 0; a run with one fault
  removed prints `recovered=2`, so the line cannot be green by accident.
  F2P: no fault injection in the browser proof.
  P2P: golden `d57b5c8ae3096614` x2; the wasm golden digests stay equal.
  needs: none.
- [ ] HX14 web: the worker is the authority. The page moves the tick into a Web
  Worker that publishes one snapshot per tick into a small pool of transferred
  `ArrayBuffer`s, and the page returns a **credit** when it has drawn the frame.
  When the credits run out the authority stops ticking, so the back-pressure is
  the page's own drawing speed. Every publication carries the tick and the
  **server digest**, so the browser is a participant and not a viewer. Donor:
  `dzhng/battlesim` (`web/src/battle/sim/authority.ts`; no licence file, so the
  shape is re-derived, not copied; 5 stars, read 2026-10-21).
  Tier A. Size M. Owns `crates/web/src/worker_authority.rs`, `crates/web/src/fight.rs`.
  Accept: `cargo test -p engine-web --lib worker_authority` passes: a worker that
  publishes faster than the page draws stops publishing (the credit pool empties
  and the status reads `waiting-consumer`), a page that draws fast keeps the
  authority running, and the digest published beside the tick equals the digest
  the window prints for the same tick.
  F2P: the page drives the tick and the draw on one thread.
  P2P: the page's own golden line `d57b5c8ae3096614` does not change; the wasm
  golden digests stay equal.
  needs: none.
- [x] HX15 web: the frame bus. The sim state crosses to the draw thread over a  Landed 8b3cb4431. `crates/web/src/frame_bus.rs` holds the seqlock: the writer bumps a counter to odd, writes the payload, bumps it to even, and a reader that sees the counter change throws its torn copy away and retries, at most `MAX_READ_ATTEMPTS` (8) times, then says so instead of spinning. The payload lives in a fixed array of `AtomicU32` words next to the counter, so the module needs no `unsafe` and no `UnsafeCell`; the writer stores the words relaxed and publishes them with a release store of the counter, and the reader loads the words relaxed and fences around the counter it reads twice. The same reader runs over a plain byte buffer (`PlainFrame`), so the postMessage fallback and the shared path share one code path. `cargo test -p engine-web --lib frame_bus` 10 passed (run 39614971453): a reader never sees a half-written frame, a torn copy is thrown away and the read succeeds, a real writer-thread race never shows a mixed frame, the plain path and the shared path agree byte for byte, no read waits on a write, a read gives up after a bounded number of attempts, the shared path stores the payload as little-endian words, bytes that do not fit one frame are refused, many readers see the same frame, and a frame keeps its bytes exactly. Clippy `-p engine-web --all-targets -- -D warnings` PASS (run 39615496316); rustfmt exit 0; `node tools/workspace-check.mjs` PASS. The module is std-only and needs no hash, so `engine-core` was not pulled in.
  **seqlock**: one writer, many readers, nobody waits. The writer bumps a counter
  to odd, writes the payload, bumps it to even; a reader that sees the counter
  change discards the torn copy and retries. The same reader works over a plain
  `ArrayBuffer`, so the `postMessage` fallback and the shared path share one
  code path. Donor: `nithish101/resonance`
  (`packages/web/src/lib/frames.ts`, MIT, read 2026-10-21).
  Tier B. Size S-M. Owns `crates/web/src/frame_bus.rs`, `crates/web/src/lib.rs`.
  Accept: `cargo test -p engine-web --lib frame_bus` passes: a reader never sees a
  half-written frame (a writer that mutates the payload while a reader copies
  makes the reader discard and retry), the `ArrayBuffer` path and the shared path
  agree byte for byte, and no read waits on a write.
  F2P: no bus between the sim and the draw.
  P2P: golden `d57b5c8ae3096614` x2; the page still plays the same match.
  needs: none.
- [x] HX16 net: the desync locator. A Merkle tree over the entity state finds a Landed 39e2fef8f. `cargo test -p engine-net --lib merkle` 9 passed (local lane, CARGO_INCREMENTAL=0); rustfmt exit 0; `node tools/rust-quality.mjs --owned crates/net/src/merkle.rs,crates/net/src/lib.rs` PASS (score 97, grade A); `node tools/workspace-check.mjs` PASS. The locator lives in `crates/net/src/merkle.rs` on top of `engine_core::digest::MerkleDigest`; `crates/net/src/guard.rs` was left alone because it is the input admission screen, not a desync check.
  desync in O(log n) instead of a full compare: the root hash is compared first
  and only the differing branch is walked, down to the entity. It is a
  **diagnostic only** and never a gate, so it stays off the golden. Donor:
  `braedonsaunders/voidstrike` (`src/engine/network/MerkleTree.ts`, MIT, 26 stars,
  read 2026-10-21).
  Tier B. Size M. Owns `crates/net/src/merkle.rs`, `crates/net/src/guard.rs`.
  Accept: `cargo test -p engine-net --lib merkle` passes: two states that differ
  in one entity are located at that entity in at most log2(n) comparisons, two
  equal states compare equal at the root, and the tree never enters the tick
  hash.
  F2P: a desync is found by a full compare.
  P2P: golden `d57b5c8ae3096614` x2; the snapshot codec digests do not change.
  needs: none.
- [ ] HX17 web: the time machine. The page can open the fight at a named tick
  (`?fight&tick=420`): the state is advanced in fixed steps and then drawn, with
  no wall clock, so the same input gives the same pixels on every machine. A
  reviewer, a bug report or a door verb can then ask the browser for one moment
  instead of a whole run. Donor: `nithish101/resonance`
  (`packages/web/src/render/loop.ts` and
  `packages/web/tests/visual.spec.ts` plus
  `docs/decisions/0007-deterministic-capture.md`, MIT, read 2026-10-21).
  Tier B. Size M. Owns `crates/web/src/fight.rs`, `crates/web/src/arena_view.rs`,
  `tools/web-browse.mjs`.
  Accept: `cargo test -p engine-web --lib seek` passes: two runs at `tick=420`
  give the same state digest and the same capture hash, a tick beyond the match
  is refused with a typed error, and the golden line still prints the full
  match digest.
  F2P: the page can only play from the start.
  P2P: golden `d57b5c8ae3096614` x2; the wasm golden digests stay equal.
  needs: none.
- [ ] HX18 physics: fight feel becomes integer table rows. The per-bone muscle
  table, the per-joint degree limits and the **hop-spread** strength reduction
  (a graph walk where each hop keeps `1 - distance/(spread+1)` of the strength)
  move into the authority layer as integers, so the same hit produces the same
  stagger, limp and knockdown on every PC and in replay. Donor:
  `blugart-dev/kickback` (`addons/kickback/resources/ragdoll_tuning.gd`,
  `resources/impact_profile.gd`, `active_ragdoll_controller.gd`,
  `resources/joint_definition.gd`; MIT, 28 stars, read 2026-10-21). The numbers
  are GDScript tables, so the tables are ported, not the code.
  Rows **PX9** (live), **C10** (one-call physical effects), **AM24** (the pops
  residual).
  Tier A. Size M. Owns `crates/physics/src/joint_limits.rs`,
  `crates/physics/src/stagger_memory.rs`, `crates/physfx/src/body.rs`.
  Accept: `cargo test -p engine-physics --lib joint_limits` passes: a per-joint
  row exists for the spine, shoulder, elbow, hip, knee and ankle with a flexion
  and an extension limit, a hit on the head spreads to the chest and then to the
  arm with the stated ratio, and two runs of the same hit give the same strength
  map and the same digest.
  F2P: `joint_limits.rs` has one cone derived from joint depth, not per-joint rows.
  P2P: golden `d57b5c8ae3096614` x2, with a receipt that names the moved rows.
  needs: none.
- [ ] HX19 proof: the two-tier hash. Every physical-effect receipt carries the
  **authoritative digest** (what the sim decided, integer) beside the **solver
  digest** (what the float solver did). When the golden moves, the receipt says
  whether the *sim* moved or the *solver* moved, instead of leaving a reviewer
  to guess. Donor: `mrdav30/Gravitas`
  (`src/Gravitas/Determinism/GravitasReplayHashMode.cs`,
  `GravitasReplayHashService.cs`, `Colliders/3D/LSCollider.ReplayHash.cs`;
  MIT, 1 star, read 2026-10-21).
  Rows **HS2** (the matrix), **C15** (the physics oracle).
  Tier A. Size S. Owns `crates/proof/src/two_tier.rs`, `crates/physfx/src/receipt.rs`.
  Accept: `cargo test -p engine-proof --lib two_tier` passes: a state change
  that only touches the solver caches leaves the authoritative digest equal and
  changes the solver digest, a state change in the sim changes both, and the
  receipt line names which one moved.
  F2P: one digest for everything.
  P2P: golden `d57b5c8ae3096614` x2; the existing receipts keep their shape.
  needs: none.
- [ ] HX20 core: a soft float. `crates/core` gains a 32-bit soft float (24-bit
  signed fraction, 8-bit exponent) with add, mul, div, floor, ceil, fmod, inv sqrt
  and log2 as integer shifts, so a float with f32's range keeps f32's exactness on
  every machine. That is the tier between our integer `Fixed` and the float
  physfx layer, and it makes "the hash is equal with the physical layer on" a
  reachable claim. Donor: `devlinzhou/deterministic_physics`
  (`gphysics/include/math/glacier_float.h`, `gphysics/src/math/glacier_float.cpp`;
  Apache-2.0, 42 stars, read 2026-10-21).
  Rows **C15** (the physics oracle), gap B7 (the hashed layer).
  Tier B. Size M. Owns `crates/core/src/soft_float.rs`, `crates/core/src/lib.rs`.
  Accept: `cargo test -p engine-core --lib soft_float` passes: the same
  operations give the same bits on two different inputs orderings, a value
  survives a round trip through the exponent range, and the type is rejected by
  a lint outside the crates that own it.
  F2P: no soft float; the hashed layer is Q47.16 integer or plain f32.
  P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-core --lib` stays green.
  needs: none.
- [ ] HX21 destruction: the fracture pattern belongs to the prop. The prop bakes
  a fixed **site list** (integer, hashed with the prop) and a hit only picks
  which sites are active and which seed stream each one uses, so the same wall
  breaks the same way wherever it is hit. Today the pattern is keyed on the hit
  position, so two hits on one wall make two patterns. Idea donor:
  `sgsellan/fracture-modes` (116 stars, GPL-3.0: read for the idea, no bytes);
  the watertight-cut half comes from `sinanata/unity-mesh-fracture`
  (`Assets/MeshFracture/Runtime/MeshFragmenter.cs`, MIT, 54 stars): every cut
  edge sealed by a cap polygon, earlier caps re-clipped by later planes, and a
  per-triangle submesh tag so a cut face carries inside material.
  Rows **RZ4** (walls and crates), **DX7** (the shard cut).
  Tier B. Size M. Owns `crates/destruction/src/csg.rs`, `crates/destruction/src/fracture.rs`.
  Accept: `cargo test -p engine-destruction --lib fracture` passes: the same prop
  hit at two different points gives the same chunk set, a cut mesh is closed (no
  open edge), and two runs of the same hit give the same chunk digest.
  F2P: the pattern is keyed on the hit position.
  P2P: golden `d57b5c8ae3096614` x2 with a receipt that names the moved rows.
  needs: none.
- [ ] HX22 anim: the foot-placement ledger. The sim records, in integers, the
  tick of every foot plant, its world position and the ground normal, so
  `foot_slide_cm` becomes a number the sim knows instead of one the picture
  guesses, and the picture replays the ledger instead of solving IK at run time.
  Donor: the shape comes from `TildeAsterisk/Physicanim`
  (`Scripts/Runtime/Physicanimator.cs`, `ConfigurableJointExtensions.cs`,
  `IKFootSolver.cs`; MIT, 36 stars, read 2026-10-21): the animated skeleton
  owns the target rotation, the physics joint gets a per-joint target composed
  with the joint's rest rotation, and the foot IK lives in its own module so it
  never fights the joint solve. Rows **AM24**, **LF12**, and the `foot_slide_cm`
  line of the product check.
  Tier B. Size M. Owns `crates/physics/src/planted.rs`, `crates/anim/src/ik.rs`.
  Accept: `cargo test -p engine-physics --lib planted` passes: a walk of 50 ticks
  plants both feet, the ledger holds the tick, the position and the normal, and
  the slide measured from the ledger is within 1 cm of the slide measured by
  the picture.
  F2P: the foot slide is only measured in the picture.
  P2P: golden `d57b5c8ae3096614` x2; the pops count does not rise.
  needs: none.
- [ ] HX23 rome: the line becomes data. The Roman formation is a **table**, not
  geometry: one entry per layout (line, column, wedge, shield wall, loose order,
  circle, arc), each with a lateral spacing scale, a depth spacing scale, a rank
  stagger, a front-rank tightening, and a **jitter that is a hash of the unit
  seed and the soldier index**, clamped so two men never come closer than a
  stated minimum. Four layers never mix: the layout of the men in one troop, the
  formation of the troops in an army, the combat stance, and the order. A troop
  can be in a Roman line, marching, internally braced in a defensive layout, and
  under hold at the same time. Donor: `djeada/Standard-of-Iron`
  (`assets/data/formations/unit_layouts/rome_shield_wall.json`,
  `rome_close_order_infantry.json`, `rome_marching_column.json`,
  `assets/data/formations/army/rome.json`; MIT, 62 stars, pushed 2026-10-18,
  read 2026-10-21). The **numbers are that game's balance, not history**: every
  value lands in `content/rome/line.toml` with a `ref`, and where history exists
  (3 ft frontage, 6 ft rank gap) the table wins.
  Rows **LEG3** (the contubernium falls in), **LEG12**, and gap **C11** (the
  data-driven game layer).
  Tier A. Size M. Owns `content/rome/formations/*.toml`, `crates/ai/src/legion.rs`.
  Accept: `cargo test -p engine-ai --lib legion` passes: a table with a layout
  and no `ref` is refused, a unit in the shield-wall layout holds its slots
  within 15 cm, the jitter is a hash of the unit seed and the soldier index (the
  same seed twice gives the same jitter, a different seed does not), and no two
  men are closer than the stated minimum.
  F2P: the formation is hard-coded geometry in `crates/ai/src/legion.rs`.
  P2P: golden `d57b5c8ae3096614` x2; the rules v1 scene is untouched.
  needs: none.
- [x] HX24 rome: the groma lays the camp. The marching camp is generated by the Landed 531854bff. `crates/rome/src/groma.rs` surveys the camp out of a height grid: the ground decides the size in whole actae, the *clavicula* is struck from the gate hinge with the circinus open and bisects the camp, the corners are rounded from the corners of the cohort lines, and the tenth gate stands on the high point of the rear wall. `cargo test -p engine-rome --lib camp` passes (run 39593398101): a flat 200 m site with threat `light` returns four ditch runs of 2664 x 2072 mm (one night: 1480 mm wide, 888 mm deep), the clavicula halves the camp, the tenth gate stands on the high point, and two surveys of the same ground give the same plan. Clippy `-p engine-rome --all-targets -- -D warnings` PASS (run 39593403812). Golden `d57b5c8ae3096614` x2 (runs 39593454390, 39593457852).
  **survey procedure** of the source, not by a rectangle: from the praetorian
  gate strike the *clavicula* (a chord from the gate hinge with the circinus
  open) to find the centre, circle the corners of the cohort lines to fix the
  sides, and place the tenth gate on the high point. The plan is **checkable**:
  the *clavicula* must bisect the camp, the gate spacing must be a whole number
  of *actae*, and the frontage must be the cohort frontage (30 ft of frontage per
  720 ft of depth). Sources: Pseudo-Hyginus 54-55 (the groma and the circinus)
  and 49-50 (the ditch and the rampart), read in the machine-readable corpus
  `cltk/lat_text_latin_library` (`hyginus/hyginus6.txt`, 22.6K, Public Domain
  Mark 1.0, 28 stars); the numbers are in
  `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md`.
  Rows **LEG7** (the camp works), **ROM5** (the `rome.camp` verb), **ROM2**.
  Tier A. Size M. Owns `crates/rome/src/camp.rs`, `crates/rome/src/groma.rs`.
  Accept: `cargo test -p engine-rome --lib camp` passes: a flat 200 m site with
  threat `light` returns four ditch runs whose sizes match `camp.toml` (one
  night: 1480 mm wide, 888 mm deep), the *clavicula* bisects the camp, the
  tenth gate stands on the high point, and two runs give the same plan.
  F2P: no `crates/rome` and no camp generator.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [x] HX25 rome: every number carries its tier. A registry holds every number Landed 4ae7d434. `cargo test -p engine-rome --lib certainty` 17 passed 0 failed (gh run 39589887759): a number with no tier is refused with the key `units.roman_foot_mm` named, an unregistered key cannot be rendered (`LoadError::Unregistered`), and the Roman foot prints with the tier `contested` and a note naming 295.7 mm or 297.4 mm; the HUD line of the pilum reads `pila 2.0 m (Vegetius 2.15)`; F2P: the red run 39588679012 failed the first and the third of those before the fix; the whole lib 26 passed 0 failed (gh run 39590640299); `cargo clippy -p engine-rome --all-targets -- -D warnings` PASS (gh run 39590220619); golden d57b5c8ae3096614 x2 (gh runs 39590172706 and 39592466625); `node tools/workspace-check.mjs` PASS; `node tools/rust-quality.mjs --owned crates/rome/src/certainty.rs --owned crates/rome/src/lib.rs` PASS. The registry is a const table of 38 numbers in `crates/rome/src/certainty.rs`, each with its tier, a note and its source; the loader refuses a claim with no tier, the renderer asks the registry by key, so a number that is not registered cannot be drawn at all, and the new `LoadError::Unregistered` lives in `crates/rome/src/lib.rs`.
  the game shows with its tier (attested, reconstructed, contested, invented), a
  note and a source, and the loader **refuses an entry with no tier**; the
  renderer asks the registry for a number by key, so a number that is not
  registered **cannot be drawn at all**. The HUD then reads `pila 2.0 m
  (Vegetius 2.15)` with a tier chip, and the contested numbers (the Roman foot
  is 295.7 mm or 297.4 mm) become visible instead of hidden. Donor:
  `dtm988/colosseum_people_flow` (`src/claims.js`, MIT, read 2026-10-21).
  Rows **KIT1**, **CDX8**, and the codex rows of `docs/SPRINT-28-rome.md`.
  Tier A. Size S-M. Owns `crates/rome/src/certainty.rs`, `crates/rome/src/lib.rs`.
  Accept: `cargo test -p engine-rome --lib certainty` passes: a number with no
  tier is refused with the key named, an unregistered key cannot be rendered,
  and the Roman foot is printed with the tier that says it is contested.
  F2P: no certainty registry.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [ ] HX26 render: the crowd on the GPU. A **counting-sort spatial hash** in
  WGSL (clear the cell counters, count the agents, a single-workgroup exclusive
  prefix sum over the cell chunks, scatter the agent indices into a
  cell-contiguous list, then a 3x3 neighbourhood steer with separation,
  alignment, cohesion, goal seeking and obstacle avoidance) makes the 20k
  crowd a **steering** problem on the GPU, while the near ranks stay full
  fighters and the VAT draw stays as it is. Donor: `iodiot/webgpu-crowd`
  (`src/sim/shaders.ts`, `src/sim/CrowdSim.ts`; MIT, read 2026-10-21). The crowd
  stays **outside the fixed tick and outside the golden**: it is presentation.
  Rows the crowd rows of `docs/FOCUS.md` (20k reacting spectators) and the
  chapter VI crowd of `docs/plan/rome.md`.
  Tier B. Size M. Owns `crates/render/src/shaders/crowd_sim.wgsl`,
  `crates/arena/src/crowd_gpu.rs`.
  Accept: `cargo test -p engine-arena --lib crowd_gpu` passes: the counting
  sort places every agent in the cell it belongs to, the cell size is at least
  the neighbour radius, and the same seed gives the same agent positions twice;
  the crowd never enters a tick hash.
  F2P: no GPU crowd; the crowd is CPU-steered or absent.
  P2P: golden `d57b5c8ae3096614` x2; the crowd budget stays inside the frame
  budget.
  needs: none.
- [ ] HX27 ai: morale is a physical fact, not a number. Morale is a hidden state
  whose inputs are physical (the casualty rate, the contact bearings, the anchor
  drift against the order, the incoming charge momentum, the arrows, a nearby
  rout) and whose outputs are physical (bodies flee). It **saturates**, so a
  charge that drops a swath in seconds reels a unit instead of shattering it;
  the drain is multiplied by the nearby enemy strength and divided by the
  nearby friendly strength; the local power share sets a **standing
  equilibrium**; and **crowd pressure is deliberately not an input**, so an
  advancing column never routs from its own deliberate press. The formation
  slot is assigned by projecting the units on the right vector and sorting with
  the unit id as the tiebreak, so the assignment is deterministic. Donor:
  `dzhng/civsim` (`crates/sim/src/morale.rs`, `crates/sim/src/steer/weave.rs`,
  `crates/sim/src/formation_orders.rs`; no licence file, so the **model** is
  re-derived, not the code; read 2026-10-21). The numbers come from Vegetius
  with a `ref`, not from a game's tuning.
  Rows **LEG4** (the shield line fights), **LEG11**, **LEG3**.
  Tier B. Size M. Owns `crates/ai/src/morale.rs`, `crates/ai/src/legion.rs`.
  Accept: `cargo test -p engine-ai --lib morale` passes: a unit that takes a
  heavy loss while the enemy is outmatched reels instead of routing, the same
  loss in an even fight routs, a unit advancing under its own weight does not
  rout, and the slot assignment is the same for two runs of the same seed.
  F2P: no morale state.
  P2P: golden `d57b5c8ae3096614` x2; the rules v1 scene is untouched.
  needs: none.
- [x] HX28 rome: the chapter maps are real ground. The Digital Atlas of the Landed 6c433bed1. `cargo test -p engine-rome --lib geo` 9 passed, 0 failed (run 39597501940): the two tables load with every entry naming a `ref`, a point in the Danube region finds Drobeta within 5 km, the Limes segments carry the names the atlas gives them, and two loads hash the same. The frontier is 51 named lines (the Upper German and Raetian Limes, the Main, the Neckar, the frontier road and the Wall mile markers); the gazetteer is 25 settlements of the six chapter boxes with their Pleiades, Barrington, DARE and Wikidata ids. Clippy `-p engine-rome --all-targets -- -D warnings` PASS (run 39598639281); golden `d57b5c8ae3096614` x2 (runs 39599002285, 39599338633); licence and sha256 of both donors in `content/rome/PROVENANCE.md`.
  Roman Empire gives the **frontier as geometry**: the Upper German and
  Raetian Limes as named, segmented lines (with the Main and the Neckar named
  as rivers), Hadrian's Wall mile markers, the AD 117 provinces, and the
  **building footprint and wall polygon layer of Pompeii** with areas and
  lengths. Donor: `johaahlf/dare` (`fortifications.geojson`, `provinces.geojson`,
  `pompeii_architecture.geojson`, `places2.geojson`; **Apache-2.0**, the cleanest
  licence in the hunt, 5 stars, read 2026-10-21). A second source gives the
  settlements: `Diasito/urbesetorbis` (BSD-2) with the cross-reference ids
  (Pleiades, Barrington, DARE, Wikidata) and the founding year, so a chapter
  area is a **box filter** over the gazetteer.
  Rows **LEG6** (the dig layer), **LEG7** (the camp works), **CDX3** (the
  places), **ROM2**.
  Tier B. Size S. Owns `content/rome/places.toml`, `content/rome/fortifications.toml`,
  `crates/rome/src/geo.rs`.
  Accept: `cargo test -p engine-rome --lib geo` passes: the table loads with
  every entry carrying a `ref`, a point in the Danube region resolves to the
  nearest settlement within 5 km, the Limes segments carry their names, and two
  loads hash the same.
  F2P: no `crates/rome` and no `content/rome`.
  P2P: golden `d57b5c8ae3096614` x2.
  needs: ROM2.
- [ ] HX29 playtest: the referee and the mutant. The build tasks are scored by
  **named predicates with a declared evidence source** (a snapshot, a diff, a
  project file, a runtime probe) and a **fixed severity order**, so the verdict
  names the first failing predicate instead of a bare false. Three honesty rules
  travel with it: **unevaluable is a fail, never a skip**; every referee ships a
  **mutant** (a build broken in exactly one way) that must fail **exactly** its
  named predicate, because an assertion that fails for the wrong reason is not
  discriminating; and the agent's own on-disk state is read **before** the
  runner's save. The Play@k arithmetic is the unbiased estimator with
  **unscored and skipped trials excluded from the denominator**, which is exactly
  our mirror's `UNKNOWN` cell. Donors: `SummerEngine/summer`
  (`evals/outcomes/lib/assert.ts`, 816 lines, `runner.ts`; MIT, 85 stars, read
  2026-10-21) and `everruns/mira` (`crates/mira-eval/src/aggregate.rs`, MIT,
  8 stars) for the arithmetic.
  Rows **D18** (the task suite), **D19** (verifier output in every build verb).
  Gate G7.
  Tier A. Size M. Owns `crates/playtest/src/referee/mod.rs`,
  `crates/playtest/src/referee/criteria.rs`, `crates/playtest/src/referee/aggregate.rs`.
  Accept: `cargo test -p engine-playtest --lib referee` passes: a task with a
  missing evidence source is a fail with the source named, the mutant of each
  criterion fails exactly that criterion, pass@k excludes the unscored cells
  from the denominator, and the verdict names the first failing predicate in
  the fixed order.
  F2P: no referee, no predicates, no pass@k.
  P2P: golden `d57b5c8ae3096614` x2; the 30-task mirror keeps its `UNKNOWN`
  cells and its parity numbers.
  needs: none.
- [ ] HX30 endpoint: the verdict that cannot be forged. The door's `pass` stops
  being a string anyone can write: it becomes a value with **no public
  constructor**, minted only by a checker run that actually ran and read an exit
  status, carrying the digest of the inputs it was handed, and answerable to
  "is this verdict about the artefact in my hand". A build verb that did not
  run its referee returns "not run", never "pass"; a missing key counts as
  **-inf and aborts**, never as zero. Donor: `fabio-rovai/open-ontologies`
  (`src/verdict.rs`, 35K, `src/feedback.rs`; MIT, 1158 stars, read 2026-10-21).
  The type must be ours: our own digest (FNV-1a 64), our own checker process.
  Rows **D19**, **D18**; gates G7 and G1.
  Tier A. Size M. Owns `crates/endpoint/src/verdict.rs`, `crates/endpoint/src/lib.rs`.
  Accept: `cargo test -p engine-endpoint --lib verdict` passes: a verdict cannot
  be built without a checker run, a verdict about a different input is refused
  by `is_about`, a build verb with no referee returns "not run", and a missing
  criterion aborts instead of scoring zero.
  F2P: `pass` is a string in the reply.
  P2P: golden `d57b5c8ae3096614` x2; the existing door tests keep their shape.
  needs: none.
- [x] HX31 proof: the differential ship check. Landed 583ff9bb. `cargo test -p engine-proof --lib differential` 17 passed (run 39575557613); clippy `-p engine-proof --all-targets -- -D warnings` PASS (run 39575534607); golden `d57b5c8ae3096614` x2 (runs 39575583574, 39576754570). The job in `.github/workflows/differential.yml` ran on the public mirror and passed (run 39579398044): the two receipts match step for step, and the golden row is in both. A curated scenario list runs Landed 40c204954. The committed scenario list is now read by a test, not by CI alone: `cargo test -p engine-proof --lib differential` 19 passed, and a typo in `fixtures/differential/scenarios.txt` fails a unit test instead of a build. Landed a4405e051. The deep tier had no automatic slot at all: nothing ever built `spine_parity_line` or `ragdoll_line` on a runner, so the deep tier could be broken and CI would stay green. The workflow now has a weekly leg (Monday 04:17 UTC) that plays the deep tier, and the shape of the workflow is held by `tools/differential.test.mjs` (8 tests pass). Landed 9d32b1fe7. The manual and the pull request legs of the job were dead: the guard read `github.event.before` on every event, and that field only exists on a push (on the others it is null, and GitHub casts null and a string to a number, so null and the zero string are the same number and the guard was false). On the mirror the manual leg had never run once, so the deep tier could not be reached by hand at all. The guard now reads the before commit only where it exists. The deep tier ran on a Windows runner for the first time and passed: `DIFFERENTIAL clean tier=deep steps=2` with `--features arena,spine,ragdoll` (run 39640204582).
  against the **reference** build and the **candidate** build, and every step of
  the receipt is diffed with a normalization list (timestamps, ids, paths),
  where a real behaviour change must still show: a random value is pinned by
  seed rather than normalized away. Exit 0 means every in-scope step matched,
  1 means one divergence. Two tiers: a fast curated set on every change and a
  deep catalog off by default. Donor: `gastownhall/beads`
  (`tests/oracle-a/harness/src/differential.rs`, 30K, `tests/oracle-a/README.md`;
  MIT, 28056 stars, read 2026-10-21). We take the **shape**, because our
  receipts are our own line format, not JSON.
  Row **E22** (the Windows ship check from CI); gate G1.
  Tier A. Size M. Owns `crates/proof/src/differential.rs`,
  `crates/proof/src/bin/differential.rs`, `fixtures/differential/scenarios.txt`,
  `.github/workflows/differential.yml`.
  Accept: `cargo test -p engine-proof --lib differential` passes: two runs of
  the same build give the same normalized receipt, a seeded change shows up as a
  divergence and is not normalized away, and the tool exits 1 on one in-scope
  divergence.
  F2P: no ship check.
  P2P: golden `d57b5c8ae3096614` x2; the golden matrix workflow is untouched.
  needs: none.
- [ ] HX32 endpoint: the door's contract is a committed file. The tool surface
  is snapshotted into a committed lockfile, and a change is classified the way
  a human would (breaking, minor, patch) with a **direction-aware** table: the
  same structural change has an opposite blast radius for what a caller sends
  and what a caller receives. The lint adds the rules that matter for agents (a
  duplicate tool name is an error, a missing description is a warning, because
  the model cannot tell when to call it). The published schema is **generated**
  from the live tool list, so it cannot drift. Donors:
  `studiomeyer-io/mcp-covenant` (`src/diff.rs`, `src/lint.rs`, `src/lockfile.rs`;
  MIT, read 2026-10-21) and `xerj-org/xerj`
  (`engine/crates/xerj-mcp/tests/published_schema_drift.rs`; Apache-2.0, 12423
  stars) for the drift test.
  Rows **D16** (the door's published contract), **D19**.
  Tier B. Size M. Owns `crates/endpoint/src/contract.rs`, `docs/door-tools.json`,
  `tools/door-contract.mjs`.
  Accept: `cargo test -p engine-endpoint --lib contract` passes: a new required
  input field is classified breaking, a removed output field is classified
  breaking, a new enum value is minor for input and breaking for output, and
  the committed file is byte-equal to the live tool list.
  F2P: no committed contract and no drift test.
  P2P: golden `d57b5c8ae3096614` x2; the 33 shipped prompt skills still list.
  needs: none.
- [ ] HX33 game: the debug kit. The window gets **cvars** (a flat registry of
  named settings with a typed value, a default and a help string, read by name
  from a string while the game still reads them as ordinary fields) and a
  **console** (a command parser trait so the engine owns the parser and the UI
  owns the text, a lexer that splits the word from the arguments, and a hint
  that carries a span into the input, so a bad command points at the wrong
  characters). The naming convention is the useful part: a prefix by owner
  (`cl_`, `d_`, `g_`, `hud_`, `r_`, `sv_`, `sys_`) and a `d_draw_*` group for
  the debug draw that already has a trait and no caller. Donors:
  `doonv/bevy_dev_console` (`src/command.rs`, `src/builtin_parser/lexer.rs`;
  Apache-2.0, 139 stars) and `martin-t/cvars` (`cvars/src/lib.rs`; AGPL-3.0, so
  the **shape and the naming only** are taken and the bytes are not).
  Row **C13** (the debug kit in the game and on the door); gates G1 and G2.
  Tier B. Size M. Owns `crates/game/src/cvar.rs`, `crates/game/src/console.rs`,
  `crates/render/src/debug_draw.rs`.
  Accept: `cargo test -p engine-game --lib cvar` passes: a cvar is set by name
  from a string, an unknown name is refused with the name, a cvar with a help
  string lists it, and **no cvar read happens inside the fixed tick** (the test
  asserts the cvar table is only read on the service path).
  F2P: no cvar, no console, and `debug_draw.rs` has no caller.
  P2P: golden `d57b5c8ae3096614` x2; the pause and step keys still work.
  needs: none.
- [ ] HX34 endpoint: the build verb returns the build. `project.build` runs the
  **real** build and answers with the build's own output (status, message,
  stdout, stderr) as structured content, not a summary of it. A `project.run`
  verb starts the game and returns its first receipt line. Donor:
  `dojoengine/dojo` (`crates/sozo/mcp/src/tools/build.rs`,
  `crates/sozo/mcp/src/lib.rs`; Apache-2.0, 478 stars, read 2026-10-21), whose
  rmcp server is a complete door in one file: five typed tools, resource
  templates with `{profile}/{name}` placeholders, a typed `resource_not_found`,
  and every failure carrying a **code and a reason payload** (our typed-refusal
  shape). `project.new` is the part we still owe a story for: a `new` that
  writes a real playable project is more than a scaffold.
  Row **C12** (`engine2040 new / build / run` plus the door verbs); gate G7.
  Tier A. Size M. Owns `crates/endpoint/src/packs/project.rs`,
  `crates/endpoint/src/packs/mod.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib packs::project`
  passes: `project.build` returns the build's own stdout in the reply, a
  failing build returns a typed refusal with the failing line, `project.run`
  returns the first receipt line, and an unknown resource is a typed
  `resource_not_found`.
  F2P: no `project.*` verb.
  P2P: golden `d57b5c8ae3096614` x2; the door keeps its 33 prompt skills.
  needs: none.
- [ ] HX35 playtest: the knob sweep comes out of the test module. The knob
  sweep is **already written** but behind `#[cfg(test)]`, so it never runs in a
  build and no verb reaches it: `crates/playtest/src/tournament.rs:1490-1700`
  holds `SWEEP_KNOB`, `SweepConfig`, `SweepReport`, `sweep_difficulty_curve` and
  `render_sweep_csv`, and the CSV already prints
  `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials`
  with `# skipped` comment rows. The work: promote the block out of the test
  module, add a `tune.sweep` verb over it (one knob, a range, 5 fixed seeds),
  return the rows and the CSV bytes, and land the winning value as one
  `settings.set` plus one `settings.undo` so the tape stays replayable. No
  donor needed: this is a re-land of our own code. Row **D20** (tune.sweep).
  Tier A. Size S. Owns `crates/playtest/src/tournament.rs`,
  `crates/endpoint/src/packs/sweep.rs`, `crates/endpoint/src/packs/mod.rs`.
  Accept: `cargo test -p engine-playtest --lib sweep` and
  `cargo test -p engine-endpoint --features mcp --lib packs::sweep` pass:
  `tune.sweep` over one knob and 5 fixed seeds returns the CSV bytes, the same
  knob and seeds give the same CSV twice, the knob may be any name in the
  `settings.get` table, and a bad knob name is a typed refusal. The sweep reads
  no clock and no unseeded RNG.
  F2P: the sweep exists only inside the test module and no `tune.sweep` verb
  answers.
  P2P: golden `d57b5c8ae3096614` x2; the existing `play.sweep` verb keeps its
  behaviour.
  needs: none.
