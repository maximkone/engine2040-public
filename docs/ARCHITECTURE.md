# ARCHITECTURE — boundaries before more subsystems

Status: target architecture, with migration work in `ROADMAP.md`. The
workspace holds 35 crates plus the root package (36 total). A crate name or
unit test is not evidence that its subsystem is integrated into an application.

## 1. Dependency direction

Replace “no cross-imports” with **an explicit acyclic dependency graph**. Rust
crates are for ownership, reuse, feature isolation and compile boundaries; use
modules for implementation details. Do not create a crate for every feature.

```text
game / editor / headless / cook / playtest applications
                |
       engine-app composition + lifecycle       (built)
                |
  gameplay | presentation | command service | MCP + AI providers
                |
  scene + spatial understanding (core services every client uses)
                |
   simulation + assets contracts
                |
        foundational types + engine-core
```

Arrows mean “may depend on.” Lower layers never import an application, editor,
provider SDK or platform event loop. No global service locator. Cross-lane
integration belongs in the composition layer, through typed IDs, snapshots,
events and commands, rather than mutual access to each other's mutable state.

| Boundary | Ownership and allowed relationships |
|---|---|
| `core` | Tick, seed streams, hash primitives; std-only, no platform/provider/GPU dependency |
| `ecs` | Entity lifetime, storage, queries, schedule; may use foundational types |
| `physics`, `nav`, gameplay `ai`, `combat` | Simulation algorithms; consume explicit input, emit results; adapters attach them to ECS |
| `assets`, `save` | Stable identities, versioned schemas and cooked data; IO/import stays outside the tick |
| `scene` | Persistent IDs, transform hierarchy, schema metadata, versioned codec; shared by runtime, editor, commands and AI |
| `spatial` | Scene understanding as a core service: checks, relation graph, bounded brief, intent placement solver; deterministic, model-free |
| `arena` | Headless deterministic match of the game; the multiplayer and playtest spine |
| `render`, `audio`, `anim`, `ui` | Presentation snapshots and resources; no authoritative feedback from frame timing |
| `rules` | NS32 (decision D2): game rules as integer-only Rhai text, sandboxed (deny-all registry, op/call/size limits), `set`/`test`/`explain` with last-good, script hash in the tape header; depends on `core` and the pinned `rhai`, plus optional `arena` behind the `arena` feature; off `default-members`; no tick crate depends on it, the arena adapter calls `Rules::tick` and applies the returned actions |
| `geo` | Cook-time mesh tools (decision D4): booleans, UV unwrap, seam-locked LOD, collision decomposition; depends on `core`, each donor behind its own feature, off `default-members`; no sim crate depends on it; reads `assets` only for its kit tests (`kit` feature) |
| `material` | The one material record/table v2 (NS31a, decision D1/D3): OpenPBR-named look plus gameplay layers; depends on `core` and `destruction`, optional `naga` behind `validate`; in `default-members` |
| `input`, `net` | Bounded input/transport adapters; enqueue typed simulation commands; C1's std-only fighter command IDs/inputs belong to input, not arena |
| `endpoint`, editor command service | Versioned tool schemas and receipts; transports call the same local service; door v2 packs live in `crates/endpoint/src/packs/` (`scene`, `break_`, `anim`, `material`, `asset`, `rules`, `geo`, `gym`, `view`) |
| `provider` | Capability discovery, async requests, cancellations, budgets; no simulation ownership |
| `playtest` | AI playtest agents over MCP; bugs carry replayable input tapes |
| `editor` | Documents, panels, selection, transactions, play process; uses runtime contracts |
| `engine-app` (built) | Startup/shutdown, plugin registration, system order and extraction; wires dependencies explicitly |

Current exceptions are migration facts: `engine-ai` directly imports `ecs` and
`physics`, and its `patch` mutates both. Move application of commands to the
composition/command service when the new seam has a working consumer; do not
break existing proofs just to rearrange folders. `engine-perf` is a tools
application and may compose multiple lanes. `engine-arena` depends on `engine-ecs`
behind the off-by-default `spine` feature (the spine step and parity harness);
`engine-endpoint` enables it to play a scene revision (`play.run scene_rev`).
`engine-endpoint` depends on `engine-anim` (feature `json`) behind its `mcp` feature: the `anim` verb pack
runs the state machine door state (`anim.graph.set`, `anim.graph.undo`, `anim.preview`); anim never imports endpoint.
`engine-anim` depends on `engine-ecs` for the RC3 job pool (`par.rs`, AM14): per-character eval fans out in index-ordered chunks for presentation and bake; `ecs` depends on `core` only, so no cycle, and the sim tick never calls it. `engine-anim` also has a dev-only edge to `engine-nav` (the TV2 contract test ties the traversal chooser to the link registry); nav never imports anim.
`engine-endpoint` depends on `engine-assets` (features `import`, `registry`) behind its off-by-default `intake` feature: the
`asset.intake` verb cooks a generated GLB through `engine_assets::gen_intake` (cook-time only; no tick crate depends on it); assets never imports endpoint.
`engine-endpoint` depends on `engine-physics` behind its `mcp` feature: the `physics` verb pack reads the std-only balance core (`engine_physics::balance::response_matrix`, `recovery_ticks`), so one tuning has one answer; physics never imports endpoint.
`engine-endpoint` depends on `engine-nav` behind its `mcp` feature: `scene.query reachable` reads each staged actor's polygon from a baked nav mesh through `engine_nav::reach::reach_view` (and refuses `needs_nav_view` when the session holds none), so reach has one owner; nav never imports endpoint.
`engine-endpoint` depends on `engine-input` behind its `mcp` feature: `play.run` validates tape inputs against the fighter contract ranges (`WISH_RANGE`, `FACING_RANGE`, `KNOWN_BUTTONS`) that `engine_input::fighter` owns, instead of mirroring them; input never imports endpoint.
`engine-endpoint` depends on `engine-playtest` behind its `mcp` feature (the real `look.judge` host) and on `engine-proof` behind its off-by-default `verify` feature: the `verify` pack serves `replay.verify`, `replay.bisect`, `bug.replay`, `trace.summary` and `verify.proof` from the playtest replay and proof bug/trace bodies, so the door and the library measure the same rows; playtest and proof never import endpoint.
`engine-proof` depends on `engine-combat` (default features): `break_corpus.rs` feeds 10,000 seeded swings to the cut resolver and pins one digest; combat never imports proof.
`kam2-game` is the KAM3 window shell (first playable window): it composes `kam2-sim` (std-only, IO-free, no engine crate), the `engine-render` window leg (the fixed 2.5D projection, the procedural sprite atlas and the HUD pass), `engine-ui` HUD data, `engine-assets` palette conversion and `engine-clock` for the 10 Hz shell tick. Render never imports the game crate, and no frame, camera or clock reading enters the sim; the crate map and `tools/workspace-check.mjs` carry the same edge.

### Crate integration status (2026-10-01)

Measured by who calls each crate outside its own tests. Unit tests prove an
algorithm, not integration; see `FINISH-RICH.md` for product gates.

| State (2026-10-01) | Crates |
|---|---|
| Composed by `game`, which now ships `arena` and `staged_fight` binaries | `arena`, `input`, `combat`, `anim`, `ai`, `physics`, `net`, `save`, `ui`, `audio`, `render` |
| Composed by `endpoint`, which now ships the `e2040-mcp` binary | `editor`, `scene`, `spatial`, `arena` |
| Used by other library crates | `core`, `ecs`, `nn`, `scene`, `assets`, `observe` |
| Used across sim crates (not root-only) | `nav` (by `destruction`), `destruction` (by `arena`, `game`, `endpoint`, `material`, `perf`) |
| No consumer yet | `playtest`, `provider`, `distill`, `world`, `game`, `endpoint` as libraries (their binaries exist; library consumers pending) |
| Binaries | the proof CLI (`src/main.rs`), `perf`, `arena`, `staged_fight` (`game`), `e2040-editor`, `e2040-mcp`, `bundle_cook` (`assets`) |

#### distill keep/kill decision (2026-10-02, F07): KEEP, parked

`engine-distill` has zero in-repo reverse dependencies (proof: `rg
"engine_distill|engine-distill"` hits only its own `Cargo.toml`/`lib.rs`,
the workspace-check allow-edge, baselines and prose docs) — but that is by
design, not dead code. It is the DT2 student trainer leaf: the default build
stays std-only (spec + dataset + student + NN2-shaped export) and only the
`train` feature pulls `burn`/`burn-ndarray` (pure-Rust CPU, no GPU/C++
runtime; see `crates/distill/Cargo.toml` backend-policy comment). Same shape
as the other intentional leaves (`endpoint`/`perf`/`playtest` bins, which are
self-only by design). SL1 remains the row that deletes whatever is still
unconsumed; until then do not delete, exclude, or pre-wire this crate.

WIN1 (`arena.exe`), the editor app binary and the MCP binary have landed
since the 2026-09-23 baseline, so RA1/WIN1/ED1/MCP5 executables exist;
DX1 wires `destruction` into the arena; SL1 deletes what stays
unconsumed.

What is missing is integration, not algorithms: `game`, `editor` and
`endpoint` run as executables now (WIN1, ED1, MCP5 landed), but GM3, the
clean-folder packaged run, is still open. EA1a wires `provider` into
`endpoint`; SL1 deletes what stays
unconsumed. Each board row names the consumer that wires its crate; do not
extend a crate before its consumer exists. `tools/workspace-check.mjs` fails
on source files no module declares and on platform float math in simulation
crates.

### Contract ownership before the next dispatch

Planned edges are not wired yet. The orchestrator documents/checks each actual
manifest change in `tools/workspace-check.mjs` before dispatch; do not pre-allow
all possible future imports.

- C1: input owns plain fighter IDs/commands, combat owns combat state/events,
  AI owns observation data, arena owns aggregate match state/checkpoints.
  Arena converts its state to those lower-level inputs; combat/AI/anim never
  import arena. Task prerequisites are not reverse dependency permissions.
- C3: assets owns cooked mesh/material data. C4: anim owns skeleton/clip/pose/
  root-motion contracts. Feature-gated assets import may depend on anim; anim
  does not depend on assets. Render consumes cooked geometry/poses; arena
  consumes deterministic root tables, never a render device.
- C5: scene owns serializable provenance records; editor fills them through
  commands. Spatial receives immutable before/staged scene data and emits
  findings/repair descriptions; it never imports editor `Batch`/`Op` types.
  Findings panels live in editor (SP2). This prevents editor ↔ spatial cycles.
- Provider owns requests/capabilities/session policy and adapters; editor EA1
  uses that service directly. MCP is another client of the shared commands,
  not a dependency of internal gameplay AI or CAP1 permissions.
  MCP1 initially consumes the existing editor command service; declare that
  endpoint → editor edge before dispatch. Do not invent a duplicate transport
  schema or put provider/transport types in scene to avoid doing the audit.
- Save owns the existing versioned envelope/storage API (SV1); game owns a
  separately versioned progression payload and its fixtures. Save never imports
  game/arena. Existing CRC/roundtrip tests are not crash-durability evidence.
- Reload producers (assets/combat/AI) publish immutable versioned results;
  game/arena composition owns application at frame or recorded tick boundaries.
  No worker edits consumer crates just to satisfy its own unit test.

### Scene and spatial understanding: the AI control loop

Scene and spatial understanding are core services, not editor features. The
editor, scripts, the CLI, MCP clients and in-engine providers all use the same
loop, and no step needs a model:

1. **Observe** — `scene` read plus the `spatial` brief: stable IDs, groups,
   relations and findings, bounded to a token budget; captures when a model
   can see images.
2. **Propose** — typed command batches, or intent placement in relations that
   the `spatial` solver turns into transforms. Models give labels and intent;
   deterministic code owns every coordinate.
3. **Check** — schema, ranges, revision, then `spatial` checks (overlap,
   floating, blocked doors, nav links). Failures return the minimal conflict
   set and legal values.
4. **Preview → apply → undo** — the command service: atomic, journaled,
   monotonic revisions.
5. **Play-test** — the proposal runs on a deterministic fork of the headless
   `arena` match before it applies (counterfactual), and on the real match
   after; AI playtest agents drive both through MCP.
6. **Verify** — verifiers act as content compilers (spatial, playability,
   balance, budgets); checkpoints, metrics and captures are the evidence; every
   bug carries its input tape, replays exactly on any machine, and causal
   replay names its cause. Pillars and research: `AI-NATIVE.md`.

## 2. Rust workspace foundation — M0 checklist

- Pin one tested stable toolchain plus rustfmt/clippy in `rust-toolchain.toml`;
  CI reads that pin. Choose an explicit MSRV after checking the resolved GPU
  dependencies. Edition and MSRV are separate; do not bump to 2024 by reflex.
- Make every package inherit workspace version/edition/license/rust-version.
  Root `[workspace.package]` currently does not propagate automatically.
  Centralize repeated dependency versions under `[workspace.dependencies]`.
- Add workspace lints and `[lints] workspace = true` in each member. Start with
  `unsafe_op_in_unsafe_fn` and justified unsafe documentation; tighten existing
  warnings incrementally. A root lint table alone does not enforce child crates.
- Keep resolver 2 until an edition/resolver migration is deliberately tested.
  Use additive features, explicit supported combinations, and `--locked` in CI.
  Check minimal/default, window/GPU, editor, provider and server builds separately.
- Keep the default proof runnable without a GPU or network. Editor/provider
  dependencies must not enter the headless build transitively. Audit with
  `cargo metadata` and `cargo tree`, then automate allowed dependency edges.
- Keep `Cargo.lock` tracked. Add measured dev/release profiles rather than
  unconditional LTO, native CPU flags, allocator swaps or mandatory cache tools.
- One repository task runner: `node tools/proof.mjs` runs every local gate. There is no `xtask/` directory and no `cargo xtask`.
- Separate reusable runtime integration from proof binaries as M1 lands;
  examples should call the same app lifecycle as the editor and shipped game.

Status 2026-09-23 (local receipt in `SPRINT.md`): toolchain pin, inherited
metadata and lints, the dependency-edge/module audit, `.gitattributes` and the
task runner exist. Open: explicit MSRV decision, feature matrix beyond
default/window. No hosted CI (owner decision, GitHub Actions off): local proof is the evidence.

## 3. Scene and data contracts — settle before a complex renderer

- **Coordinates:** proposed right-handed, +Y up, camera forward -Z, meters,
  radians and seconds. Specify matrix multiplication order and clip/depth
  conversion in the render adapter. Existing 2D/centimeter helpers need explicit
  adapters; do not reinterpret their stored values silently.
- **Identity:** generational entity handles for live objects; persistent UUIDs
  for authored objects/assets; content digest for an artifact version. A content
  hash must not be the persistent asset ID: reimport should retain references.
- **Transforms:** parent/local/world with cycle rejection, deterministic update
  order, nonuniform-scale policy, and precise reparent-with-world-pose behavior.
- **Schema:** stable type/field IDs, typed values, bounds, enum choices, read-only
  metadata and migration versions. One schema feeds serialization, inspector,
  AI tools and validation. Memory layout and `TypeId` are not save formats.
- **Scenes/prefabs:** stable references, prefab overrides, missing-reference
  diagnostics, format versions, forward-compatibility policy and migration tests.
- **Assets:** source + settings + importer version + target profile → cooked
  artifact digest. Dependency graph, invalidation, atomic cache publication and
  asynchronous loading with fallback/error states. Use a cryptographic digest
  for artifact integrity; the simulation FNV digest is not a security primitive.
- **Lifecycle:** deterministic spawn/despawn queues; resource load/unload states;
  explicit teardown order for threads, devices, jobs and play processes.

## 4. Three clocks, separate responsibilities

1. **Simulation:** fixed tick, stable system ordering, explicit seeded streams.
   Authoritative physics, gameplay animation/root motion, NPC decisions and
   rollback state are owned here. Hash all state that can affect future ticks.
2. **Presentation:** variable frames interpolate previous/current snapshots.
   GPU skinning, visual particles, camera effects and audio device scheduling
   do not mutate authoritative state. Wall-clock timing is appropriate here.
3. **Services:** IO, import, compilation, model requests and networking run on
   bounded workers. Apply their validated results at defined boundaries.

Parallel simulation uses declared read/write access and deterministic reduction
order. Job completion order must not select gameplay outcomes. A bounded queue
needs a documented overflow policy (clamp and count overflow_tiles in the frame receipt); async is not permission for unbounded work.
Start with a simple schedule; measure before custom ECS/allocator/task-system
rewrites. Select one primary 3D physics backend in a decision record instead of
simultaneously promising Rapier, Jolt and a custom XPBD solver.

## 5. Plugins, reload and failures

- Built-in Rust plugins register systems, resources, asset types and schemas at
  startup through a versioned contract. Compile-time composition is the first
  implementation; a plugin interface does not require dynamic libraries.
- Rust dynamic-library ABI is unstable. Native extensions require a versioned
  C ABI with ownership/error rules, or a process protocol; WASM needs a bounded
  capability host. Pick an actual user need before adding either.
- Data/shader hot reload comes before native-code reload. Validate replacements,
  retain the last good version, retire GPU resources only after in-flight work,
  and publish changes transactionally. Runtime GPU pipeline creation is normal;
  avoid uncached first-use hitches rather than promising zero runtime compilation.
- Editor play runs in a child process with a project snapshot and explicit
  apply-back diff. A game crash must leave the authoring document recoverable.
- Use typed errors with context at boundaries, no panics on invalid files,
  stale IDs or unavailable devices. Unsafe code needs an ownership/aliasing/
  lifetime justification plus targeted tests; FFI and GPU byte layouts need
  explicit alignment, padding and version rules.

## 6. Verification is part of the design

`HASH-LAW.md` owns simulation scope; `VISUAL-TARGET.md` owns render evidence;
`EDITOR_AI_CONTROL.md` owns transactions and model contracts. CI must exercise
the code path named by each job. Headless hashes, GPU images, frame timings,
provider conformance and editor recovery are separate gates.
