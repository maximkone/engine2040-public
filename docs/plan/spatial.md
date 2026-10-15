# Spatial

Pre-merge text: `git show a0e13976:docs/plan/spatial.md` (the dated merge and index lines are dropped; history stays in git).

Goal: the engine understands a scene as numbers before any model is asked.

Acceptance:
1. The golden scene brief for the arena fits the token bound; `spatial_eval` (100 seeded tasks) gives the same score twice.
2. Every SP row keeps the brief suites green (`cargo test -p engine-spatial`); golden `d57b5c8ae3096614` unchanged.
3. SP35: the same brief on mock, local and remote gives the same placements.

Order: rows whose `needs` are all ticked first (SP16, SP21, SP40, SP50, SP60 and the rest), then the rest in ID order. The `needs` column is the rule; the table is the list.

Owner crates: `spatial` (`crates/spatial`), `endpoint` (scene tools, consumer only), `playtest` (`spatial_eval`), `scene`, `game` (`view_truth.rs`: the frame the window draws, as numbers).

Open board IDs: SP16 SP19 SP20 SP21 SP28 SP33 SP36 SP37 SP40 SP43 SP44 SP45 SP46 SP49 SP50 SP56 SP58 SP60 SP61 SP62 SP63 SP63b SP63c SP64 SP65 SP67 SP68 SP69 SP70 SP71 SP72 (ticked and never-boarded rows are dropped).

## 1. Where it stands

| Capability | Status |
| --- | --- |
| Staged before/after gate (SP1) | unit-tested, integrated |
| Revision-bound repairs (FX1) | unit-tested, integrated |
| Digest-stable relation graph (SP3) | unit-tested, integrated |
| Bounded brief (SP4) | unit-tested, integrated |
| Intent placement solver (SP5) | unit-tested, integrated; audit open (SP38) |
| Semantic diff as relation changes (SD1) | unit-tested, integrated |
| Editor findings panel (SP2) | unit-tested, integrated |
| MCP tools brief/spatial/place (MCP3) | unit-tested, integrated |
| 3D index (SP6) | unit-tested, integrated |
| Relation vocabulary v2 (SP7) | unit-tested, integrated |
| Sight and reach (SP8) | unit-tested, integrated |
| Production reroute onto index3 (SP14) | unit-tested, integrated |
| Incremental index3 rebuild (SP15) | done |
| Affordances (SP9) | done (Wave 54) |
| Scene questions + sampler (SP10) | done |
| Intent macros (SP11) | done |
| Spatial gold eval (SP12) | done |
| Composition checks (SP13) | done (Wave 54) |
| View truth: drawn bodies as numbers, run laws, JSONL brief (0948ba6f) | integrated: `arena view:`, product check floors |
| Hierarchical brief (SP19) | open (needs SP18 ticked) |
| Spatial questions as engine verbs (SP20) | open (needs SP19) |
| ai-native SP6 proposal (BEV fixture) | superseded |

Board check 2026-09-30: SP6, SP7, SP8, SP14, SP15 ticked on `../SPRINT.md`.
Library-present never means consumer-wired: SP54 library hunks are present
(`place.rs` `RepairHint`/`ConflictSet.hints`, `diff.rs` `dry_run_diff`) but
`ScenePlaceOutput` carries no hints field yet — rows below scope the wired
side. Likewise `index3` is fully 3D but the SP5 solver rests on the ground
plane (`place.rs:26-31`), so stacked rest is SP16 only.

Module notes constraining the rows: `crates/spatial` is std-only and owns
no sim state (callers map documents to plain `Actor` snapshots; yaw
octant-snapped, libm-free; solver output enters only through the command
service). SP16 reads the SP37 enumeration evidence and adds the
ground-vs-support rest distinction with flat-room behavior byte-identical.
The SP3 `Relation` enum and `Graph` are frozen; v2 is a separate layer
(`RelGraph`, `digest_rel`).

## 2. Finish line

Tier A alpha (agents place without breaking scenes): SP9 affordances,
SP10 questions + sampler, SP14 standing, plus new SP15 (incremental
index), SP16 (stacked placement), SP17 (repair hints). Bar: 100 gold
queries at least 95% with reasons; sampled placements 0 overlaps, 0
floating, 100% reachable; dense-room solve under the 50 ms p95 bar;
10 AB1 briefs place with zero findings and undo cleanly; every
conflict carries a solver-computed repair hint and a dry-run diff
equals the applied result 10 of 10.

Tier B v1 (agents build fast): SP11 macros, SP12 gold eval. Bar: 20
briefs build with at least 50% fewer calls than raw placement, same
checks clean, one undo each; one command reproduces the spatial score
per build and a seeded relation bug drops exactly its cases.

Tier C (taste): SP13 composition. Bar: 20 reference shots ranked like
the review panel (rank correlation at least 0.7); feeds the P1 hero
shots and the SHOW2 director as advice, never as a gate.

CUT (never): learned 3D encoders or 3D backbones in git, SGClip
weights, open-ended 30-turn agent loops, automatic save-time repairs
(FX1 refusals stand), second index/tree systems beside index3.
Search today is exact graph query + budgeted brief text — no
embedding/vector asset index exists, multi-modal retrieval is absent
by design. Offline/advisory methods (collapsed briefs, ranked
candidates, bake-side indices that never enter git or the tick) may
be researched as SPIKEs with numbers; none is authorized by research
alone, and no row silently adds weights, tick code, or auto-apply
paths. SP33 stays advisory (ranked candidates, never auto-apply);
vision cross-check stays advisory in `spatial_eval.rs`.

View truth (owner 2026-10-11: "know the scene blindly, better than a screenshot"): the
engine knows its own frame as numbers. 0948ba6f: `crates/game/src/view_truth.rs` gives each
drawn body's act, clip, tint, bounds, screen rect and joints, and the run's laws in
`arena view:` (pops, sinking, NaN, camera in a body, sim ticks shown); the product check
scores them. Bar: SP61 the same with no window at 2000 frames/s joints-only; SP62 the door
verbs; SP63 a-c the GPU's per-body pixel numbers replace the PNG floors; SP64 props and
light; SP65 picks and line of sight 100/100. A model reads numbers first; pictures stay for
taste (EG30, LF15).

## 3. Target design

Data flow (one direction, no cycles):

```
scene document -> Actor snapshot -> index3 build/probe
  -> checks (Report) + relation graph (Graph/RelGraph, digest)
  -> brief (budgeted text) / diff (relation changes)
  -> intent solve (TransformOut or ConflictSet + repair hint)
  -> command service (preview, apply, undo) -> digest line back
```

Ownership: `crates/spatial` owns checks, index3, relations, brief,
diff, repair, sight, place, and (new) afford/sample/compose. It never
owns the document, the command batches, or the MCP wire. `scene_tools.rs`
owns MCP DTOs, validation and the scene-to-actor mapping only.
`intents.rs` and `scene_query.rs` own the macro/query expansion into
SP5 placements in one undoable batch. `spatial_eval.rs` owns the gold
harness. Consumers: MCP3 tools today; SP10/SP11/AB1/EA1b tomorrow;
ARC9/ARC12 write spine transforms through the same tick-boundary
recording once ARC4 lands (glue in endpoint call sites, never spatial).

Threads: single-threaded caller code, stays that way. RC3 fork-join may
parallelize bake/verify lanes later, but tick and solver verdict never
depend on thread count (id-sorted builds, id tie-breaks, BTree order,
frozen check order).

Determinism: meters in, integer centimeters inside; the only float touch
is the shared `to_cm` input rounding, never the digest. FNV-1a over
canonical integer bytes per domain with distinct tags (`e2index3:1`,
`e2sight:1`); no `HashMap` iteration reaches a hash. Unknown
positions/sizes degrade to isolated nodes, never errors; inverted boxes
and bad blobs refuse with typed errors.

Budgets (kept green forever): 10k objects query within 1 ms p95;
`place.rs` solve under 50 ms p95 on 500 objects; ray query at most
0.2 ms; brief fits its byte budget with the digest line mandatory;
SP12 suite scores per build; PF1 tick budgets untouched (spatial never
runs in the tick).

API sketch (existing, rows deepen — never mirror):

```rust
// build once per solve, probe at microseconds
let idx = Index3::build(&actors);            // id-sorted, typed errors
assert!(!idx.overlaps_any(&probe_of(&cand)));
let report: Report = run_checks(&actors, &SPATIAL_CHECKS)?;
let graph: Graph = build(&actors);           // frozen v1
let brief: Brief = brief(&actors, &graph, &report, budget_bytes);
let out: Result<TransformOut, ConflictSet> = solve(&intent, &actors);
```

## 4. Hard problems

1. Words to transforms with honest refusal. Problem: models state
   relations; geometry must decide. Technique: quantized candidates
   (z, then x, then yaw ascending) with the first fully-satisfying
   self-verified candidate winning, else the minimal conflicting
   subset by removal in intent order — plus a solver-computed repair
   hint per conflict, then a canonical FNV digest plus a
   permutation-stable audit proving minimality. Why: deterministic,
   previewable, undoable; a dry-run graph diff equals the applied
   result; the audit stops silent over-claims. Reference:
   SpatialGrammar 2604.27555 (language + hint idea), QuickXplain
   (Junker 2004, conflict discipline). Best rival: allenai/Holodeck
   floor-objects solver (Apache-2.0) — constraint-then-search with no
   minimality proof and no digest; we go past it with shrink-by-removal
   plus the SP38 audit gate. Donor: O3DE intent API shape (pattern
   only); Holodeck constraint-plan shape reimplemented std-only (§5).

2. Scores that survive contact with agents. Problem: unit tests pass
   but fresh agents still mis-place. Technique: 100-task
   place/query/fix gold through MCP by a fresh agent, scored per
   build, with vision cross-check as advice only. Why: the headline
   metric is builds that pass asset checks but fail at runtime
   (EV2 rule); seeded relation bugs must drop exactly their cases.
   Reference: SP12 row shape; GameGen-Verifier injection discipline
   (VK1) as the neighboring pattern. Donor: none (harness is ours).

3. Answers that name their frame. Problem: "left of" asked of a
   raw brief gets model-guessed geometry (SP3-only 14/50,
   world-only 20/50 on 50 seeded Qs). Technique:
   Subject+World+Viewer cone verb over the frozen v2 predicates —
   ahead means strictly forward inside the 45-degree cone, viewer
   resolves actor position+yaw with nearer-means-front, integer-cm
   throughout, FNV digest x2, MCP verb answering from recorded
   fixtures. Why: the engine owns geometry; +60pp over SP3-only at
   identical bytes with 0 float. Reference: LADDER SP58 spike
   (50/50 + digest, ACCEPTED 2026-09-30; SP20 detail). Best rival:
   hovsg/HOV-SG hierarchical scene graphs (MIT) — open-vocabulary
   graphs with no edit contract and no undo story; we go past it
   with a typed verb, integer predicates and a digest. Donor: HOV-SG
   hierarchical-graph shape reimplemented std-only integer-cm (§5).

## 6. Slim

- Delete: the ai-native SP6 BEV-fixture text once this plan lands
  (it describes a different SP6 than the board's ticked index3; keep
  the eval-fixture idea only as SP12 acceptance, never as a second
  SP6).
- Merge: SP10 owns the sampler (`sample.rs`) and the query tool
  (`scene_query.rs`) in one row — never a sampler crate plus a query
  crate. SP11 owns all macros in `intents.rs` — never per-macro
  modules.
- Freeze: SP3 `Relation`/`Graph`, the 18-check catalog order, the
  brief priority (reds, relations, yellows, groups), and the
  `GRID_CM`/`YAW_STEPS` search order. Changes arrive as new layers
  (v2 pattern), never as silent edits.
- What we will not build: a second index beside index3 (grid stays
  only as the measured contender until a row retires it with
  numbers); learned encoders/backbones; open agent loops; automatic
  save-time repairs; composition as a gate (advice only); nav-mesh
  or physics code inside spatial (audio occlusion, NV3 rebakes and
  NV6 funnels query spatial; they never move into it).

## 7. Board tasks

Ticked rows are dropped; rows never boarded are not listed. Table: open SP rows, with the `needs` column.

| ID | Title | Tier | needs |
|---|---|---|---|
| SP16 | stacked placement | A | SP6, SP7 |
| SP19 | hierarchical brief for large scenes | A | SP18 |
| SP20 | spatial question answering from the graph | A | SP19 |
| SP21 | incremental free-space field on for placement and reachability | A | SP16 |
| SP28 | relation-cycle consistency over the SP3 graph | A | SP3, SP5 |
| SP33 | fuzzy referring-expression grounding over the SP3/SP7 graph | A | SP19, SP20 |
| SP36 | dirty-tile export from index3 incremental rebuild for touched-only nav re-bake | A | SP15 |
| SP37 | support-surface enumeration for stacked rest , feeding SP16/SP9 | B | SP16, SP9 |
| SP40 | integer-cm k-d tree for nearest/k-neighbor queries | B | SP3 |
| SP43 | VLM marked-proposal + engine-solver repair loop for brief compile | A | SP23 |
| SP44 | quantitative metric spatial-VQA from the graph | A | SP20 |
| SP45 | region-proposal metric relations | A | SP33 |
| SP46 | offline open-vocabulary tag lift from labeled captures into SP3 nodes | A | SP22 |
| SP49 | smallest-cycle refusal + repair receipt over the SP3 graph | A | SP28 |
| SP50 | per-instance prefab override deltas | A | RC4 |
| SP56 | : canonical digest + perm-stable audit gate as the SP38 proof leg | B | SP38 |
| SP58 | : Subject+World+Viewer cone verb + endpoint answer verb, 50/50 + digest x2 | B | SP20 |
| SP60 | frame-aware verb baseline tape | B | SP7, SP10 |
| SP61 | view truth with no GPU: pose split, joints-only mode, headless `view_run` | A | none |
| SP62 | door verbs `view.observe` and `view.laws` | A | SP61, EG16 |
| SP63 | GPU numbers slice 1: draw-id target | A | EG2 |
| SP63b | GPU numbers slice 2: per-id reduction, ring readback | A | SP63 |
| SP63c | GPU numbers slice 3: `arena pixels:`, brief px, product floors | A | SP63b |
| SP64 | props and light in the view truth | A | none |
| SP65 | ask the frame: `view_pick` and `view_sees` | A | SP61 |
| SP67 | tiny offline embedding model for scene queries (spike) | C | none |
| SP68 | quantized spatial runtime for nav and place queries (spike) | C | none |
| SP69 | CPU sense verbs: sense.view, sense.lidar, sense.map, scene.pixel | B | none |
| SP70 | capture-to-symbol lift: capture plus depth to Actor proposals | B | none |
| SP71 | SP12-B 200-task parity bench through MCP mock, local, remote | B | SP44 |
| SP72 | grasp points and trajectory waypoints | B | SP70 |

## 8. Quality gates

- `cargo test -p engine-spatial` green; SP1–SP8 + SP14 suites stay
  green under every row (dense-room solve under the 50 ms p95 bar, bytes x2).
- Determinism: same scene in any input order gives identical
  findings, graphs, briefs, placements and digests; 20 seeded
  permutations per row; golden `d57b5c8ae3096614` unchanged (spatial
  never feeds the tick).
- Budgets: 10k objects query within 1 ms p95; ray query at most 0.2
  ms; brief fits budget with the digest line always emitted;
  incremental rebuild p95 under 2 ms (SP15).
- Fresh-agent gates: SP10 100 queries at least 95%; SP11 50% fewer
  calls with identical clean verdicts and one-undo each; SP12 one
  command reproduces the score and a seeded relation bug drops
  exactly its cases; SP13 rank correlation at least 0.7 (advisory).

