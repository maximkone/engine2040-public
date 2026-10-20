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
- [ ] HX6 rome: the codex pipeline. A tool reads the two public-domain sources
  (Vegetius *De re militari* I, Pseudo-Hyginus *De munitionibus castrorum*) and
  prints the numbers they carry next to `content/rome/camp.toml` and
  `content/rome/line.toml`, so a changed number in the table shows as a diff against
  the source. The passages and the conversions are already written down in
  `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md` (25 numbers with
  chapter refs, 1 Roman foot = 296 mm).
  Tier A. Size S. Owns `tools/codex-check.mjs`, `content/rome/camp.toml`,
  `content/rome/line.toml`, `docs/research/hunt-2026-10-21/07-codex-vegetius-hyginus.md`.
  Accept: `node tools/codex-check.mjs` prints `codex: numbers=25 unmatched=0` and
  exits 0; a table value that no passage supports is printed as `unmatched` and
  exits 1; the ditch row reads 1480 mm wide and 888 mm deep with `ref = "PHG 49"`.
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
- [ ] HX10 tools: the ship build is reproducible. A CI job builds the Windows ship
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
- [ ] HX12 ci: the test report a stranger can read. The CI publishes a structured
  test report (the CTRF JSON shape) for the door and playtest suites, so a run's
  result is read without opening the log. Donor: `ctrf-io/github-test-reporter`
  (MIT, 378 stars, pushed 2026-10-11).
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
- [ ] HX15 web: the frame bus. The sim state crosses to the draw thread over a
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
- [ ] HX16 net: the desync locator. A Merkle tree over the entity state finds a
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
