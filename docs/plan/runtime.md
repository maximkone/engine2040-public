# Runtime

Goal: one runtime and one world model under the hash law.
Acceptance:
1. `node tools/proof.mjs` golden d57b5c8ae3096614 x2.
2. The spine step equals the legacy step hash on the canonical fixture.
3. GE ledger gate: CUT-genre fixture games boot, play and replay with zero engine edits.
Order: RC8 RC9 RC11 RC15 RC16 RC19 RC20 RC21 RC22 RC24 RC25 RC26 RC27 RC28 RC30 RC31 RC32 RC34 RC35 ARC5 ARC6 ARC8 ARC9 ARC10 ARC11 ARC12 ARC13 ARC14 ARC15 ARC16 ARC17 ARC7 ARC18 ARC19 GE6 GE11 GE12
Owner crates: core, ecs, app, arena (read-only consumer), scene.
Open board IDs: RC8 RC9 RC11 RC15 RC16 RC19 RC20 RC21 RC22 RC24 RC25 RC26 RC27 RC28 RC30 RC31 RC32 RC34 RC35 ARC5 ARC6 ARC7 ARC8 ARC9 ARC10 ARC11 ARC12 ARC13 ARC14 ARC15 ARC16 ARC17 ARC18 ARC19 GE6 GE11 GE12
Pre-merge text: `git show d96091cd:docs/plan/runtime.md`
Pre-merge text: `git show d96091cd:docs/plan/architecture.md`
Pre-merge text: `git show d96091cd:docs/plan/general.md`

## 1. Where it stands

| Capability | Status |
|---|---|
| Fixed tick + seeded RNG + hash (core) | unit-tested, integrated |
| Minimal SoA ECS (ecs) | unit-tested, consumed |
| Fixed-order phases (ecs schedule) | unit-tested |
| Bulk-loop jobs (ecs jobs) | unit-tested |
| Versioned snapshots (ecs snapshot) | unit-tested |
| Scene hierarchy + codec + provenance (scene) | unit-tested, integrated |
| Headless arena match (arena) | unit-tested, integrated |
| Engine-app lifecycle | implemented (16 tests), editor-optional |
| Unified 3D world model | bridge landed, spine open |
| Prefabs + schema registry | implemented, unit-tested |
| Scripting story | evaluated, row ticked |
| Deterministic sim jobs | bake-only fork-join landed |

### genre-neutral ledger

| Capability | Status |
|---|---|
| HUD scoreboard model | unit-tested, integrated, **not moved (GE6 open)** |
| Camera framing window | implemented, private 4-cap remains (GE13 open) |
| Bench duel harness | implemented, duel-only shape (GE14 open) |
| Minimap presentation floats | implemented, unquarantined (GE16 open) |
| Human-slot HUD/wins follow local slot | implemented, slot-0 hardcode remains (GE17 open) |
| HUD cap literals mirror the canonical caps | implemented, literal caps remain (GE18 open) |
| Provider observation 8-window on a 32-slot bout | implemented, single-window limit remains (GE19 open) |
| Beacon browser capacity vs match slots | implemented, transport bound undocumented (GE20 open) |

## 2. Finish line

- **Tier A (alpha):** `engine-app` lifecycle crate LANDED (`crates/app`,
  16 tests, startup verify frame `VERIFY_TICKS=8`); world-model verdict
  recorded bridge-first (`bridge.rs`, wire v1); schema v1 + prefab
  instantiate landed (`schema.rs` 9, `prefab.rs` 38 KB / 15 tests);
  jobs stay bake-only. Remaining for the tier: WIN1/ED1 boot through
  `App::run` (WIN/ED-owned wiring), RC30 retires the f32 clock, RC31–RC33
  pin the app overhead/startup/extraction budgets. Measurable: prefab
  instantiate under 1 ms per kit, startup under 2 s cold, per-frame app
  overhead under 0.1 ms p50, all posted as PF1-style workload numbers
  (RC31/RC32), determinism audit strict with zero hits.
- **Tier B (v1):** second game boots on the same runtime (PROVES the
  engine, per Finish B): its own config, menu, match, save and package
  flows, zero engine forks, sharing tick, RNG, scene, heal and proof
  infrastructure. Deterministic fork-join jobs for sim-adjacent work
  (bake, verify, broadphase) with order-proof tests; script-lite
  evaluation landed (data-driven logic or an embedded language decision
  with license cleared); schema migration v1→v2 demonstrated with a
  fixture. Measurable: two games, one runtime; migration museum test.
  Second-game boot checklist (what PROVES it): stock config loads, menu
  renders through the same extraction, a headless match replays to
  identical checkpoints, saves migrate through the same schemas, and the
  package step ships without engine edits. Any fork of engine code for
  game two fails the tier by definition.
- **Tier C :** hot reload of systems (plugin order +
  extraction versioning); visual scripting stays CUT forever (data and
  text, never nodes); distributed bake farm stays CUT (local pools
  suffice). Measurable: reload drills, bake-time numbers.

Scale and edges (r4, the numbers Tier B/C must hold): 64 players —
server tick p99 ≤ 4 ms (P6) with the netcode-1.02 handshake (RC15) and
the digest-chain divergence naming (RC22) localizing the first divergent
tick and stable id; 1,000 agents — archetype-World tick p99 ≤ 2 ms (P6)
via the RC9 incremental digest (spike scoped 2026-09-29: 20/20 seeded
divergences, 74.5% fewer bytes, digest `5d740c17` x2 identical), the
RC11/RC20/RC21 bake-only parallel lanes (bit-identical across 1/2/4/8
threads or killed) and RC16 wrapping change ticks (2x on 1%-dirty 10k
entities); this laptop's GPU (RTX 3050 6 GB) — presentation only, never
hashed: extraction snapshots carry tick + build hash + schema version so
a stale renderer/audio consumer detects skew and degrades loudly (RC33
freezes `EXTRACTION_SCHEMA` v1 with a museum refusal test). Bridge caps
already size the path: 256 arena rows (covers 64 fighters + AI fill),
65536 scene/ecs slots (covers siege streaming + crowd). Best existing
implementation to beat or extend: `crates/app/src/app.rs` (ordered
startup, verify frame, journaled shutdown), `bridge.rs` ExtractionView,
`parallel.rs` fork-join, `counter_rng.rs` keyed streams, `time.rs`
integer-ns clock, `prefab.rs` instantiation, `snapshot.rs` append_hash
plus RC29 `run_arena_tick`.
- **2040-level runtime:** one `World` handle across scene/arena/ecs with
  per-owner write rules (§3) and zero legacy paths (f32 clock gone via
  RC30, legacy arena step retired via ARC7); integer-ns time the only
  clock; incremental digest the default rehash with exact localization;
  the second game (urban war, UW39 companion) booting with zero engine
  forks (RC34 receipt); a 10-minute 64-slot siege replaying exactly (C5,
  RC35 soak); schema museum v1→vN with recorded migrations. What we will
  never build does not change: no visual scripting, no second
  game-specific runtime branch, no distributed baking, no machine-code
  hot reload (data + system order only).

### genre-neutral ledger

Tier A alpha (the ledger is law): GE2 ledger green, GE3 one canonical
cap with the golden unchanged, GE9 ratchet in proof, GE2b ledger re-green,
GE10 menu cap derived — all ticked. Left: GE6 scoreboard moved, GE11
ledger refreshed, GE12 cap decision, GE13 camera-16 framing, GE14 bench
verdict + smoke, GE15 split-32 composition, GE16 minimap quarantine. Bar:
`git grep -i fighter|gladiator|arena` over engine crates below
`arena`/`game` returns only ledger-listed contract sites; `ui` builds
with no `engine-input` fighter import; camera frames the full roster without
heap (last measured at 16 slots, the cap is 32); the 4-fighter golden
`d57b5c8ae3096614` is identical twice.

Tier B v1 (a second game fits): GE4/GE5/GE7 land — all ticked. Bar met:
the GP5 fixture rules crate (different tables, different params, same
framework) boots headless; a 16-slot loopback soak converges (measured at
16; a soak at the 32 cap is still owed); a fixture
table set with different numbers resolves without engine edits; a
scripted non-gladiator policy drives the same tick.

Tier C (EMPIRE: general engine, any genre could be built):
GE1's fixtures — top-down twin-stick, vehicle course on D1 math,
third-person traversal course — each boot, play and replay headless
through public engine APIs with zero engine edits; the shipped game
stays Rome. **Done on board.** Bar achieved: fixtures green in proof,
engine diff empty, hash twice each.

CUT (never): RTS/TD/BR/racing/platformer/building/survival genre systems,
naval, fighting-genre frame-data, skilltrees-full, crafting-full,
economy-full, progression-full, stealth-full, survival-full,
building-full. Fixtures prove those genres *could* run; none of them
ships content, modes or waves.

## 3. Target design

Threads (decision D6, 2026-10-12: center for the owner, "decide on all"; record `docs/adr/` via CQ22): the sim may run an index-ordered fork-join on the one `JobPool` when the hash is equal at 1, 2, 4 and 8 threads (RC36), otherwise it stays single-threaded and deterministic; jobs parallelize
bake/verify/import with index-ordered chunk ranges (existing `jobs.rs`
pattern) plus deterministic fork-join reduction (sum/fold in declared
order, never thread-count-dependent — HASH-LAW forbids it). The app
spawns worker threads only for forks and bakes, exactly like FK1/PT1 do
today.

Determinism: same binary + config + inputs → same checkpoints, now
including prefab instantiation order and schema migration output. Schema
versions enter hashed state (a v1 scene and its migrated v2 must hash
differently by rule, like the arena sim-track tag byte). System
registration order is part of the build, not data — reordering systems
is a code change with a replay receipt, never a config tweak.

Budgets: startup under 2 s cold on the reference machine (measured,
posted); prefab instantiate under 1 ms per arena kit; schema migration
linear in component count; per-frame app overhead (ordering + extraction)
under 0.1 ms p50, asserted in PF1-style workload tests once they exist.

### spine

Boundaries: `docs/ARCHITECTURE.md`.

Data flow: inputs and tapes enter the tick exactly as today; the tick
steps systems registered in `schedule::Phase` order over spine
components; each tick folds a canonical checkpoint hash; extraction
publishes one snapshot (spine state + tick + build hash + schema
version) that render, audio, UI, net, save, editor and the MCP door all
read. Nothing outside the tick writes sim state; providers and scripts
submit recorded actions applied at tick boundaries, as HASH-LAW already
requires.

Ownership: `core` stays std-only with zero deps (tick, RNG, Fixed,
hashes). `ecs` owns the spine: entity lifetime, the five components as
SoA columns, the schedule, snapshots. `arena` owns match rules and
becomes a consumer of `engine-ecs` (the one new dependency edge, declared
by the orchestrator before dispatch). `scene` keeps authored hierarchy
and IDs; the spine holds live simulation — scene writes hierarchy, the
spine writes simulation, the bridge's `StableId` discipline becomes the
lookup between them until a consumer receipt proves unification pain.
`app` orders and extracts; it never steps combat or physics itself.

Threads: the sim stays single-threaded. Parity runs the two paths in
sequence inside one test process (no threads, no nondeterminism); FK1
forks keep their existing worker-thread shape and fork spine state the
same way they fork arena state today.

Status 2026-10-17: disk shows the S17 row's `:138-139` cite is off by 42 lines: `docs/plan/runtime.md:138` still carries the D6 fork-join sentence ("the sim may run an index-ordered fork-join on the one `JobPool` when the hash is equal at 1, 2, 4 and 8 threads (RC36), otherwise it stays single-threaded and deterministic"), while the standalone false line is `docs/plan/runtime.md:180` "Threads: the sim stays single-threaded."; `docs/adr/0006-single-threaded-sim.md:3` now records that answer as `Status: Proposed` (Test-Path True), so the `:138` "record `docs/adr/` via CQ22" half is no longer future work, and the door is still shut: RC36 is an open, ready board row at `docs/SPRINT-20-game.md:221`, so the S17 gap holds at `:180` and only the `:138-139` cite is stale.

Determinism: spine math is `Fixed` and integer only, stable orders only
(index order, never hash order), no wall clock, no float
transcendentals — the HASH-LAW tier-3 rule verbatim. Component hamt/sets
are `BTreeMap` or index-ordered Vecs from birth so RC12 never has to
re-audit the spine. Canonical bytes per component reuse the
`CanonicalBytes` + `StableId`-order contract RC9's row requires.

Budgets: spine step overhead vs the legacy step within 10% on the PF1
8-fighter workload (posted A/B, kill the abstraction if it misses);
0 allocations per tick after warmup (counting-allocator gate, RC10
shape); snapshot/restore round-trip under 1 ms at 8 fighters; parity
harness (600 ticks x2 paths) under 60 s so proof stays fast.

API sketch: `Spine { transform: Col<Transform>, body: Col<Body>,
fighter: Col<Fighter>, ai: Col<AiState>, anim: Col<AnimState> }`,
`Spine::step(tick, inputs, rng) -> Checkpoint`,
`Parity::compare(old, new, ticks) -> FirstDivergence`,
`spine::snapshot() / restore()` versioned beside the ecs v1 envelope,
`ExtractionView::from_spine(&Spine)` as the first real consumer of RC2.

Migration shape (side by side, then retire): a `spine` feature flag on
`engine-arena`. Flag off: today's code path, today's golden, untouched.
Flag on: inputs translate once into spine components, the spine steps,
checkpoints translate back to the legacy checkpoint shape for
comparison. The parity harness asserts byte-equal checkpoints for 600
ticks on seeded tapes. Only when green twice does the retirement row
flip the default, delete the legacy step, and move the golden with its
HASH-LAW receipt.

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Schedule proof: every system declares the columns it reads and writes; the build refuses an unordered conflicting pair; a test runs the golden under 50 seeded shuffles of every free ordering (Bevy shuffle_seed). RC36 may not tick before that test is green (RC38). No machine-code patch of sim crates (subsecond patches only the tip crate); OPT13 is re-scoped to a reload drill: snapshot, relaunch, restore, the same hash at tick N+K.

## 4. Hard problems

- Jobs that stay deterministic: index-ordered chunks plus declared-order reduction, proven by order-permutation tests (same result under every chunking). HASH-LAW bans thread-count-dependent reductions. Rows: RC11, RC20, RC21.
- 1,000-agent tick without breaking the golden: first approach is RC9 (incremental digest beside the frozen `append_hash`), then RC16 (change-tick scan-skipping), then RC35 (measure the archetype World on a 1,000-agent fixture). Kill rules travel with each row: under 10% gain at 8 fighters (RC9), under 2x Changed-scan (RC16), p99 over 2 ms (RC35). Do-not-repeat list: table-only, wall-clock salt, vendored trie.
- 64-player server tick that names the culprit: RC15 handshake (netcode.io shape, spec as reference only) for session order, RC22 digest chain for first-divergent-tick naming, RC24 sim-verify harness for 10k-tick injected-event soaks. Transport stays in `multiplayer.md` (`### internet play (ON)`). Donor: madsim deterministic sim-test shape (Apache-2.0; PORT cite and EVIDENCE row at RC24 port time).
- Two clocks in one core (`FixedAcc` f32 beside `time.rs` integer-ns): RC30 measures the f32 residual on the 10M-push soak, then deletes the legacy path with a HASH-LAW receipt (fixture version, reason, old/new hashes). No consumer may read the f32 clock after; the quarantine test fails the build if one does.
- App overhead asserted, not hoped: RC31 and RC32 measure ordering plus extraction p50 and cold startup on the reference machine as PF1-style workloads. The section 3 budgets are refusal gates; over-budget startup refuses with the slowest stage named.
- Spine migration without voiding 130+ ticked tasks of evidence: the migration shape is in `### spine` (section 3), the rows in `### ARC rows` (section 7). This plan owns only the runtime seams ARC6 and ARC14 consume (extraction schema RC33, integer-ns dt in `time.rs`). The golden moves exactly once, with a HASH-LAW receipt, or not at all.

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4.

## 6. Slim

Delete: the script stub is already gone (SL1); no other runtime code is
dead. Merge: prefabs into `scene`, schemas into a `schema` module, jobs
into `ecs` — no new crates except `app`, and `app` only because something
must own startup order. Freeze: core stays std-only with zero deps
forever (workspace audit enforces); phase order is code, not data; schema
v1 once shipped. We will not build: visual scripting (CUT), a second
game-specific runtime branch (one runtime or it is not an engine),
distributed baking (CUT), hot code reload (reload data and systems order,
not machine code). Test count is not a runtime metric on its own: every
new test must pin an order, a version, a migration or a refusal, and
tests that only count their own existence get deleted in the next slim
pass.

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): RC38 declared access and seeded order shuffle. Row change: OPT13 becomes the reload drill (snapshot, relaunch, restore, equal hash).

Rows from the books (2026-10-12, board shard `../SPRINT-20-game.md`; `BOOK-GAPS-2-2026-10-12.md` 2.1): RC36 the fighter step on the job pool (index-ordered fork-join, golden equal at 1 and 8 threads; decision D6), RC37 fork runs on the shared pool.

- [ ] RC8 deterministic math spike: correctly rounded sin, cos, exp and atan2 behind a gate; `Fixed` stays the default sim math. Accept: a 1M-input corpus is bit-identical on two toolchains and twice per build; zero allocations; the sin/cos ban in CODELAW lifts only on green.
- [ ] RC9 incremental digest experiment: per-component hashes folded into archetype roots and a tick root, O(changed) per tick, beside the full rehash. Accept: same verdict as the full rehash on 20 of 20 seeded divergences with exact localization; at least 10% faster at 8 fighters or killed with numbers; golden unchanged.
- [ ] RC11 parallel bake and verify: the RC3 fork-join extended to bake and verify. Accept: bit-identical across 1, 2, 4 and 8 threads times 1 and N chunks; at least 2x on a scaled bake or killed.
- [ ] RC15 netcode 1.02 protocol reference: a custom Rust session handshake (netcode.io shape, spec as reference only). Accept: handshake roundtrip x2; bad keys refuse.

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| RC16 | wrapping change ticks | B | none |
| RC19 | optional views + dynamic spawn | B | ARC1, RC4 |
| RC20 | ordered slot-write collect for bake/verify/anim-eval lanes | B | RC3, RC11 |
| RC21 | dispatch plan for bake/verify | B | RC11, PF2 |
| RC22 | slice-pumped off-tick jobs plus digest-chain divergence | B | RC9, ARC3 |
| RC24 | deterministic simulation testing harness | A | RC7, ARC11 |
| RC25 | deterministic hash for cross-platform entity IDs | A | RC7, ARC12 |
| RC26 | deterministic tick recording and replay | A | RC7, ARC13 |
| RC27 | deterministic ECS schedule verification | A | ARC13, RC7 |
| RC28 | counter-based PRNG quality upgrade | B | RC7 |
| RC30 | retire the f32 `FixedAcc` legacy path | B | RC6 |
| RC31 | per-frame overhead harness | A | RC1 |
| RC32 | cold-startup + prefab-instantiate budget proof | A | RC1, RC4 |
| RC34 | second-game boot receipt | B | RC1 |
| RC35 | 1,000-agent and 64-slot runtime soak | B | RC29 |

### ARC rows

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| ARC5 | spine rollback and persistence | A | ARC4, NET1 |
| ARC6 | spine extraction | A | ARC4, RC2 |
| ARC8 | spine keypoint fixtures | A | ARC4, VK1 |
| ARC9 | relations to spine | B | ARC4, SP5 |
| ARC10 | spine scorecard | A | ARC3 |
| ARC11 | rules-as-code coverage on spine | A | ARC4, VK1 |
| ARC12 | relation-diff edits to spine | A | ARC4, SP5 |
| ARC13 | parity-gated retro | A | ARC3 |
| ARC14 | spine float quarantine | A | ARC1, ARC2 |
| ARC15 | spine fork parity | A | ARC4, FK1 |
| ARC16 | VP1 digest over spine | A | ARC4, VP1 |
| ARC17 | spine columns on the archetype store | A | RC29 |
| ARC7 | retire legacy | A | ARC4, ARC5 |
| ARC18 | integer-ns dt through `run_arena_tick` | A | ARC17, RC6 |
| ARC19 | single-tape spine parity probe | A | ARC17 |

### genre-neutral rows

- [ ] GE6 scoreboard moves from `ui` to `game`: killer and victim fighter rows. Accept: HUD tests green; ui builds with no `engine-input` fighter import.
- [ ] GE11 ledger refresh for post-GE2 files: re-audit crates for genre names. Accept: zero unlisted genre names; a seeded new assumption fails the gate; proof generality gates green.
- [x] GE12 canonical participant-cap decision: rule whether the cap stays 16 or the split-match-32 harness is defined. Accept: 32-bot x600 soak hashes x2 or 16 stays with the split-match-32 harness defined; golden `d57b5c8ae3096614` unchanged x2; GE2 ledger green. Landed 15049ef3. Owner decision 2026-10-20: 32; scale filter 19 passed run 39485189547, input 52 passed run 39482930043, golden x2 runs 39485643367/39486071864.

## 8. Quality gates

- One `App::run` entry boots the game and the editor; shutdown reverses
  startup; journaled.
- Prefab instantiate round-trips; unknown schema versions refuse loudly;
  museum holds v1.
- Fork-join returns identical results under every chunking including
  single-thread; determinism audit stays strict with zero hits.
- No system writes another lane's state; extraction reads snapshots only
  (reviewed per edge).
- Scripting decision lands as an EVIDENCE row before any runtime code.
- Scale gates join the permanent set: app overhead p50 (RC31), cold
  startup + prefab instantiate (RC32) and the 1,000-agent/64-slot soak
  (RC35) re-run every wave beside P5/P6; a regression is a red gate and
  the next wave's first packet. The extraction museum (RC33) refuses
  stale bytes forever — a guessing loader is a red gate on sight.
- Proof integration per row: RC1's startup order is asserted by a
  registration test (duplicates refuse); RC2's bridge is asserted by a
  three-state extraction test; RC3's order proof runs 1-thread and
  N-thread chunkings; RC4's museum test loads v1 and refuses the unknown;
  RC5's evaluation is a docs-only row with license re-reads attached. No
  row ticks on a red gate, same as every code row on the board.
- RC1 boots the game and the editor in review (follow-up wiring rows, not
  this plan); RC2 carries its unify-vs-bridge verdict row; RC3 proves
  order-independence under 1-thread and N-thread chunkings; RC4 shows the
  v1 museum fixture loading; RC5 shows the license re-reads or the
  documented no-language verdict.

### genre-neutral ledger

- Every row: golden `d57b5c8ae3096614` identical twice; replay-exact
  twice; `node tools/proof.mjs` green on the landed tree.
- GE2/GE11 gate: workspace-check fails any new genre name below
  `arena`/`game` outside the ledger; a seeded assumption must fail it.
  GE11 re-audit 2026-10-18: the scan also counts `FIGHTER`/`GLADIATOR` caps
  and ids (`MAX_FIGHTERS`), the ledger lists 85 files with a verdict per
  group (contract site, harness, budget-shape, move), and an entry that
  names no genre word any more prints `WARN genre ledger` (prune it).
  `ai/src/legion.rs` names none; `game/` sits above the gate.
- Fixture gates (shared with GE1/GP5): fixtures build on public APIs
  only; `git diff -- crates/` empty after a fixture lands, enforced
  in proof; each fixture hash twice.
- GE6 gate: `ui` builds with no `engine-input` fighter import; HUD
  tests green; windowed HUD path (game → ui quads) unchanged visually.

## 9. Session estimate

Sizes live on the board rows; the old estimate is in `git show d96091cd:docs/plan/runtime.md`.
