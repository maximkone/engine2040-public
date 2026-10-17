# Content (Look)

Outcome: licensed characters, clips and props become cooked game data through one import, check and cook path, with one provenance row per file.
Proof: (1) cook x2 byte-identical, `cargo test -p engine-assets --lib` green; (2) arena kit (8 glTF) and 4 fighters load from the bundle in the window with textures (VS1); (3) every file under `content/` has a provenance row (workspace-check gate).
Order: CT21, CT23, CT58, CT59 (the Tier A chain: the weapons fix, the 80-clip set, its receipt and the consumer proof), then section 7 in table order. No row of this plan is in FOCUS Now (2026-10-10).
Crates: assets, game, `content/`.
Open board IDs: section 7 (the table), AS4 AS5 AS6 among them.

Tag NEXT (Look). Prefixes: CT, AS (assets merged as `### cook and delivery`). Plain summary: the engine can only look and move as well as its content. Landed: glTF import, retarget, mesh cook, bundle v1, the synthetic locomotion bake, the AI modeling loop, hero + 3 opponents, the arena kit, map kits, the fighter bind and the 600-tick staged fight. Open: the 80-clip Tier A set, the provenance gate and its audit, the crowd kit, the PBR pass, streaming handles and the engine cook gaps.

Merged 2026-10-10 (batch A of the plans review): `docs/archive/2026-10-10/plans/assets.md` as `### cook and delivery`.

Pre-merge text: `git show d96091cd:docs/plan/content.md`; `git show d96091cd:docs/plan/assets.md` (assets is archived at `docs/archive/2026-10-10/plans/assets.md`). Cut here: the wave-by-wave file map, the per-row kill rules, the "next milestone" list and the session estimates, in `docs/archive/2026-10-10/plans/content-cut.md`.

## 1. Where it stands

The table is what the code confirms today. Ticked rows are dropped (git history and the board keep them).

| Capability | Status | Evidence |
|---|---|---|
| Weapons graft CT21 | Code landed, board tick pending | `content/fighters/manifest.json` + `hero.weapon.json` (verts 2934 -> 3049) |
| Sidecar substrate (14 canonical sidecars) | Landed, audit gate open | `content/fighters/sidecars/` (CT24 audit + CT33 schema + CT36 wiring open) |
| Intake batches CT42/CT47/CT54 (48 clips) + finisher CT55 + batch-d CT68 | Boarded open / plan-only | `content/fighters/incoming/batch-a/`, `batch-b/`, `batch-c/`, `finisher/`, `batch-d/` (14 of 80 landed; CT58 reconciles) |
| Locomotion real intake CT37/CT48 + bake proof CT69 + scout CT76 | Open / plan-only | `content/locomotion/real/`, `real-batch-b/`, `PROOF.md`, `SCOUT-REPORT.md` (bake is 8 synthetic clips) |
| Crowd kit CT25 + variety CT38 + eyes CT44 + bake CT70 + staging CT50 | Open | `content/crowd/` (missing), `EYES.md`, `SOAK-REPORT.md`, `BAKE-REPORT.md`, `REACT-REPORT.md` (CT64) |
| PBR pass CT26 + report CT45 + intake gate CT74 + src receipt CT77 | Open | `content/arena/textures/`, `content/arena/pbr/REPORT.md`, `PBR-INTAKE.md`, `SRC-RECEIPT.md` |
| Provenance gate CT28 + audit CT46 + enforce CT52 + ledger CT71 | Open | `content/PROVENANCE-AUDIT.md`, `PROVENANCE-ENFORCE.md`, `PROVENANCE-LEDGER.md` |
| Load/stream CT51 + baselines CT61/CT72 + night CT57 + staged CT60 | Open / plan-only | `content/arena/LOAD-REPORT.md`, `SOAK-BASELINE.md`, `POPULATED-SOAK.md`, `NIGHT-REPORT.md`, `STAGED-PROOF.md` |
| Urban-war pilot CT56 + scout CT78 + full kit CT66 + props CT73 | Open / plan-only | `content/maps/urban-war-pilot/`, `PILOT-SCOUT.md`, `urban-war/`, `content/props/urban/` |
| Sound list CT63 + splat spike CT67 + own-capture CT65 + stream proof CT62 | Open / plan-only | `content/sound/HIT-LIST.md`, `content/props/splat/`, `content/capture/`, `content/maps/STREAM-REPORT.md` |
| Staged triage CT75 + dedupe tool CT79 + consumer proof CT59 + receipt CT58 | Open / plan-only | `content/fighters/incoming/STAGED-TRIAGE.md`, `tools/dedupe.py`, `CONSUMER-PROOF.md`, `CLIP-80-RECEIPT.md` |
| Engine cook gaps CT10/CT11/CT12/CT13/CT14/CT16 | Open | `crates/assets/src/` (no conformance test, anim_compress, manifest, proxy, motion_source; no `crates/endpoint/src/sandbox/content.rs`) |
| Establishing CT22 + window bind CT29 | Open | `content/arena/establishing.json`, `content/fighters/window_bind.json` (missing) |
| Untracked PBR/HDRI bytes + arena src + staged dir | On disk, uncommitted/untriaged | `content/arena/textures/wood_*_2k.jpg`, `content/incoming/` (HDRIs), `content/arena/src/`, `content/fighters/incoming/staged/` |
| Finish bars fed (A4 partial, C3 open, C5 partial, P6 partial) | Rows named in §2 | CT57/CT60 (Q1 night + staged pixels); CT64/CT70 (reacting crowd); CT62 (stream proof); CT61/CT72 (soak baselines) |
| LADDER lines touching content | Spike-first, no new row | splat CARD REJECTED stays behind CT67; SN11 boarded grows the SN10 bridge CT63 lists; compression TAKE cards stay behind CT11 |

### cook and delivery

Assets (formats, cook, delivery). Source = portable intake bytes (glTF,
sidecars, rights and hash manifest); built = engine-native cooked artifacts
(retarget, mesh and texture cooks, `BundleWriter` E204BNDL with blake3
digests, byte-identical x2); integrated = built artifacts bound and presented
through a consumer caller. Uncommitted bytes never count.

Landed as library code (not yet on a consumer's hot path; kept green):

| Capability | Status | Evidence |
|---|---|---|
| Streaming queue (AS2) | unit-tested, library | `crates/assets/src/stream.rs`: `StreamQueue`, `StreamBudget`, priority + blake3-tie order, frame-boundary `publish`, placeholders, typed `StreamError`; TEMPORARY local mirrors of `bundle.rs` budgets stay until AS9 |
| BC encode in-cook (AS3) | unit-tested, library | `crates/assets/src/bc.rs`: std-only BC1/BC3 `encode_image`, no vendored encoder |
| Runtime LOD swap (CT5) | unit-tested, library | `crates/assets/src/lod_swap.rs`: `LOD_BAND_MM`, `HYSTERESIS_MM`, `MAX_SWAPS_PER_TICK`, `poll_into`, 0 allocs after construction |
| Museum automation (CT6) | unit-tested, library | `crates/assets/src/museum.rs`: version registry, fixture template, `prove` fails a version without its fixture |

Open assets rows (AS4 AS5 AS6, CT10 CT11 CT12 CT13 CT14 CT16) are in section 7
with their files: no `handle.rs`, `residency.rs`, `usd.rs`, `gltf_conformance.rs`,
`anim_compress.rs`, `manifest.rs`, `proxy.rs`, `motion_source.rs` yet (Glob, 2026-10-03).

## 2. Finish line

- **Tier A (packaged arena):** hero + 3 opponents on one skeleton with the
  full 80-clip Tier A set (locomotion, directional combat, reactions, deaths,
  finisher pair, vault/hurdle/mantle/climb/drop), every clip with an audited
  sidecar (loop, root, contacts, hit windows, traversal edges) that CB4 and
  LM2 read on the staged fight (CT59 proves it); arena kit + establishing shot
  + window bind staged with 0 findings and one joint log==pixels capture
  (CT60); provenance gate locked (release refuses missing URL+SPDX+author+SHA
  or AI generator chain); cold arena load <= 5 s on the RTX 3050 6 GB with the
  empty-stands P6 baseline posted first (CT61 feeds CT27). Measured: cook
  identical x2 everywhere, budgets hold, `fighter_bind` green, captures show
  fighters not cubes. Nothing blocks CT10/CT12/CT13/CT14/CT16.
- **Tier B (dependable v1):** real-bytes motion DB (100STYLE motions-only +
  CMU, credited, Geno refused) baked into LM1 (CT37 first 16 clips, CT48
  second 16, CT31 style bake); combat batch-b under CT47 toward the 80; traversal
  markers under CT49; compression 5-20x with cm/degree error bounds and
  0-decode-alloc (CT11 + CT39 audit on the pin bundle); manifests with exact
  invalidation (CT12); urban-war content through the same pipeline with no fork
  (CT56 pilots the corner; CT66 full kit only after the pilot proves no fork).
  Measured: MB before/after posted, bundles load, replays x2.
- **Tier C (2040 tier):** 20k reacting spectators within budget (C3) from the
  crowd kit, LOD/impostor chain and variety pass (CT25, CT38, CT34, CT44); a
  day-to-night arena with 2k PBR maps and HDRI rows (CT26 + CT45, VRAM caps);
  the bounded splat cook stays spike-first (CT67: bake x2 + receipt + cap, or
  no row). Presentation never moves the golden hash `d57b5c8ae3096614`.

### cook and delivery

- Tier A (alpha, ships): AS2 streaming, AS3 BC encode, CT5 LOD swap, plus the
  AS8/CT17/CT18 locks. Measurable: the arena bundle streams under budget with
  placeholders that never leak into Q1 captures; textures ship BC with mips;
  fighters swap LOD by distance with hysteresis; cook identical x2; a seeded
  streaming regression fails the proof gate.
- Tier B (v1, production pipeline): CT6 museum automation plus the CT18
  completeness lock. Measurable: releasing a format version without its
  fixture fails the gate; 20 of 20 rebuilds byte-equal.
- Tier C: none here. OpenUSD stays LATER (after Finish A); AS6 waits CT10.
- CUT (never): UGC marketplace, mod tooling beyond the CT4 loop, console cert
  formats, runtime AI mesh generation in the shipped game.

## 3. Target design

Data flow: licensed source (`content/*/incoming/`, or `content/local/` for
redistribution-forbidden files) -> import (A1-A3, typed refusals, never
panics) -> engine-native cook (CT1 retarget + CT2 mesh + texture) -> bundle
(AS1) -> stream (AS2 priorities) -> bind (`fighter_bind.rs`) -> place (SP5)
-> present (RA14/WIN6). Every step writes a receipt; release bundles refuse
assets without provenance rows.

Ownership: `content/` owns data, sidecars and manifests (CC0/CC-BY may live
in git; Mixamo/Fab/Quaternius-QAL live in git-ignored `content/local/` and
ship only cooked); `crates/assets` owns the cook (std-only, deterministic);
`crates/game` owns bind glue; `crates/spatial` owns placement checks. Cook and
bake run offline; the fixed tick never cooks, never allocates, never touches
the network.

Determinism: same source bytes + cook settings -> same cooked bytes (x2 gate
on every row that writes bytes); sidecar JSON keeps document order; retarget
uses only finite math with 1e-4/1e-5 tolerances; presentation may use anything
but never changes the golden hash.

Budgets: hero <= 40k tris LOD0, opponents <= 25k, props <= 5k, <= 4 materials
per character; textures power-of-two (props <= 2k, hero <= 4k); base color
sRGB, normal and metal-roughness linear. 64 players + 1000 agents must hold P6
on the RTX 3050 6 GB (CT27 soak, CT35 fighter LODs, CT25 crowd kit, CT34 20k
card); VRAM capped and posted per row. Scale envelope: 8 fighters + day HDRI at
base 1080p60 with 0 validation errors (RA14 + CT29 + Q1); cold arena load <= 5 s
with VRAM posted (CT27); 1000-agent sim p99 <= 2 ms with 0 tick allocs (CT27 +
CT35).

Consumers (every row names its reader; no orphan content): clips + sidecars ->
CB4/LM2 via CT36 (schema CT33, audit CT24, proof CT59); locomotion bake -> LM1
via CT37/CT48 (proof CT69, scout CT76); fighter bundles -> WIN1 + staged
captures; arena + establishing -> RA14/Q1 via CT22/CT29 (proof CT60); crowd ->
soak + C3 card via CT25/CT38/CT34/CT44; manifests -> DT5/PR1 via CT12;
compressed bundles -> P9 via CT39; PBR + night -> Q1/RA14 via CT45/CT57;
provenance -> release/PR1 via CT28/CT46/CT52/CT71; maps -> C5/WD via CT62;
sounds -> SN10/AU3 via CT63; own-capture -> CT23/CT31 via CT65; urban-war ->
UW39 via CT56/CT66/CT73/CT78; splats -> VX7/C3 via CT67.

### cook and delivery

Data flow: source bytes + settings + importer version + target profile ->
artifact bytes + blake3 digest (cook, proven x2) -> `BundleWriter` pack ->
`StreamQueue` request/publish at frame boundaries -> resident LOD picked by
`LodSwapper` per fighter per tick -> frame presents. Failed jobs keep last-good
and never publish partial bytes (atomic publish, AS1 bar).

Ownership: `crates/assets/src/stream.rs` (AS2/AS8/AS9 hunks), `bc.rs` (AS3,
frozen), `lod_swap.rs` (CT5/CT18 hunks), `museum.rs` (CT6/CT18 hunks),
`crates/perf/src/workloads.rs` + `baselines.rs` (AS8/CT18 budget rows). Open
rows own their own new files.

Cross-engine boundary: the factory emits portable source only (glTF + sidecars
+ rights/hash manifest: URL + SPDX + author + SHA, or generator + model + prompt
digest). The engine cooks engine-native assets only (`retarget_cook`,
`cook_gltf_bytes`, `BundleWriter` E204BNDL + `verify_bundle_bytes`); those
bundles are never a Flax format. Blender stays an external DCC worker, never
vendored. Model weights and training-data licenses are a separate gate from
code licenses: weights stay fetch-only, never in git; unlicensed-generator output
is refused at the license gate before cook.

Determinism and threads: cook and stream jobs on workers; publication at frame
boundaries only; the sim never waits. Swap path uses integer compares and
stable actor order (mm distances, no floats, no hash-map iteration, no wall
clock); cook pins thread count 1 and posts the encoder version in the receipt.
The tick never calls a model, a generator or the network.

Budgets: stream <= 2 ms/frame amortized with a per-bundle memory cap posted;
LOD swap at most `MAX_SWAPS_PER_TICK` (8) per `poll_into` with pre-reserved
event logs and 0 per-tick allocs after construction; BC per-slot PSNR/SSIM
posted, a failing slot failing only its row. API (landed):

```rust
pub struct StreamBudget { pub bytes: u64, pub jobs: u32 }
pub fn request(id: AssetId, prio: u8); // never blocks the tick
pub fn publish(&mut self, frame_bytes: u64) -> Vec<StreamEvent>;
pub fn poll_into(&mut self, tick: u32, budget: usize) -> usize; // LOD swaps
```

## 4. Hard problems

Kept: the rows that gate the cook, the licence and the bundle (the other rows are in git history; their open work is in section 7).

| Problem | First approach | Why it works | Reference |
|---|---|---|---|
| Real locomotion bytes without license contamination | CT37 motions-only intake (100STYLE + CMU), Geno/LAFAN1 refusal tests kept green, credit rows in EVIDENCE + bake | synthetic bake already refuses Geno with `BakeError::Unlicensed` | `content/locomotion/ATTRIBUTION.md`, Zenodo 8127870 |
| Provenance that survives AI generation | CT28 release gate (URL+SPDX+author+SHA or generator+model+prompt digest) + CT38/CT40 per-file chains | gate is a cook-time refusal matrix (50 assets), not a doc check | PR1 receipts, CT4 license gate |
| Bundles that stay small and bit-identical | CT39 dedupe + CT11 compression + BC encode audit on the pin bundle, MB before/after, `fighter_bind` green | CT32 manifest names the 14 canonical clips; compression error-loop bounds drift | nfrechette/acl ideas (reimplemented, §5) |
| Claiming 80 without proving 80 | CT58 receipt counts 14 + batch-a/b/c + finisher against CT23 with audit + consumer wiring + `fighter_bind` green | receipt is a count with names, never a claim | CT32 manifest, CT24 audit, CT36 wiring |
| Untracked bytes read as landed | CT74 commit-gate: SHA-pin + license row per untracked file, provenance refusal rehearsal, cook x2, commit; CT45/CT57 read only the committed set | uncommitted bytes never count | CT46 matrix, CT52 rehearsal |

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. The licence table of this section is in `docs/archive/2026-10-10/plans/content-cut.md`.

## 6. Slim

Delete: per-asset-tuned gates (one provenance gate CT28, one sidecar schema CT33, one consumer test per reader). Merge: CT36 lands the CB4+LM2 consumers together on the audited sidecars (no separate combat/animation schema rows). Freeze: CT5 `lod_swap.rs`, CT2 LOD ratios, SP3 relation graph, UI2 overlay model — new rows add hunks beside them, never forks. What we will not build: no Blender rewrite in the engine, no bundled AI generator, no sculpting / retopology / UV / rigging tools in the editor, no runtime AI mesh generation in the shipped game, no marketplace or user-generated content, no second
cooker/registry/harness, no neural supersampling in the present loop, no
audio engine in content (code stays in `crates/audio`), no photogrammetry
cooker (CT67 is a bounded spike with a cap), no second map pipeline
(CT62/CT66 walk the CT9 kit through CT1/CT2/SP5).


### cook and delivery

- Delete: the TEMPORARY local mirrors in `stream.rs` become the documented `bundle.rs` edge in AS9; no second cooker, no second registry.
- Merge: AS8/CT18 budget rows merge into the existing PF1 workloads and baselines; no parallel perf harness, no second manifest format.
- Freeze: `bc.rs` codec, `LOD_BAND_MM`/`HYSTERESIS_MM`, and the museum `prove` shape; tuning rides staged tables, never new knobs.
- We will not build: handles and residency (AS4/AS5), USD (AS6), conformance (CT10), compression (CT11), manifests (CT12), proxies (CT13), motion source (CT14), sandbox (CT16). Those are open rows that keep their files and owners. OpenUSD after Finish A; no runtime recook (hot reload stays HR1 background recook with swap boundaries).
- Status 2026-10-12: AS4/AS5 are open B rows at docs/plan/content.md:220-221, bundle streams with placeholders at docs/plan/content.md:89-90 and queue to LodSwapper at docs/plan/content.md:144-146, so "will not build handles and residency" is false.
- Status 2026-10-17: disk shows the false line still at docs/plan/content.md:210, with AS4 "handles and load states | B" at docs/plan/content.md:222 and AS5 "one residency manager | B | AS4" at docs/plan/content.md:223 (each row one line lower than the 2026-10-12 cite of :220-221), bundle streams with placeholders at docs/plan/content.md:89-90 and the LodSwapper queue at docs/plan/content.md:144-146, so "will not build handles and residency" stays false.

## 7. Board tasks

Rows live on the board; slice any row with `node tools/board-slice.mjs <ID>`. Open rows of this plan (table from the board; `needs` is what each waits on). CT22 (establishing JSON) and CT28 (provenance gate) are plan-only, not on the board; they stay in git until boarded.

| ID | Title | Tier | Needs |
|---|---|---|---|
| CT29 | window fighter-bind staging | A | VS4 |
| CT12 | bake manifest | B | LATER |
| AS4 | handles and load states | B | - |
| AS5 | one residency manager | B | AS4 |
| AS6 | USD subset import | C | CT10 |
| CT21 | hero weapons-graft fix + re-cook | A | - |
| CT23 | combat + traversal clip expansion | A | - |
| CT24 | clip fight-data audit | - | CT23 |
| CT25 | crowd impostor kit | - | - |
| CT27 | 64-player + 1,000-agent soak on RTX 3050 6 GB | - | CT23 CT25 |
| CT31 | motion-DB style bake | A | - |
| CT32 | 14-canonical-clip dedupe manifest + before/after MB through AS1 bundles | A | - |
| CT33 | sidecar consumer schema + conformance test | A | - |
| CT34 | crowd 20k C3 scale card with P6 numbers on the RTX 3050 | A | CT25 |
| CT36 | sidecar consumer wiring | A | CT24 CT33 |
| CT37 | real-bytes locomotion intake | A | - |
| CT38 | crowd variety via the CT4 loop | C | CT25 |
| CT39 | bundle dedupe + compression audit on the pin bundle | B | CT11 CT32 |
| CT40 | AI prop intake | B | - |
| CT41 | LOD ms/VRAM table over the CT35 bake | A | - |
| CT42 | combat clip batch-a | A | - |
| CT43 | sidecar audit report for the canonical 14 + batch-a | A | CT24 CT33 |
| CT44 | crowd eyes + soak staging over the CT25 kit | A | CT25 |
| CT45 | PBR 2k + HDRI provenance report with VRAM caps | B | - |
| CT46 | provenance audit dry-run for the CT28 gate | A | - |
| CT47 | combat clip batch-b | A | CT42 |
| CT48 | locomotion real second slice | A | CT37 |
| CT49 | traversal edge markers | A | CT24 CT33 |
| CT50 | crowd soak staging report over CT25 + CT44 | A | CT25 CT44 |
| CT52 | provenance enforce rehearsal before the CT28 lock | A | CT46 |
| CT54 | combat clip batch-c | A | CT47 |
| CT55 | finisher-pair + death/reaction closure for | A | CT24 CT33 |
| CT56 | urban-war pilot corner through the same pipeline | B | - |
| CT57 | night-HDRI torch staging with VRAM report | A | - |
| CT58 | 80-clip | A | CT42 CT47 CT54 CT55 |
| CT59 | counted-clip consumer proof on the staged fight | A | CT36 CT58 |
| CT60 | joint establishing + window-bind capture proof | A | CT29 |
| CT61 | empty-stands P6 baseline for the CT27 soak | A | - |
| CT62 | district + siege stream staging for the C5 corner | B | - |
| CT63 | combat sound list with provenance | A | - |
| CT64 | reacting-crowd staging toward the C3 20k card | C | CT25 CT44 |
| CT65 | own-capture signature-move pilot | B | CT37 |
| CT66 | urban-war full kit after the CT56 pilot proves no fork | B | CT56 |
| CT67 | bounded splat-cook spike pilot | C | CT40 |
| CT74 | PBR/HDRI intake commit-gate for the untracked bytes | A | CT46 |
| CT75 | staged-dir triage into the batch digest queue | A | CT32 |
| CT76 | 100STYLE scout spike before the CT37 intake | A | - |
| CT77 | arena src commit-gate | A | - |
| CT78 | urban-war pilot scout on the CT9 kit | B | - |
| CT79 | digest-dedupe tool for every intake batch | A | CT32 CT42 |
| CT80 | studio HDRI vendoring for the Q1/WIN9 bin proof | A | - |
| CT82 | manifest single truth | A | - |
| CT10 | glTF conformance | A | - |
| CT11 | animation compression | B | - |
| CT13 | ray-tracing proxies | A | - |
| CT14 | AI motion source | B | CT12 |
| CT16 | sandbox boundary for untrusted content | B | - |


## 8. Quality gates


- `cargo test --locked -p engine-assets sidecar` green on every content row that touches clips (retarget tolerance 1e-4/1e-5, refusals typed).
- Same source + cook settings give the same cooked bytes (x2 gate on every row that writes bytes); bundle loads via AS1; corrupt bytes refused.
- Triangle/material/texture budgets checked at import (hero 40k, opp 25k, prop 5k, <= 4 mats; props <= 2k maps); over-budget cooks refused with counts (`over_budget_meshes_refused_with_counts` stays green).
- Release bundles refuse assets without provenance rows (CT28 matrix: 50 assets, URL+SPDX+author+SHA or generator+model+prompt digest).
- `fighter_bind` green on any row that moves fighter bytes; tampered-SHA and missing-body refusals stay green.
- Eyes: every new body/prop ships front/side/top previews against the 1.8 m figure (CT19 12/12 is the bar); untextured-grey preview style is stated, never hidden.
- Scale rows post adapter+driver+power-mode lines on the RTX 3050 6 GB (CT35/CT41 LOD ms/VRAM, CT34 20k card, CT27 soak, CT45 PBR bytes/VRAM, CT50 soak staging, CT51 load + VRAM); no number without the line.
- The golden hash never moves on content rows (presentation only); any tick byte drift kills the row.
- Forbidden bytes never land: Geno/LAFAN1/QAL-v1.0/Unreal-only refusal tests green; `content/local/` never committed.
- LADDER discipline: any spike behind these rows lands its numbers in `docs/research/LADDER.md` with fixture, baseline, candidate and hash check before the row is ticked; a REJECTED spike kills its row's approach, never the row's bar.

### cook and delivery
- Every cook row: identical bytes x2; corrupt/future/truncated refused
  (AS1 bar extended; CT17 matrix proves it with new fixtures).
- Streaming: budget enforced in tests (AS8 kill bars); tick-hitch fixtures
  fail loudly; AS9 keeps exactly one owner per budget constant.
- Placeholder honesty: tinted-wireframe convention asserted in a capture
  test; placeholders in any Q1 capture fail the wave.
- BC quality: per-slot PSNR/SSIM vs uncompressed posted (AS3 frozen; a
  slot below bar fails its row, not the whole cook).
- LOD stillness: 600-tick orbit at every band edge proves no oscillation
  (CT18); distance swap proven in match captures.
- Museum: every released format version loads, migrates, replays; a
  version without a fixture fails proof by construction (CT18).
- License rows per encoder/content batch (CT3/CT4 precedent); generators
  without a readable license refused.
- Counting-allocator floors read 0 steady-state allocs on the swap path;
  NCSA-avif grep gate in the window graph stays green.


## 9. Session estimate

Moved to git history: `git show d96091cd:docs/plan/content.md` (section 9) and `docs/archive/2026-10-10/plans/assets.md` (section 9). Each row carries its own estimate on the board.

