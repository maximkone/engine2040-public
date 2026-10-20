# Proposed rows 15 — layered architecture (LYR)

One paragraph. The engine's layer rules live in `docs/ARCHITECTURE.md` §1 (the
seven-level diagram) and are machine-checked at crate granularity only, in
`tools/workspace-check.mjs` (`allowedEdges`, lines 19-102). Everything below
crate granularity is unwritten: no rule that a simulation crate may not import
a presentation crate, no rule that the GPU may only be touched under
`crates/render/src/present/**` (31 render files outside `present/` name
`wgpu::`), no rule that a tick module may not do IO. The horizontal axes are
worse: the axis data exists (collision layers, layered navmesh, pass report,
quality knobs, LOD pick, job pool) and almost none of it is wired — `pass_report`
has zero callers, `engine_nav::layers` had zero callers, `pixel_thresholds_px` is
`None` in every constructor, and the input layer stack says "No live consumer
yet" in its own source. This file proposes twelve rows: three of them land in
this lane (the job pool in the sim, the nav consumer, the input layer), two are
patch-ready specs for the held render crate, and four are the missing
workspace-check rules.

## The layer map (kept)

```
 L7 apps          engine2040(bin) engine-perf engine-viewer kam2-game kam2-web
 L6 door+editor   engine-endpoint engine-editor engine-app engine-provider engine-playtest
                  engine-proof engine-distill engine-rules engine-ossim engine-chaos
 L5 gameplay      engine-arena engine-ai engine-combat engine-input engine-nn engine-world
                  engine-game (the only crate that owns a window)
 L4 tick core     engine-core (std-only: tick, seed, FNV hash) engine-ecs (life, phases, JobPool)
 L3 simulation    engine-physics engine-physfx engine-destruction
                  engine-nav (door-only leaf: engine-endpoint, before this lane)
 L2 presentation  engine-render engine-ui engine-audio engine-anim (also read by sim crates)
 L1 core services engine-scene engine-spatial
 L0 platform      engine-clock engine-net engine-web engine-assets engine-save engine-observe
```

After this lane: `engine-nav` gains a caller in L5 (`engine-arena`), and
`engine-anim` is re-read as L1.5 **data** (see "The anim question" below), not
L2 presentation.

## The five horizontal gaps: the one row that owns it, the one command that proves it

| Gap | The one row that owns it | The one command that proves it |
|---|---|---|
| Nav layers: `crates/nav/src/layers.rs` landed, zero consumers | LYR-1 | `node tools/gh-check.mjs cargo test -p engine-arena --lib nav_layers` prints `3 passed` |
| Render pass order: `pass_report::Plan::validate` has zero callers | LYR-5 | `node tools/gh-check.mjs cargo test -p engine-render --lib pass_report` prints `6 passed` and `pass_report_live_frame_names_every_pass` is green (spec only: render is held by GS44) |
| Feature ladder: two tables (`quality.rs`, `tier.rs`), no three-mode ladder | LYR-6 | `node tools/gh-check.mjs cargo test -p engine-render --lib ladder` prints `5 passed` (spec only: render is held) |
| Physics islands / job pool: `step_islands` is not in the tree, no sim caller | LYR-2 + LYR-3 | `node tools/proof.mjs --skip-build` with `E2040_THREADS=1` and `=8` both print golden `d57b5c8ae3096614` |
| Input layers: `ActionSetStack` has no live consumer | LYR-4 | `node tools/gh-check.mjs cargo test -p engine-input --lib layers` prints `3 passed` and `node tools/gh-check.mjs cargo test -p engine-arena --lib input_layers` prints `1 passed` |

## Proposed rows

### LYR-1 — nav: give the layered navmesh one caller in the sim lane

- **What it is.** `crates/nav/src/layers.rs` bakes several walkable spans per
  column (`MAX_SPANS_PER_COLUMN = 16`, `E2LAY2` bytes, `MESH_VERSION = 2`) and
  nothing outside `nav` read it. This row bakes the match floor into that table
  inside the arena and asks it, after every live tick, which walk surface each
  fighter stands on.
- **Donor.** recastnavigation/recastnavigation, `Recast/Source/RecastLayers.cpp`
  and `Recast/Include/Recast.h`, https://raw.githubusercontent.com/recastnavigation/recastnavigation/main/Recast/Source/RecastLayers.cpp
  and https://raw.githubusercontent.com/recastnavigation/recastnavigation/main/Recast/Include/Recast.h,
  read 2026-10-20. Licence **Zlib** (`License.txt` read live 2026-10-20).
  Read live: `rcBuildHeightfieldLayers` (line 104 of the .cpp), `RC_MAX_LAYERS_DEF 63`,
  layer ids stored as a byte, and "layer overflow (too many overlapping walkable
  platforms)". Port the shape only; write our own integer-mm code.
- **Closes.** UW12, UW40 (the code half landed; the consumer half did not).
- **Owned paths.** `crates/arena/src/nav_layers.rs` (new), `crates/arena/src/live.rs`,
  `crates/arena/src/lib.rs`, `crates/arena/Cargo.toml`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-arena --lib nav_layers`
  prints `3 passed`; `nav_layers_two_floors_keep_two_spans_in_one_column` shows one
  column with two spans and two walk surfaces; the golden stays
  `d57b5c8ae3096614` (`node tools/proof.mjs --skip-build` prints it unchanged).
- **Kill line.** If `engine-arena` may not take `engine-nav` (the lead refuses the
  one-line `allowedEdges` change), the row dies and the consumer moves to the door.
- **Cost.** M.

### LYR-2 — ecs+arena: the shared job pool with a thread-count switch

- **What it is.** `JobPool::shared()` was crate-private, so the sim could not use
  it, and the pool sized itself from the core count only. This row makes the
  shared pool public, sizes it from `E2040_THREADS` (pure rule, unit tested), and
  runs the arena's fork worker and its gym branch fan-out on that pool instead of
  spawning a thread per branch.
- **Donor.** rust-lang/rayon, `README.md` ("join and scope", custom pools),
  https://raw.githubusercontent.com/rayon-rs/rayon/master/README.md, read
  2026-10-20. Licence **MIT OR Apache-2.0** (`LICENSE-MIT` read live 2026-10-20).
  Reading donor only: the engine already owns `ecs::parallel::fork_join_on`.
- **Closes.** E21 (finish-line gap 21, "job pool in the sim"), RC36.
- **Owned paths.** `crates/ecs/src/pool.rs`, `crates/arena/src/fork.rs`,
  `crates/arena/src/gym.rs`, `crates/arena/Cargo.toml`.
- **Accept.** `node tools/proof.mjs --skip-build` with `E2040_THREADS=1` and
  `E2040_THREADS=8` both print golden `d57b5c8ae3096614`;
  `node tools/gh-check.mjs cargo test -p engine-ecs --lib job_pool_threads` prints
  the pool suite green; `RUSTFLAGS="--cfg loom" cargo test -p engine-ecs --lib loom_pool`
  prints the scope model green (the loom model itself landed as CQ17, commit
  a811a709 "ecs: CQ17 a loom model of JobPool::scope").
- **Kill line.** If the golden moves at any worker count, or if the arena's fork
  bytes differ between 1 and 8 workers, the row dies and the fork keeps its own
  thread.
- **Cost.** M.

### LYR-3 — arena: the per-fighter step on the pool, merged in fighter order

- **What it is.** The recorded fork path built one dataset row per fighter per
  tick in a serial loop. This row builds the rows per fighter on the shared pool
  and merges the partials in fighter order, so the bytes are the ones the serial
  loop wrote whatever the worker count.
- **Donor.** rust-lang/rayon, `README.md` (parallel iterators, custom pools),
  https://raw.githubusercontent.com/rayon-rs/rayon/master/README.md, read
  2026-10-20. Licence **MIT OR Apache-2.0**. Shape only.
- **Closes.** RC36 ("run the per-fighter step through one shared pool with results
  merged in fighter order"), E21.
- **Owned paths.** `crates/arena/src/fork.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-arena --lib fork` prints
  the fork suite green and the dataset rows of two runs are byte-equal;
  `E2040_THREADS=1` and `=8` give the same dataset bytes.
- **Kill line.** If the recorded rows differ in order or bytes between worker
  counts, the row dies and the loop stays serial.
- **Cost.** S.

### LYR-4 — input: hash the input layer beside the tape

- **What it is.** `ActionSetStack` (Duel / Menu / Pause) had no live consumer, so
  a menu-gated tick and an idle duel tick looked the same to the sim. This row
  adds `engine_input::layers::LayerTape` (the tape plus the layer of each tick,
  FNV-1a 64 digest over the layer rows) and an arena reader
  (`input_layers::run_layered`) that returns the checkpoints plus the layer digest.
- **Donor.** no external donor: the digest is `engine_core::Fnv64`, the same
  owner as every other engine hash. Rule source: `docs/HASH-LAW.md` (the golden
  `d57b5c8ae3096614` is byte-frozen).
- **Closes.** new row **IN-1** (the input layer). No board row today: write
  "IN-1 input: the input layer is hashed beside the tape".
- **Owned paths.** `crates/input/src/layers.rs` (new), `crates/input/src/bindings.rs`,
  `crates/input/src/lib.rs`, `crates/arena/src/input_layers.rs` (new),
  `crates/arena/src/lib.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-input --lib layers` prints
  `3 passed`; `node tools/gh-check.mjs cargo test -p engine-arena --lib input_layers`
  prints `1 passed`; the golden stays `d57b5c8ae3096614` because the layer never
  enters the tick hash.
- **Kill line.** If the layer ever has to enter the tick hash (owner call), this
  row dies and is replaced by a tape format version bump plus a golden migration.
- **Cost.** S.

### LYR-5 — render: the pass plan is the live frame's plan (SPEC ONLY, render held)

- **What it is.** `crates/render/src/pass_report.rs` has the right rule (a read
  with no writer fails the plan) and zero callers; the live frame encodes its
  passes imperatively in `present/gpu_present/submit.rs`, and `arena_frame_graph()`
  is used only in a test. This row makes the live frame declare its passes as a
  `Plan` and fail the frame when a read has no writer.
- **Donor.** Themaister/Granite, `renderer/render_graph.cpp`
  (`void RenderGraph::validate_passes()`, line 562),
  https://raw.githubusercontent.com/Themaister/Granite/master/renderer/render_graph.cpp,
  read 2026-10-20. Licence **MIT** (`LICENSE` read live 2026-10-20).
  Correction to GS45: Granite checks input/output shape parity and dimension
  match, **not** "a read with no writer"; that rule is ours and stays ours.
  Second donor: uinosoft/zenfg, `crates/zenfg/src/{graph.rs,pass.rs,report.rs}`,
  https://raw.githubusercontent.com/uinosoft/zenfg/main/crates/zenfg/src/pass.rs,
  read 2026-10-20, licence **MIT** (`LICENSE` read live 2026-10-20).
- **Closes.** GS45.
- **Owned paths.** `crates/render/src/pass_report.rs`, `crates/render/src/present/gpu_present/submit.rs`,
  `crates/render/src/present/gpu_present/presenter.rs`, `crates/render/src/graph.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib pass_report`
  prints `6 passed` plus the new `pass_report_live_frame_names_every_pass`; the
  manifest's `passes` line names every pass the frame ran or culled; the golden
  stays `d57b5c8ae3096614`.
- **Kill line.** If the live frame cannot be described as data without touching
  the hashed sim path, the row dies and the report stays a tool.
- **Cost.** M.

### LYR-6 — render: one feature table with three modes (SPEC ONLY, render held)

- **What it is.** `quality.rs` (four tiers, knobs, cut order) and `tier.rs`
  (the RT0 lighting tier) are two tables, and the device asks only
  `TIMESTAMP_QUERY`. This row makes one table of optional features with three
  modes (none, preprocess-only, culling), requests exactly the features the
  adapter has, stamps the mode in the manifest and on the passes line, and adds
  the `E2040_LADDER=none` floor.
- **Donor.** google/dawn, `src/dawn/native/Features.h` (one `Feature` to
  `FeatureInfo` table, `FeaturesBitSet`, `ToSupportedFeatures`),
  https://raw.githubusercontent.com/google/dawn/main/src/dawn/native/Features.h,
  read 2026-10-20. Licence **BSD-3-Clause** (`LICENSE` read live 2026-10-20).
- **Closes.** GS35.
- **Owned paths.** `crates/render/src/quality.rs`, `crates/render/src/tier.rs`,
  `crates/render/src/present/gpu_present/device_init.rs`, `crates/render/src/manifest.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib ladder` prints
  `5 passed`; the manifest carries the mode and the passes line does too; the golden
  stays `d57b5c8ae3096614`.
- **Kill line.** If the three modes cannot be told apart by adapter facts alone
  (no timeouts query, no preprocess support), the row dies and the tiers stay as
  they are.
- **Cost.** S.

### LYR-7 — physics: the collision channel matrix as data (SPEC, physics held)

- **What it is.** `crates/physics/src/layers.rs` is a 5-row Rust const table with
  one layer bit per collider and a symmetric "blocks" rule; destruction and
  spatial carry no layer at all. This row makes the matrix data (a group plus a
  mask per collider) and gives the destruction chunks and the spatial reports a
  layer.
- **Donor.** bulletphysics/bullet3,
  `src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h`
  (`CollisionFilterGroups` enum plus the `m_collisionFilterGroup` /
  `m_collisionFilterMask` pair),
  https://raw.githubusercontent.com/bulletphysics/bullet3/master/src/BulletCollision/BroadphaseCollision/btBroadphaseProxy.h,
  read 2026-10-20. Licence **Zlib** (`LICENSE.txt` read live 2026-10-20).
- **Closes.** new row **CX-1** (the channel matrix as data); CQ20 for the one-owner
  half (`Handle(pub u32)`, `Fixed(pub i64)`).
- **Owned paths.** `crates/physics/src/layers.rs`, `crates/physics/src/collider.rs`,
  `crates/destruction/src/lib.rs`, `crates/spatial/src/lib.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-physics --lib layers` prints
  the layer suite green; `rg -n "struct Handle(pub|struct Fixed(pub" crates` prints
  nothing; the golden stays `d57b5c8ae3096614`.
- **Kill line.** If the symmetric 5-row table answers every pair the game ships,
  the override half dies and only the data half stays.
- **Cost.** M.

### LYR-8 — render: LOD by projected pixel error, armed (SPEC, render held)

- **What it is.** `draw.rs` has `lod_by_pixel_error` and `DrawTable.pixel_thresholds_px`,
  but every constructor sets the thresholds to `None`, the cook stores no per-LOD
  error, and the live loop never arms it. This row stores the per-LOD error in the
  cook record and arms the live pick.
- **Donor.** meshoptimizer clusterlod (already mined: the row itself names it).
  Correction only. The glTF draft `EXT_mesh_lod` README
  (https://raw.githubusercontent.com/KhronosGroup/glTF/main/extensions/2.0/EXT_mesh_lod/README.md)
  **404s today (2026-10-20)**.
- **Closes.** RL61.
- **Owned paths.** `crates/render/src/draw.rs`, `crates/geo/src/lod.rs` (the cook record).
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib lod_pixel` prints
  the pixel-error suite green; FOV 20 and FOV 60 pick different LODs at one
  distance; a 1080p to 540p step switches one LOD sooner; the golden stays
  `d57b5c8ae3096614`.
- **Kill line.** If the 1080p to 540p test cannot be made deterministic, the row
  dies and the distance bands stay.
- **Cost.** S.

### LYR-9 — render: the GPU cull on indirect draws (SPEC, render held)

- **What it is.** `gpu_cull.rs` (GS18 slice 1) computes the visible set and the
  5-u32 indirect args, and no draw path uses them; the CPU cull is the only path.
  This row reads the indirect args in the static instance draws, with
  `E2040_CPU_CULL=1` as the fallback.
- **Donor.** no new donor: wgpu `draw_indexed_indirect` (wgpu already mined), and
  the tree already holds the cull shader and the args builder.
- **Closes.** GS18b.
- **Owned paths.** `crates/render/src/draw.rs`, `crates/render/src/gpu_cull.rs`.
- **Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib gpu_cull` prints
  the cull suite green; the frame's draw count drops with the GPU cull on and the
  `E2040_CPU_CULL=1` frame matches the CPU path; the golden stays
  `d57b5c8ae3096614`.
- **Kill line.** If the GPU and CPU paths disagree on one frame, the row dies and
  the CPU cull stays.
- **Cost.** M.

### LYR-10 — workspace-check: a simulation crate may not import a presentation crate (NEW RULE)

- **What it is.** The crate-edge allowlist forbids unknown edges but never says
  what a layer may read. `engine-physics` and `engine-arena` both import
  `engine-anim`, which looks like a presentation import and is not one (see the
  anim question below). This row writes the rule down and checks it.
- **Donor.** SanderMertens/flecs, `include/flecs/addons/pipeline.h` (a pipeline is
  a set of tags on systems; phases are tags),
  https://raw.githubusercontent.com/SanderMertens/flecs/master/include/flecs/addons/pipeline.h,
  read 2026-10-20. Licence **MIT** (`LICENSE` read live 2026-10-20). Reading donor
  only: the rule text is ours.
- **Closes.** the crate-layer rules in `docs/ARCHITECTURE.md`; new row **LY-1**.
- **Owned paths.** `tools/workspace-check.mjs` (lead applies; the speedup lane may
  be in that file), `docs/ARCHITECTURE.md` (lead applies).
- **Accept.** `node tools/workspace-check.mjs` prints
  `PASS layer rules: no simulation crate imports a presentation crate`.
- **Kill line.** If any simulation crate really needs a presentation type, the
  rule dies for that crate and the exception is written in the same map.
- **Cost.** S.

### LYR-11 — workspace-check: the GPU boundary (NEW RULE)

- **What it is.** 31 files under `crates/render/src/` name `wgpu::` outside
  `present/**`, so the GPU boundary is a directory name, not a rule. This row
  writes the rule down as a list first (report-only), then fails a new file
  outside `present/**` that names `wgpu::`.
- **Donor.** none needed: the rule is the engine's own `docs/ARCHITECTURE.md` §1
  row for `render` ("Presentation snapshots and resources; no authoritative
  feedback from frame timing") read 2026-10-20.
- **Closes.** the crate-layer rules in `docs/ARCHITECTURE.md`; new row **LY-2**.
- **Owned paths.** `tools/workspace-check.mjs` (lead applies), `docs/ARCHITECTURE.md`
  (lead applies).
- **Accept.** `node tools/workspace-check.mjs` prints
  `WARN gpu boundary: 31 files outside present/ name wgpu:: (report-only)` and
  `PASS gpu boundary: no new file outside present/ names wgpu::`.
- **Kill line.** If the 31 files cannot be moved into `present/**` without
  breaking the render crate's public API, the rule stays report-only.
- **Cost.** M.

### LYR-12 — workspace-check: the IO boundary per module (NEW RULE)

- **What it is.** The IO boundary is enforced only by crate edges; a module
  inside a simulation crate could open a file and nothing would notice. Today
  the only `std::fs` hits in the simulation crates sit in `#[cfg(test)]`
  modules. This row scans the production code of the simulation crates for IO.
- **Donor.** none needed: the rule is `docs/ARCHITECTURE.md` §1 ("IO/import stays
  outside the tick") read 2026-10-20, and the scan reuses
  `tools/sim-source-scan.mjs` `productionCode(src)`.
- **Closes.** the crate-layer rules in `docs/ARCHITECTURE.md`; new row **LY-3**.
- **Owned paths.** `tools/sim-source-scan.mjs`, `tools/workspace-check.mjs` (lead
  applies).
- **Accept.** `node tools/workspace-check.mjs` prints
  `PASS io boundary: no simulation crate does IO outside its tests`.
- **Kill line.** If a simulation crate needs IO in production (a save file in the
  loop), the rule dies for that crate and the exception is written down.
- **Cost.** S.

## The anim question: is `engine-anim` presentation or data?

**Verdict: `engine-anim` is a data crate, not a presentation crate.** It sits
at **L1.5, between the core services and the simulation**, and the map in this
file should say so.

Why (all read today):

1. `docs/ARCHITECTURE.md` C4 says "anim owns skeleton/clip/pose/root-motion
   contracts" and "Render consumes cooked geometry/poses". A contract owner is
   not a view. The crate's own job line says "SoA clip sampler + blend + quant.
   Copies [f32;3] out as Position, never shared types" (`docs/CRATE-MAP.md`).
2. The authoritative sim reads it: `crates/arena/src/lib.rs` calls
   `engine_anim::pose::extract_root` for the root motion that the tick hash
   folds, and `engine_physics` and `engine_physfx` take `engine-anim` for the
   ragdoll and pose types. A presentation crate the hash depends on is a
   contradiction.
3. It has no GPU, no clock, no IO, no provider: its dependencies are
   `engine-core`, `engine-combat`, `engine-ecs`, `engine-nn` and (dev) `engine-nav`.

What has to move: **nothing in the code**. The crate's dependencies, its
position in the map, and the `allowedEdges` row are already right. What moves is
the *label* in `docs/ARCHITECTURE.md` (the `render, audio, anim, ui` row of the
§1 table) and the layer map. Split the row:

```
| `anim` | Data contracts (skeleton, clip, pose, root motion, mask tables): std-only, no GPU, no clock, no IO; the simulation may read it, the renderer draws from it |
| `render`, `audio`, `ui` | Presentation snapshots and resources; no authoritative feedback from frame timing; no simulation crate may import them |
```

Exact crates affected by the split: `engine-anim` (moves from the presentation
row to the data row), `engine-render`, `engine-ui`, `engine-audio` (stay in the
presentation row). Crates that read anim and stay legal: `engine-physics`,
`engine-physfx`, `engine-arena`, `engine-assets`, `engine-endpoint`,
`engine-game`, `engine-proof`. Crates that become illegal under the new rule
(none today): every simulation crate importing `engine-render`, `engine-ui`,
`engine-audio`, `engine-clock`, `engine-web`, `engine-net`, `engine-assets`,
`engine-save` or `engine-observe`.

Rule text for the workspace check (lead applies, `tools/workspace-check.mjs`):

```js
// Layer rule LY1 (the layer map): a simulation crate may not import a
// presentation crate. engine-anim is NOT presentation: it owns data contracts
// (skeleton, clip, pose, root motion, mask tables), so engine-physics,
// engine-physfx and engine-arena may read it.
const PRESENTATION_CRATES = new Set([
  "engine-render",
  "engine-ui",
  "engine-audio",
]);
const PLATFORM_CRATES = new Set([
  "engine-clock",
  "engine-web",
  "engine-net",
  "engine-assets",
  "engine-save",
  "engine-observe",
]);
const SIMULATION_CRATES = new Set([
  "engine-physics",
  "engine-physfx",
  "engine-destruction",
  "engine-nav",
  "engine-arena",
  "engine-combat",
  "engine-ai",
  "engine-input",
  "engine-rules",
]);
```

and, inside the same audit loop that checks the crate edges
(`tools/workspace-check.mjs`, the `for (const pkg of workspacePackages)` block
that builds `allowed` from `allowedEdges`):

```js
  if (SIMULATION_CRATES.has(pkg.name)) {
    for (const dependency of pkg.dependencies) {
      if (PRESENTATION_CRATES.has(dependency.name) || PLATFORM_CRATES.has(dependency.name)) {
        errors.push(
          `${pkg.name} -> ${dependency.name}: a simulation crate may not import a presentation or platform crate (LY1)`
        );
      }
    }
  }
```

The rule passes today with zero breakage, which is the cheap fix: the collision
lane can add a channel matrix under `crates/physics` without the layering
question blocking it.

## Patch-ready specs (STEP 3)

### GS45 — the pass plan is the live frame's plan (render is held by GS44)

- **Files.** `crates/render/src/pass_report.rs` (the plan type stays),
  `crates/render/src/present/gpu_present/submit.rs` (the live encode),
  `crates/render/src/present/gpu_present/presenter.rs` (the frame receipt),
  `crates/render/src/graph.rs` (`FrameGraph::compile`).
- **Function names.** In `pass_report.rs`: keep `Plan`, `Plan::validate`,
  `Plan::report_json`, `Plan::report_line`, `ReportError::MissingWriter`. Add
  `pub fn live_frame_plan(quality: &crate::quality::Knobs) -> &'static Plan`
  (one table of the frame's passes: sky, IBL, sun shadow, GTAO, bloom, TAA and
  the culled ones, each with `reads` and `writes`).
  In `present/gpu_present/submit.rs`: add
  `fn frame_passes(plan: &'static Plan) -> Vec<&'static str>` — the passes the
  live encode actually ran, read from the same table, so the report cannot drift
  from the frame.
  In `graph.rs`: `FrameGraph::compile` gains the missing-writer check: a pass
  whose input has no earlier writer returns
  `GraphError::MissingWriter { pass, resource }` (new variant beside `Cycle`,
  `UnknownPass`, `BadLifetime`).
- **Test names.** `pass_report_live_frame_names_every_pass` (the live plan
  validates and its report names sky, IBL, sun shadow, GTAO, bloom and TAA as
  `ran`, plus one disabled pass as `culled`), and
  `graph_compile_refuses_a_read_with_no_writer`.
- **Proof command.** `node tools/gh-check.mjs cargo test -p engine-render --lib pass_report`
  prints `7 passed` (6 existing plus the new one), and
  `node tools/gh-check.mjs cargo test -p engine-render --lib graph` prints
  `1 passed` for the new compile check.
- **The golden must stay** `d57b5c8ae3096614`: the plan is presentation only; no
  sim byte changes.

### GS35 — one feature table with three modes (render is held by GS44)

- **Files.** `crates/render/src/quality.rs` (the table),
  `crates/render/src/tier.rs` (the tier probe; keep it, but it reads the same
  table), `crates/render/src/present/gpu_present/device_init.rs` (the device
  request), `crates/render/src/manifest.rs` (the stamp).
- **Function names.** In `quality.rs`: add
  `pub enum LadderMode { None, PreprocessOnly, Culling }`,
  `pub const FEATURES: &[(&str, &str)]` (one table: feature name, the adapter
  fact it needs), `pub fn ladder_mode(caps: &AdapterCaps) -> LadderMode` (pure
  function over fake adapter caps), and `pub fn requested_features(mode: LadderMode,
  caps: &AdapterCaps) -> wgpu::Features` (exactly the features that exist).
  In `device_init.rs`: the device request uses `requested_features` instead of
  `adapter.features() & wgpu::Features::TIMESTAMP_QUERY`.
  In `manifest.rs`: add `pub fn stamp_ladder(manifest: &mut CaptureManifest, mode:
  LadderMode)` — the mode rides the manifest and the passes line.
- **Test names.** `ladder_mode_picks_the_floor_without_preprocess` (a GL-like
  adapter gets `None`), `ladder_mode_asks_only_for_the_features_that_exist`
  (a 3050-like adapter validates with zero errors), `ladder_env_none_forces_the_floor`
  (`E2040_LADDER=none` wins), `ladder_mode_is_stamped_in_the_manifest`.
- **Proof command.** `node tools/gh-check.mjs cargo test -p engine-render --lib ladder`
  prints `5 passed`.
- **The golden must stay** `d57b5c8ae3096614`: the ladder is presentation only.

### LY1 / LY2 / LY3 — the three missing workspace-check rules (lead applies)

- **LY1 (sim may not import presentation or platform).** Rule text and the exact
  code block are in "The anim question" above. Place: `tools/workspace-check.mjs`,
  the two constant sets right after the `allowedEdges` map (after line 102), the
  check inside the dependency-edge loop (the block that starts at
  `const allowed = new Set(allowedEdges.get(pkg.name) ?? []);`, line 462), and the
  pass line next to `PASS dependency edges and std-only engine-core` (line 883).
  Proof: `node tools/workspace-check.mjs` prints
  `PASS layer rules: no simulation crate imports a presentation crate`.
- **LY2 (the GPU boundary).** Rule text: only `crates/render/src/present/**` may
  name `wgpu::`. The 31 files that break it today are named in the report, not
  failed: the check is report-only until the list is empty, and a **new** file
  outside `present/**` that names `wgpu::` fails. Place: a new export
  `export function gpuBoundaryHits(files)` in `tools/sim-source-scan.mjs` (it
  reuses `maskRust` and `productionCode`), called from `tools/workspace-check.mjs`
  in the same place as the genre ledger scan (the `genreFindings` call near line
  669), with the pass/warn line next to the genre ledger line (line 830).
  Proof: `node tools/workspace-check.mjs` prints
  `WARN gpu boundary: 31 files outside present/ name wgpu:: (report-only)` and
  `PASS gpu boundary: no new file outside present/ names wgpu::`.
- **LY3 (no IO in a simulation crate).** Rule text: a simulation crate's
  production code may not use `std::fs`, `std::net`, `std::io::Write`,
  `File::create` or `std::process::Command`; `#[cfg(test)]` code may. Place: a
  new export `export function ioHits(src)` in `tools/sim-source-scan.mjs` (it
  reuses `productionCode(src)`, which already cuts the file at the first
  `#[cfg(test)]`), called from `tools/workspace-check.mjs` over the same crate
  list `SIM_CRATES` (from `tools/rust-quality.mjs`) that the platform-float scan
  uses. Proof: `node tools/workspace-check.mjs` prints
  `PASS io boundary: no simulation crate does IO outside its tests`.

### The new crate edge this lane needs (lead applies)

`crates/arena` now reads `crates/nav` (the layered nav consumer, LYR-1). The
allowlist needs one word:

```js
  ["engine-arena", ["engine-input", "engine-physics", "engine-combat", "engine-ai", "engine-anim", "engine-destruction", "engine-ecs", "engine-provider", "engine-nav"]],
```

Until the lead applies that line, `node tools/workspace-check.mjs` prints
`engine-arena -> engine-nav: undocumented local dependency edge` — that red is
this lane's edge, not dirt.

## Rejections

- `engine-ecs` `JobPool::shared()` was crate-private; making it public was the
  whole door for the sim, so no new pool crate and no rayon dependency.
- The per-fighter parallel step inside `Sim::step_with` was rejected: the brain
  loop mutates `self.brains[slot]` while the percept reads `self.states`, so the
  split needs a borrow refactor of the hashed path. The per-fighter fork-join
  landed on the recorded path instead (same bytes, no hash risk).
- `engine-physics` `step_islands` is not in the tree and `crates/physics` is held
  by the collisions lane, so PX15 stays theirs; this lane did not touch it.
- The input layer was not put in the tick hash: the golden is byte-frozen, so the
  layer is hashed beside the hash (the cost is written in
  `crates/arena/src/input_layers.rs`).
- Unity layers, Unreal `ECollisionChannel`, Godot `collision_layer`/`collision_mask`:
  already mined and proprietary, research-only; Bullet is the portable donor.
- `bevyengine/bevy`, `jrouwe/Jolt`, `dimforge/rapier`, `meshoptimizer`, `Godot`,
  `Unreal`, `Unity`, `O3DE`, `Filament`: already mined, skipped.

## Open items (not landed here)

- The arena crate does not compile in the current tree: `crates/ai/src/plan_trace.rs`
  (another lane, `Blackboard: Eq` missing) and `crates/physics/src/{layers.rs,cloth.rs}`
  (the collisions lane) are red. `engine-input` and `engine-ecs` compile green.
- `crates/render` (GS44), `crates/assets` and `crates/game` (WEB6), `crates/anim`
  (AM24), `crates/physics` (collisions), `crates/physfx` (arXiv lane) and
  `tools/` (speedup lane) were not touched.
- The lead applies: the `engine-arena -> engine-nav` allowlist line, LY1, LY2 and
  LY3, and the `docs/ARCHITECTURE.md` row split for `anim`.
