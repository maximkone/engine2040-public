# STEAL Dive 13 — Brief-to-game generation (seeded arenas, encounters, names) for EMPIRE outcome 4 (2026-09-26)

**Dive**: 13 (brief→game generation: seeded layout, WFC, procedural world, seeded text). Home: `crates/world`, `crates/ai` (AB3 encounters), `crates/endpoint` (AB1 brief intake, AB4 demo), `docs/plan/world.md` (WD), `docs/plan/living-world.md` (LW/AB), `docs/plan/game-ai.md` (GA).
**EMPIRE outcome 4** (AI makes whole games): every card ends in a brief→game consumer (AB1 intake → AB3 encounter/world tables → AB4 demo + EV2 eval, gated by SP spatial checks + VK1 keypoints). Generation that cannot serve "same brief + seed → same game" is rejected.
**Baseline**: hand-authored arena + AB3 validated encounter tables + SP5 relation solver; no seeded layout generator, no WFC, no seeded text grammar in the tree.
**Fixture**: one terse arena brief ("12 fighters, ruined forum, dusk raid") + seed 7 → generated layout + encounter + names → SP checks pass → EV2 rebuild byte-identical x2.
**Kill (dive)**: any float/RNG in the tick path; any unseeded generation; any layout that fails SP checks; any text generator that can emit unreviewed lore into the build.
**Method**: GitHub `search_repositories` (seeds: `wave function collapse rust`, `procgen rust`, `dungeon generator`, `bracket-lib roguelike tutorial rust`, `rust arena encounter spawner bevy`, `rust seeded RNG deterministic game`) + `github_get_file_contents` root listings + LICENSE body reads live 2026-09-26. No donor repeat: Dive-11 (`dolly`, `smooth-bevy-cameras`, `bevy_trauma_shake`, `saltmire-juice`, `Quad-Fighter-2`, `game-feel-hitstop-juice`), Dive-12/wave-36 (`nab`, `AegisForge-MCP`, `wirken`, `munal-os`, `boxagnts`, `jailguard`, `Quipu-Log`, `WasmVault`, `kprun`, `rust-agent-sandbox`, `prism`), wave-28 sandbox/auth, wave-41 crowds/nav, flax-mcp E-groups.
**deny.toml allow-list**: MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0.
**Access date**: 2026-09-26 (all URLs + LICENSE reads via `github_get_file_contents`, live).

## Ranked cards (best-first, 5) + 1 moonshot BET

### Card 1 — TAKE — `amethyst/bracket-lib` — seeded map-builder corpus + deterministic RNG as the AB3/WD layout discipline (AB8)

- **Question**: Does the donor's 80+-builder map corpus (cellular, BSP, Delaunay rooms, drunkard walk, maze, Voronoi) + seeded `RandomNumberGenerator` + map-level symmetry/noise helpers transfer as the *seeded, replay-exact* arena-layout generator behind AB3 encounters and WD world rows?
- **Engine home**: `crates/world/src/gen/` (new) feeding AB3 tables + SP5 solver. **Simpler baseline**: one hand-authored arena, no generator.
- **URL + rev + access**: https://github.com/amethyst/bracket-lib @ `ce9124a` (MIT, bracket-geometry/noise/random/pathfinding/algorithm-traits split, RLTK tutorial corpus), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` body read live — Bracket Productions 2019, on allow-list). Data/weights: none (tutorial maps are generated, no art taken). `PORT(amethyst/bracket-lib@ce9124a:bracket-random/src/,rltk/src/map_builders/)` (builder pipeline + seeded-RNG discipline only) + EVIDENCE row below.
- **Exact claim**: modular roguelike toolkit — `bracket-random` seeded RNG, `bracket-noise` (FastNoise port), `bracket-geometry`/`pathfinding`, and `rltk` map-builder chain (each builder takes a seed + produces rooms/corridors/spawns, composable as builder → symmetry → prefab passes).
- **Assumptions**: arena layouts (forum ruins, ludus yard, circus ring) factor as room/corridor/spawn placement over a tile grid the engine already navs (NV rows); seeded RNG can be reimplemented integer-side so same (brief, seed) → same layout bytes x2.
- **Counterevidence**: donor RNG/noise is float (`f32` FastNoise, `rand`-family floats) — none of that enters the tick; donor targets 2D roguelikes, not 3D arena sightlines/door-clearance; Bevy/terminal bindings irrelevant; tutorial code quality varies by builder.
- **What does NOT transfer**: float noise/RNG internals, terminal/Bevy renderers, tutorial game logic, tuning constants. Only the *pipeline discipline* transfers (seed → staged builders → symmetry/prefab passes → spawn table, all integer-reimplemented).
- **Cost / fixture / metric / budget**: M (2 sessions). Fixture: brief + seed 7 → 20 layouts; metric: 20/20 SP-clean (overlap/door-clear/nav-connect), seed-rerun byte-identical x2, generation ms. Budget: generation offline/import-time only, ≤ 200 ms per arena, 0 per-tick allocs, 0 float in tick.
- **Ablation/comparator**: hand-authored arena (baseline) vs seeded-builder layout (candidate) vs builder-without-symmetry-pass (must show degraded symmetry scores — proves passes matter). Success threshold: ≥ 15/20 layouts SP-clean first pass, 20/20 after solver repair, 100% rerun-identical.
- **Kill criterion**: any float RNG in tick path; any seed rerun divergence; < 15/20 SP-clean; generation > 200 ms; any builder that needs its renderer to validate.
- **Board row**: `[ ] AB8 world: seeded arena-layout builder chain behind AB3. Tier B. Size M. Owns gen hunks beside crates/world/src/gen/ (AB3 tables frozen). Accept: brief+seed 7 → 20 layouts, ≥15/20 SP-clean first pass and 20/20 after repair; rerun x2 identical; gen ≤ 200 ms; 0 tick float. needs: AB3, SP5, NV3.`
- **Falsifiable experiment + consumer**: reimplement 3 builders (rooms+MSt corridors, cellular ruins, drunkard rubble) integer-side with a mock seeded RNG; run fixture x2; consumer is the AB3 encounter author + EV2 rebuild. Killed if reruns diverge or SP-clean < 15/20.
- **EMPIRE outcome 4**: seeded builders are the "brief goes in, arena comes out" step — without them every generated game is the same hand-made room.

### Card 2 — TAKE — `AustinHellerRepo/WaveFunctionCollapse` — generic node-constraint collapse as the tile/room solver (WD14)

- **Question**: Does a generic graph-based WFC solver (nodes + restrictions → collapsed state, selectable algorithm) transfer as the constraint solver for arena tilings (wall/rubble/gate/door tiles) and room-adjacency graphs behind WD14?
- **Engine home**: crates/world/src/gen/wfc.rs (planned) (behind Card 1 builders). **Simpler baseline**: Card 1 builders alone with hand-written adjacency repair.
- **URL + rev + access**: https://github.com/AustinHellerRepo/WaveFunctionCollapse @ `f447dc1` (23★, Rust, benches + examples + `src/`), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE` listed at root; MIT body read live — on allow-list). Data/weights: none (example inputs are constraints, operator-authored). `PORT(AustinHellerRepo/WaveFunctionCollapse@f447dc1:src/)` (propagate-and-collapse discipline only) + EVIDENCE row below.
- **Exact claim**: converts nodes and their restrictions into a collapsed node state under a selectable algorithm; ships `benches/`, `examples/`, graph-oriented (not bitmap-only) design.
- **Assumptions**: arena tiling constraints (gate needs wall-neighbors, rubble clusters, door clearance) express as node restrictions the engine authors; collapse runs offline at generation time with a seed + backtrack cap.
- **Counterevidence**: 23★, self-reported bench only; graph-WFC can non-terminate or need backtracking storms on tight constraints; donor RNG/selection order may be float- or time-seeded; overlapping-model tuning is taste, not science.
- **What does NOT transfer**: donor RNG internals, benchmark numbers, example tilesets. Only *observe → propagate → collapse with cap* transfers, reimplemented over integer seeds.
- **Cost / fixture / metric / budget**: S (1 session). Fixture: 3x3–9x9 forum tiling + room-adjacency graph, 20 seeds; metric: solve rate, ms per solve, SP-clean rate. Budget: offline only, ≤ 100 ms p50 per arena, backtrack cap fixed, 0 tick code.
- **Ablation**: builder+repair (baseline) vs builder+WFC (candidate) vs WFC-without-propagation (must thrash — proves propagation matters). Threshold: WFC solves ≥ 18/20 with fewer repair passes than baseline.
- **Kill criterion**: any non-termination past the cap; any unseeded choice; solve < 18/20; p50 > 100 ms; any tick-path use.
- **Board row**: `[ ] WD14 world: node-constraint collapse for arena tilings. Tier B. Size S. Owns solver hunks beside crates/world/src/gen/wfc.rs (builders frozen). Accept: 20 seeds ≥18 solved; p50 ≤ 100 ms; SP-clean ≥ baseline; rerun x2 identical; cap holds. needs: AB8.`
- **Falsifiable experiment + consumer**: reimplement propagate-and-collapse over a 9x9 tile set with integer-seeded choice; run 20-seed fixture x2; consumer is the WD arena author. Killed if propagation-less control solves as well (then WFC adds nothing) or cap fires > 2/20.
- **EMPIRE outcome 4**: WFC turns "a ruined forum" from one prefab into a family of SP-valid variants — the variety half of brief→game.

### Card 3 — TAKE — `bones-ai/rust-procedural-world` — Bevy procedural-2D-world staging as the arena-ground bake pattern (WD15)

- **Question**: Does the donor's Bevy procedural-2D-world staging (seeded terrain passes → staged generation → rendered preview) transfer as the *bake-time* arena-ground pattern (courtyard dirt, rubble scatter, worn paths) behind WD15, with the engine's own integer ground truth?
- **Engine home**: `crates/world/src/ground/` bake step. **Simpler baseline**: flat arena floor + hand-placed rubble.
- **URL + rev + access**: https://github.com/bones-ai/rust-procedural-world @ `9bf8c01` (86★, Rust+Bevy, procgen/terrain-generation, updated 2026-08-01), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` body read live — Bones 2025, on allow-list). Data/weights: none (screenshot only, not taken). `PORT(bones-ai/rust-procedural-world@9bf8c01:src/)` (staged-bake discipline only) + EVIDENCE row below.
- **Exact claim**: procedural 2D world generation in Rust + Bevy with staged passes and a preview screenshot; `src/` + `assets/` layout.
- **Assumptions**: ground detail factors as staged passes (base → wear → scatter) that bake to deterministic tile/prop tables; Bevy staging maps to engine bake steps, never to tick code.
- **Counterevidence**: 86★ demo-scale; Bevy `f32` worldgen + render preview are presentation-only; no determinism, replay, or SP-gating claims; asset contents unaudited (not taken).
- **What does NOT transfer**: Bevy systems, `f32` noise, render code, assets. Only the *pass-staging* (ordered, seeded, inspectable passes with a preview capture) transfers.
- **Cost / fixture / metric / budget**: S. Fixture: 10 ground bakes from brief+seed; metric: rerun-identical x2, SP/nav-clean (no blocked gates/paths), bake ms. Budget: bake ≤ 100 ms, 0 tick float.
- **Ablation**: flat floor (baseline) vs staged bake (candidate) vs single-pass scatter (must look uniform — proves staging matters). Threshold: staged bake rerun-identical 10/10 + playtester prefers staged in blind A/B.
- **Kill criterion**: any rerun divergence; any blocked path/gate; bake > 100 ms; any ground float reaching the tick.
- **Board row**: `[ ] WD15 world: staged arena-ground bake. Tier B. Size S. Owns bake hunks beside crates/world/src/ground/ (floor frozen). Accept: 10 bakes rerun-identical x2; 0 blocked paths; bake ≤ 100 ms; blind A/B prefers staged. needs: AB8, NV3.`
- **Falsifiable experiment + consumer**: implement 3 integer passes (base/wear/scatter) + capture; run fixture x2; consumer is the arena-ground author + WIN1 captures. Killed if single-pass control is indistinguishable or any bake diverges.
- **EMPIRE outcome 4**: ground is what makes 20 generated arenas stop looking like the same floor — cheap variety with a determinism receipt.

### Card 4 — TAKE — `rant-lang/rant` — seeded procedural-text templates as the brief→name/flavor compiler (AB9)

- **Question**: Does a Rust procedural-text templating language (seeded RNG, dictionaries, conditional blocks — 214★) transfer as the *deterministic* name/flavor compiler for brief→game output (fighter names, ludus lines, encounter titles), with every output recorded on the replay tape?
- **Engine home**: `crates/endpoint/src/flavor/` (AB1 brief intake side) + LW records. **Simpler baseline**: fixed name lists + format strings.
- **URL + rev + access**: https://github.com/rant-lang/rant @ `caf374f` (214★, Rust, `src/` + `tests/` + `examples/`, updated 2026-09-17), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE` at root; MIT body read live — Robin Pederson 2022, on allow-list). Data/weights: none (dictionaries are operator-authored). `PORT(rant-lang/rant@caf374f:src/)` (seeded-template discipline only, never the interpreter bytes) + EVIDENCE row below.
- **Exact claim**: templating language for procedural generation — probabilistic text with dictionaries, repeaters, conditionals, seeded runs; ships tests + examples + changelog.
- **Assumptions**: flavor text factors as seeded templates over closed dictionaries (no open LLM output at build time); the engine reimplements a *tiny* template subset (choice/repeat/condition), never vendors the interpreter; outputs are recorded like any provider output.
- **Counterevidence**: full interpreter weight (parser, runtime, float RNG paths) is unwanted as a dependency; probabilistic text can emit lore that contradicts LW records unless dictionaries are closed and outputs reviewed; donor targets general procgen text, not game-canon text.
- **What does NOT transfer**: the interpreter crate, its RNG internals, stdlib blocks. Only the *closed-dictionary + seeded-choice* discipline transfers.
- **Cost / fixture / metric / budget**: S. Fixture: 50 briefs → names/titles; metric: rerun-identical x2, 0 canon contradictions vs LW records, ms per brief. Budget: intake-time only, ≤ 5 ms per brief, 0 tick code.
- **Ablation**: fixed lists (baseline) vs seeded templates (candidate) vs open-vocabulary templates (must contradict canon — proves closed dictionaries matter). Threshold: 50/50 rerun-identical, 0 contradictions, variety (unique-name rate) ≥ 2x baseline.
- **Kill criterion**: any rerun divergence; any canon contradiction; any unreviewed lore reaching the build; intake > 5 ms.
- **Board row**: `[ ] AB9 endpoint: seeded flavor-text compiler for brief output. Tier B. Size S. Owns compiler hunks beside crates/endpoint/src/flavor/ (intake frozen). Accept: 50 briefs rerun-identical x2; 0 canon contradictions; variety ≥ 2x lists; ≤ 5 ms. needs: AB1, LW6.`
- **Falsifiable experiment + consumer**: reimplement choice/repeat/condition over a closed 200-word dictionary; run fixture x2; consumer is the AB1 brief-intake author. Killed if fixed lists already hit the variety bar (then templates are vacuous).
- **EMPIRE outcome 4**: generated games need generated names — seeded templates give every fighter a name that replays.

### Card 5 — TAKE — `greysquirr3l/morgan-bevy` — BSP+WFC editor export pipeline as the bake-and-verify pattern (CT17)

- **Question**: Does a Bevy 3D level editor advertising procedural BSP + WFC, transform gizmos, asset management and multi-format export transfer as the *bake-and-verify* content pipeline pattern (edit → procedural fill → export → engine-side verify) behind CT17?
- **Engine home**: content bake path (`crates/assets` cooker side, CT rows). **Simpler baseline**: hand-modeled arena imports with cooker checks only.
- **URL + rev + access**: https://github.com/greysquirr3l/morgan-bevy @ `3da1f27` (8★, Tauri+React+Three.js+Rust, `crates/` + `src-tauri/`, updated 2026-09-15), accessed 2026-09-26.
- **Licenses**: code MIT + Apache-2.0 (dual `LICENSE-MIT` + `LICENSE-APACHE`; MIT body read live — Nick Campbell 2026, on allow-list). Data/weights: `Assets/` demo content not taken. `PORT(greysquirr3l/morgan-bevy@3da1f27:crates/)` (pipeline discipline only) + EVIDENCE row below.
- **Exact claim**: professional 3D level editor for Bevy with procedural generation (BSP, WFC), gizmos, asset management, multi-format export; Tauri + React + Three.js + Rust workspace.
- **Assumptions**: the useful transfer is process, not code — procedural fill happens in the editor, export is a dumb interchange, the engine re-verifies (SP checks + cooker rules) on import; Tauri/React/Three.js never enter the engine.
- **Counterevidence**: 8★, editor-scale claims unverified here; multi-format export risks format drift (needs the LT1 museum rule); BSP/WFC internals unaudited; heavy non-Rust stack (TS/React/Tauri) is a non-starter as a dependency.
- **What does NOT transfer**: the editor, UI stack, exporters' code, gizmo math. Only *generate-in-editor → export → engine-verifies* transfers.
- **Cost / fixture / metric / budget**: S. Fixture: 5 exported arena shells → engine import; metric: import acceptance rate, SP-clean rate, cooker ms. Budget: import-time only, ≤ 500 ms per shell.
- **Ablation**: hand-modeled import (baseline) vs procedural-filled export (candidate) vs export-without-engine-verify (must let a bad shell through — proves verify matters). Threshold: verify catches 100% of planted defects (overlap/float/scale).
- **Kill criterion**: any unverified shell reaching the build; any format that cannot round-trip through the museum rule; import > 500 ms.
- **Board row**: `[ ] CT17 assets: procedural-fill export with engine-side verify. Tier B. Size S. Owns verify hunks beside the cooker (cook frozen). Accept: 5 shells import; 100% planted defects caught; import ≤ 500 ms; rerun-identical. needs: CT1, SP5.`
- **Falsifiable experiment + consumer**: author 5 shells with planted defects, run engine verify x2; consumer is the CT cooker + AB4 demo import. Killed if verify adds no catches beyond CT1 (then fold into CT1, no new row).
- **EMPIRE outcome 4**: editors that procedurally fill + engines that verify are how brief→game output stays shippable — generation without verification ships broken arenas.

### Moonshot BET — `majidmanzarpour/threejs-procedural-dungeon` — deterministic Delaunay/MST dungeon + themed dressing as the arena-variant engine (BET-AB10)

- **Bet**: a deterministic-seed dungeon pipeline (Delaunay triangulation → MST graph layouts → room semantics → five themes → bloom/tilt-shift presentation) can be reimplemented integer-side as the arena-variant engine: same brief + seed → same graph → same rooms → same themed dressing, with presentation (bloom/tilt-shift) staying capture-side only.
- **Baseline**: Card 1 builders (rooms + corridors, unthemed).
- **Metric**: (a) 20 seeds → 20 connected layouts, 100% rerun-identical x2; (b) MST keeps corridor length ≤ 1.3x Delaunay-total on the fixture; (c) 5 themes dress the same graph without moving walls/doors (0 SP regressions); (d) presentation (bloom/tilt-shift) changes captures only, never sim bytes.
- **URL + rev + access**: https://github.com/majidmanzarpour/threejs-procedural-dungeon @ `0a2aa09` (504★ + 83 forks, JS/Three.js, updated 2026-09-26 — freshest donor this dive), accessed 2026-09-26.
- **Licenses**: code MIT (`LICENSE` body read live — Majid Manzarpour 2026, on allow-list). Data/weights: none taken (themes re-authored as engine palettes). `PORT(majidmanzarpour/threejs-procedural-dungeon@0a2aa09:src/)` (seed→graph→rooms→themes discipline only) + EVIDENCE row below.
- **Claim / assumptions / counterevidence / no-transfer**: claim — real-time generator with deterministic seeds, Delaunay/MST layouts, room semantics, five themes, custom bloom/tilt-shift pipeline. Assumptions: graph layout factors integer-side (fixed-point Delaunay/MST over room centers); themes are palette/prop maps, never geometry moves. Counterevidence: JS/Three.js float geometry + GPU post stack — none enters the tick; 504★ is web-demo popularity, not determinism proof; Delaunay/MST over float centers risks cross-platform divergence unless reimplemented integer-side — hence moonshot, not Tier A. No-transfer: Three.js scene code, bloom/tilt-shift shaders, JS RNG — only seed→graph→rooms→themes transfers.
- **Kill rule**: any rerun divergence x2; any disconnected layout; any theme moving a wall/door (SP regression); any post effect touching sim bytes; corridor blowup > 1.3x on the fixture.
- **Board row (BET)**: `[ ] BET-AB10 world (MOONSHOT): deterministic graph-layout arena variants with themes. Tier C. Size M. Owns layout hunks beside crates/world/src/gen/ (AB8 frozen). Accept: 20/20 connected + identical x2; corridor ≤ 1.3x; 5 themes 0 SP regressions; post capture-side only. Kill: divergence, disconnect, theme-moves-wall, sim-byte change, blowup. needs: AB8, SP5, NV3.`
- **Falsifiable experiment + consumer**: reimplement seeded room-center → integer Delaunay/MST → rooms → 2 themes on the Card-1 fixture x2; consumer is the AB4 demo variant reel + EV2 rebuild. Killed by the rule above.
- **EMPIRE outcome 4**: the bold version of "AI makes whole games" — one brief grows a season of arenas, every one replay-exact.

## Rejects (one-line reasons)

- `Nightwishtomer/Procedural-Dungeon-Map-Generator-Rust-` (1★, room-based Diablo-style): REJECT — no LICENSE file at root (full listing 2026-09-26) → UNVERIFIED, never a code donor; idea overlaps Card 1 with no new gap.
- `pprobst/roguelike` (8★, archived bracket-lib roguelike, "tons of dungeon procgen"): REJECT — no LICENSE file at root (listing 2026-09-26) → UNVERIFIED; ideas overlap Card 1, revisit only with a license + played-layout numbers.
- `vazgriz/DungeonGenerator` (832★, Unity C#, MIT verified live): REJECT — C# Unity room/corridor/T-junction patterns add no falsifiable gap beyond Cards 1+5 (both seeded + verified); cite as idea-only if AB8 needs a second witness.
- `radium-io/sanity` (22★, Rust+Amethyst roguelike, WFC tag): REJECT — game, not a consumer crate/library; WFC tag adds no solver gap beyond Card 2.
- `Llywelwyn/rust-rl` (4★, rltk/bracket-lib roguelike): REJECT — downstream of Card 1, no new pattern.
- `OutOfTheVoid/kahuna`, `madwareru/simple-tiled-wfc`, `DVDTSB/wave-function-collapse`, `math-fehr/fast-wfc-rs`, `BonsonW/wfc_voxel` (all Rust WFC): REJECT — no new solver gap beyond Card 2's graph formulation this pass; revisit with solve-rate numbers on the WD14 fixture.
- Dive-11/12 donors listed in Method: not re-swept — no new evidence.

## EVIDENCE rows (proposed)

| Idea | Source | Verdict |
|---|---|---|
| Seeded builder chain (rooms/cellular/walk + symmetry/prefab passes, integer RNG) | `PORT(amethyst/bracket-lib@ce9124a:bracket-random/src/)` MIT, https://raw.githubusercontent.com/amethyst/bracket-lib/ce9124ad744553b2576e44df44e94b0f72faa8e8/LICENSE, verified 2026-09-26 | IN — AB8 |
| Node-constraint propagate-and-collapse with backtrack cap | `PORT(AustinHellerRepo/WaveFunctionCollapse@f447dc1:src/)` MIT+Apache-2.0, https://raw.githubusercontent.com/AustinHellerRepo/WaveFunctionCollapse/main/LICENSE-MIT, verified 2026-09-26 | IN — WD14 |
| Staged ground-bake passes with preview capture | `PORT(bones-ai/rust-procedural-world@9bf8c01:src/)` MIT, https://raw.githubusercontent.com/bones-ai/rust-procedural-world/main/LICENSE, verified 2026-09-26 | IN — WD15 |
| Closed-dictionary seeded text templates (choice/repeat/condition) | `PORT(rant-lang/rant@caf374f:src/)` MIT+Apache-2.0, https://raw.githubusercontent.com/rant-lang/rant/master/LICENSE-MIT, verified 2026-09-26 | IN — AB9 |
| Generate-in-editor → export → engine-verifies pipeline | `PORT(greysquirr3l/morgan-bevy@3da1f27:crates/)` MIT+Apache-2.0, https://raw.githubusercontent.com/greysquirr3l/morgan-bevy/main/LICENSE-MIT, verified 2026-09-26 | IN — CT17 |
| Deterministic seed→Delaunay/MST→rooms→themes variant engine | `PORT(majidmanzarpour/threejs-procedural-dungeon@0a2aa09:src/)` MIT, https://raw.githubusercontent.com/majidmanzarpour/threejs-procedural-dungeon/main/LICENSE, verified 2026-09-26 | BET — AB10 moonshot |

## Unverified

- No code cloned or run; all takes are pattern reimplementations (Rust, integer-seeded, offline/import-time, tick-float-free) — REPRODUCE stamps require the listed experiments.
- Donor numbers (stars, "80+ builders", solve rates, theme counts, bake ms) are donor/README claims, not engine measurements; engine values come from the Dive-13 fixture, not donor pages.
- `PORT` revs pin listing SHAs captured this pass (`ce9124a`, `f447dc1`, `9bf8c01`, `caf374f`, `3da1f27`, `0a2aa09`); re-pin to full commit SHAs before any port.
- rant/morgan/bevy-adjacent donors carry float RNG/render paths — the experiments are designed to fail closed to hand-authored content if integer reimplementation diverges.
- vazgriz MIT `LICENSE.md` read live but rejected regardless, so no take depends on it.
