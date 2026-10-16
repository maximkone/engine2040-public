# ROADMAP — integrated milestones, evidence before expansion

Background for scope. The order of work is `FOCUS.md` "## Now" (owner 2026-10-10); where this file differs, FOCUS wins.

`FINISH-RICH.md` defines alpha/v1 product acceptance. `SPRINT.md` holds the
task board on one tree (`C:\engine2040` on master, owner 2026-10-10), run by
`/sprint` in waves of up to 20 packets. The game is `FOCUS.md`.

## Current baseline

Run `node tools/finish.mjs` for the current baseline; do not copy old crate or
test counts forward. The 2026-09-28 report records A2/A3 editable-scene and
editor runs, A6 provider evaluations, and A7/A9/A10 replay, failure and latency
evidence. A4 is partial (GX3/GX5/Q1); A5 is partial and A8 open on GM3, the
clean packaged run. The window and editor evidence therefore supersede the
old headless-only baseline. A receipt proves its recorded revision and fixture,
not every current build. Re-run affected proof after changes.
`ARCHITECTURE.md` owns boundaries; `VISION.md` Delivery evidence owns the next
integrated results. A crate, trait or unit test alone never completes a milestone.

M0 status 2026-10-01: met locally, including `cargo deny check`, default and
all-features tests and lint; no hosted CI by owner decision (local proof is
the CI, see FINISH-RICH A1); CPU perf-baseline pins landed in
`tools/perf-baseline.json` (PF13 gate), GPU/VRAM rows pending the quiesced
reference-GPU fill (PF19).

## Finishing strategy — what makes this great

The research is broad enough; the gap is integration. Four rules govern the
remaining plan:

1. **Research that reaches a consumer.** Use `VISION.md` Research contract,
   the existing steal map and `research/LADDER.md`. Competitor code and arXiv
   feed measured spikes beside delivery; expand those existing records rather
   than creating duplicate catalogs. Subsystem plans live in `plan/`, written
   by `PLN` board rows. A source read is not a shipped capability.
2. **Adopt proven crates for layers that do not differentiate.** Proposed
   defaults below; each needs a `DEPENDENCIES.md` row, a passing `cargo deny
   check` and a short decision record. Licenses listed are expected, not verified.
3. **One vertical slice defines "great".** The Roman arena (M3), edited in the
   editor, changed by an AI proposal through the command service, played, saved
   and replayed. Other features wait for it. `RESEARCH-VERDICT.md` marks which
   researched ideas are IN, LATER or DROP.
4. **Differentiate on the AI-safe command service, not feature count.** Every
   editor, script, CLI and model action goes through one schema-typed,
   validated, previewable, undoable, replayable transaction API, exposed to
   external agents through MCP and to in-engine providers through adapters.
   Mature engines have renderers; few have that contract. Build it in M1.

| Layer | Adopt (expected license) | Build here |
|---|---|---|
| Math | `glam` (MIT/Apache-2.0) | Fixed-step simulation numerics in `core` |
| Window/GPU | `winit`, `wgpu` (in use); WGSL ported from Bevy 0.18/0.19 (MIT/Apache); FSR 3.1 wgpu port for upscaling | Passes, materials, frame graph, captures |
| Import | `gltf`, `image` (MIT/Apache-2.0) | Persistent asset IDs, cook cache, schema |
| Physics | `rapier3d` with `enhanced-determinism` (Apache-2.0) | Adapters, character glue, hash coverage |
| Editor UI | `egui` + `egui-wgpu`/`egui-winit`, `egui_dock` or `egui_tiles`, `transform-gizmo-egui` | Panels, schema inspector, transactions |
| Authored files | `serde` + RON or JSON (MIT/Apache-2.0) | Versioned schema, migrations, canonical hashing |
| IDs/digests | `uuid`, `blake3` (allowed licenses) | Identity vs content-version rules |
| Audio | `cpal` (Apache-2.0), `lewton`/`hound` decoders | Mixer and spatial model in `audio` |
| Profiling | `puffin` or `tracy-client` (MIT/Apache-2.0) | Budgets and receipts |
| AI transport | Official Rust MCP SDK; HTTP client only in the adapter crate | Command service, validation, replay, evaluation |

Scale honestly: Flax is over a decade of dedicated engine work. Engine-wide
parity is not a milestone; a Flax-class reference scene on the declared GPU
tier is. Visual quality there comes mostly from content, lighting and temporal
stability.

### Content track (beside M1–M3)

Good pixels need good assets as much as a good renderer. Every asset gets a
provenance row (source, license, author, SHA). Content that may not be
redistributed stays outside this MIT repository.

- M1: Khronos glTF sample models for correctness (license per model) and CC0
  HDRIs/textures for the material chart.
- M3: a CC0 or purchased arena kit; one hero gladiator (bought, commissioned
  or CC-BY with attribution); animations whose terms allow use in a shipped
  game; crowd impostors. Generated assets only after a license check; several
  generators forbid commercial use.

## M0 — trustworthy workspace and proof surface

**Outcome:** a new contributor can reproduce what the repository actually proves.

- Implement the workspace checklist in `ARCHITECTURE.md`: toolchain/MSRV,
  inherited package metadata/lints, feature matrix and dependency-edge audit.
- Use valid discoverable OpenCode skills; align agent rules with the architecture.
- Remove misleading CI coverage. A GPU job must run GPU code, a seed test must
  use parsed seeds, and a replay test must consume a saved input/state tape.
- Add a real proof CLI: seed, ticks, record/replay, backend selection, strict
  unknown-argument rejection, machine-readable receipt and nonzero failure exit.
- Keep the legacy 600-tick fixture frozen while adding independently versioned
  integration fixtures. Ensure negative tests fail when a seed/input/state
  change should be detected; repeating the same constant is insufficient.
- Record baseline compile time, test duration and binary size before setting
  regression budgets. Select the first desktop CPU/GPU/driver target explicitly.

**Exit:** locked build/test/lint/license checks on Windows and Linux; default
and window-feature compilation; two matching legacy hashes; true multi-seed
and recorded replay tests; zero misleading “passed” jobs. Unsupported GPU runs
are reported as unsupported, never passed. Skills appear in `opencode debug skill`.

## M1 — one scene from source asset to editable running game

**Depends on:** M0; schema/identity decisions are the first implementation work.

- Establish app lifecycle, 3D transforms, stable asset/object IDs and a versioned
  component/scene schema. Wire through reusable runtime integration.
- Import a licensed glTF mesh and PBR textures, cook/cache them, instantiate a
  scene, render it with a real 3D camera/depth buffer and save/reopen it.
- Build the small editor now: viewport, hierarchy, schema-backed inspector,
  selection, transform gizmo, log, save, undo/redo and child-process play.
- Define command transactions used by inspector, scripts, CLI and future AI.
  Implement revision checks, idempotency, validation and recovery before chat.
- Render milestone V1 in `VISUAL-TARGET.md`; do not delay this until every
  subsystem or advanced geometry technique is finished.

**Exit:** one documented workflow imports → places → edits → saves → reopens →
plays; asset reimport retains references; 100 edit/undo/redo operations restore
the expected document; stale/invalid transactions do not partially apply;
killing the play process preserves the document. Capture the rendered scene
and attach its manifest. One viewport and one scene are sufficient.

## M2 — AI is a usable engine/editor capability

**Depends on:** M1 command/schema seam. Provider protocol can be designed during
M1, but world writes wait for working transactions.

- Deterministic NPC perception, memory/blackboard, bounded decisions and action
  execution wired into the actual simulation, with a trace inspector.
- Provider-neutral request/result/capability interfaces and mock conformance
  tests; implement one local and one remote adapter without vendor scene data.
- Editor assistant can inspect the scene, propose a typed diff, preview/apply
  it, run play validation and show an undoable receipt.
- Scene and spatial understanding are core services (`ARCHITECTURE.md` AI
  control loop): a relation graph, a bounded brief for any model, and intent
  placement where a model states relations and a solver returns transforms
  (SP1–SP5). Every AI edit passes `engine-spatial` checks (no new overlap,
  floating object, blocked door or broken nav link) before apply; models
  supply labels and intent, deterministic code owns geometry.
- AI playtest agents drive the headless arena through MCP and file bugs with
  replayable input tapes (QA1–QA2).
- External agents of any vendor connect through the MCP door (N8) and use the
  same command service as the editor; no second write path.
- Optional live NPC requests run outside the tick with deadlines, cancellation,
  backpressure and scripted fallback. Accepted outputs become recorded actions.
- See `EDITOR_AI_CONTROL.md` for authority, offline and failure semantics.

**Exit:** the same edit task works through manual controls, both adapters and
one external MCP client; 20 fixed task fixtures report completion/rejection/latency/cost; malformed,
stale, duplicate, cancelled and timed-out outputs pass failure tests; 10,000
ticks with slow/offline providers keep advancing; saved actions replay without
network/model access and reproduce every authoritative checkpoint.

## M3 — the multiplayer arena slice: looks good, plays well, fights together

**Depends on:** M1; M2 lands alongside. The slice is the multiplayer Roman
arena from `FOCUS.md`. Two spines run in parallel lanes on the `SPRINT.md`
board: the **simulation spine** (headless match → integrated physics, combat,
AI and root motion → LAN rollback), so multiplayer works before visuals do,
and the **visual spine** (V1 → IBL/shadows → skinning → V2 lighting → VFX →
upscaling). Content items:

1. Netcode early on the simulation spine: server-authoritative input
   validation and replication, rollback over LAN, 2+ fighters in one arena,
   AI gladiators fill empty slots; tier-3 determinism per `HASH-LAW.md`.
2. Skinned characters: GPU skinning, clip playback and blending; simulation
   owns root motion.
3. Player combat: third-person camera and character controller on the chosen
   physics backend; sword and shield with attack, block, dodge, hit-react,
   hit-stop, stamina, active ragdoll and gore.
4. Opponents: deterministic gladiator AI (perception on a clock, utility/BT
   attacks, morale), 1v1 then 2v2+, with a pacing director.
5. Arena: sunlit sand and stands plus a dim gate tunnel; V2–V3 in
   `VISUAL-TARGET.md` (shadows, IBL, baked GI/probes, stable AA, bloom,
   grading); torches, dust, banners, decals, baked destruction debris and
   pre-broken props (runtime fracture lands only when profiles demand it),
   ballistics/siege props; the crowd as an instanced backdrop with cheer
   reactions (presentation only).
6. Sound: hits, footsteps, crowd bed, spatial output.
7. Save/load of progression; replay any fight from its recorded input tape.
8. Optional live announcer or taunt lines from any provider: presentation
   only, scripted fallback. This shows provider-agnostic AI in the game.
9. Editor: material editing, asset diagnostics, profiler overlay, bake
   progress; changes visible without restarting the editor.

Not in the alpha slice: naval (CUT, not postponed), campaign (OPEN,
undecided), and the district, siege and ludus maps (M6). Platform: a Windows packaged game on the
declared GPU tier; Linux keeps headless determinism CI only. Profile before
meshlets, visibility buffers, ray tracing, virtual textures or custom
allocators.

**Exit:** a packaged 5–10 minute multiplayer arena session on Windows at the
declared quality tier (2+ fighters over LAN, AI-filled slots), with
moving-camera clips, reference shots, CPU/GPU/VRAM reports, clean device
validation and fight replays that reproduce every authoritative checkpoint.
The reviewer accepts the visual reference suite; a sprite count cannot satisfy
this gate.

## M4 — reliability, scale and networked play

**Depends on:** M3. Replay/snapshot/input contracts were designed in M0–M1.

- Asset streaming, dependency invalidation, prefab overrides, scene migration,
  crash recovery, project validation and incremental build/cook packaging.
- The second game (Finish B): urban war, a tactical CQB shooter
  (`plan/urban-war.md`). Its genre-neutral engine rows (UW1–UW26) run beside
  M3/M6; its game rows start after Finish A (UW0); dense ruined cities
  follow in Tier C (city is in `plan/urban-war.md`).
- Harden the M3 multiplayer core: dedicated server, more fighters, real
  transport under jitter/loss/reordering/disconnect/reconnect with actual
  clients. Authority and replay diagnose mismatches.
- Opt-in rollback only for systems whose state and determinism scope are
  demonstrably covered beyond the M3 slice.
- Test jitter/loss/reordering/disconnect/reconnect with actual clients, server
  and transport. In-memory algorithm tests remain a separate lower-level suite.
- Content-scale benchmark and soak scenes, memory/load-time budgets, allocation
  traces, import fuzzing and supported hardware/driver matrix.

**Exit:** packaged multiplayer session (dedicated server, N fighters) under specified network faults converges;
save migration and crash recovery fixtures pass; repeated load/unload/play
cycles have no unbounded growth; captures and budgets remain within baseline.

## M6 — the 2040 tier: Rome online, at the V5 look (Finish C)

**Depends on:** GX1–GX6 (the look is live on the GPU) and M3 for gameplay.
Rows run beside M3–M4 on the board's `### 2040 tier` section; none skips a
Finish A gate. Operator 2026-09-24.

- Look: tier probe, acceleration structures, RT shadows, ReSTIR many-light
  and GI with no bake, RT reflections, denoiser, path-traced reference,
  probe-grid GI for Base, GPU-driven virtual geometry, GPU crowd, volumetric
  fog and clouds, skin/eyes/hair, cinematic camera, photo mode (RT, VG, VX).
- Online: QUIC, relay and NAT traversal, interest management for 64 players
  plus AI legions, WAN fault soak, server-authority cheat checks, sessions,
  networked physics, voice (ON).
- World: cell streaming, ludus hub, Rome district, siege field with AI
  legions and breakable walls, day/night and weather, mode framework (WD).

**Exit:** a packaged Windows build joins a dedicated server over the internet
by invite code; a 64-slot bot-filled siege plays 10 minutes under the ON4
fault model and replays exactly; the arena night scene with 250 shadowed
torches and a day-to-night sweep with no bake meet the RT-tier budget on the
reference GPU; RT-tier stills sit within perceptual tolerance of the path
tracer on the material chart. Ultra stays unverified until O10 names a GPU.

## M5 — shipping tiers and wider platforms

Release a real sample game with documentation, accessibility/input remapping,
localization, asset licensing manifests, project templates and compatibility
policy. Add Web/mobile quality tiers only after capability and thermal/memory
tests. Consoles need partner access, toolchains and certification; no date or
parity promise follows from a Rust compile. Advanced rendering, large worlds,
destruction, vehicles and domain-specific game systems follow demonstrated needs.

## Execution rules

- Work runs from the `SPRINT.md` board on one tree: waves of up to 20 packets,
  one code packet per crate, shared types committed first, one gate per wave.
  Worker count is not a progress metric; ticked acceptance is.
- Each task names consumer, deliverable, dependencies, runnable acceptance test
  and failure artifact. A proposal/spec/stub is labeled as such.
- Milestone dates follow measured throughput and hardware access. Do not turn
  repo-star counts or arbitrary wave counts into estimates.
- On repeated identical failure, quarantine that task with owner/repro/output
  and continue independent work. No failing requirement becomes “done.”
