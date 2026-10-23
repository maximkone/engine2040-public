# Must-audit: the finish-line must list against 415bdc916

Date 2026-10-23. Scope: the ten must items in `docs/plan/finish-line.md`
"What we still lack" (A1, B7, B8, B9, C10, C13, D19, E22, F24, F25).
No row, board or code file was edited; nothing was rebuilt.

How this was checked:

- Row text: `node tools/board-slice.mjs <ID>`; parked rows from
  `docs/plan/parked-2026-10-22.md`.
- Landed shas: `git merge-base --is-ancestor <sha> 415bdc916`; artifacts:
  `git ls-tree` and `git grep` at that commit.
- **MET** = the row's Accept has its landed commit, its named test or
  artifact, and its proof run id in the row, all on 415bdc916. GPU, browser
  and ship tests were not re-run, so MET is evidence-on-HEAD, not a fresh
  green.
- **OPEN** = a named leg cannot be shown on HEAD; it is in the last column.

HEAD moved while auditing: `78ca02801` (rustfmt), `1c52b7a3f` (board) and
`e6be7d28a` (adds `tools/reference-web.mjs`, no web PNGs) sit on top of
415bdc916. None of them moves a verdict below.

| Item | Owning row(s) | Latest proof on 415bdc916 | Verdict | Missing leg |
| --- | --- | --- | --- | --- |
| A1 | PW37 owns; WEB15/16/17 ticked; WEB5 open (wasm split, not look) | `6c767d8`; golden run 39564307544; clippy 39565641494; receipt `scene=arena-kit kit=11/11 bodies=2/2`, `frame gate: ssim=0.9992 bar=0.95 verdict=pass`; stand-in ring 0.452 | **MET** | none; WEB5's feature split is a precondition, not look work |
| B7 | PW03 (parked), LF3, LF4 `99e624b`, LF5 `122e05dea`, LF6 `f0975158b`, LF7 `61cc048a3`, EG11 `1d01a41`, EG12 `356cb445a`, EG38 `3253dbcbd`, RL61 (not ticked), RL36 (open) | EG12 `356cb445a` (2026-10-23 live torch path); LF5 run 39950874767; RL61 run 39930897590 | **OPEN** | cull, sort and instancing in the live loop: the sphere cull is tests-only (EG38 (a), `DrawItem`/`GpuMesh` carry no bounds); no `prepass.wgsl` (normal target absent, PW03) and `present/gpu_present/submit.rs` has no TAA call |
| B8 | PW14 | `3712cceec` + diff `912a44723`; look run 39739553579 (76 passed); golden 39740514789/39741714317 | **OPEN** | 10 captures per tier: `content/reference/index.json` holds native 11 rows (real `arena600-t000..t540`), web 1, potato 1 |
| B9 | ENG20 (`docs/SPRINT-31-research.md:95`, not ticked) | `846aa1cf0`; `tools/temporal-gate.mjs` + test on HEAD | **OPEN** | a labeled TAA-on/off capture pair so the metric separates them (rz2-ab is ragdoll on/off, different ticks); CGVQM+D dropped by the lead 2026-10-22 |
| C10 | QR12 `515bd14c`, RZ4 `c661bf63`, RZ6 `3fb8e4194`, AD30 (open) | RZ6 runs 39940872541/39940837795/39940842721; QR12 run 39300193269 | **OPEN** | the four fx door verbs (AD30); no `body.toml` in the tree |
| C13 | PW41 (parked, ticked) `87f895e85`, overlay `e39dd7a`, game console (`crates/game/src/console.rs`) | PW41 `87f895e85`; overlay `e39dd7a` | **OPEN** | a caller for `crates/render/src/debug_draw.rs` (tests and `pub mod` only), the leg the item names |
| D19 | PW22 (parked, open, `docs/SPRINT-28-powerups.md:150`) | none | **OPEN** | a per-instance verdict from each build verb; the three build verbs carry only a `verify` string (item 16) |
| E22 | ENG19 | `85d5d5d26` + `88ca9b8dc`; mirror 39865729635 green; `SHIP-RUN tick=600 hash=d57b5c8ae3096614`; 30 checks, broken receipt red | **OPEN** (row Accept holds) | the game played and the frame compared: the log prints `SHIP-GAME skipped adapter=none` and `SHIP-FRAME skipped ... reference-png-not-on-disk` |
| F24 | ENG9 | `0a6e86fe8`; runs 39864469448/39864469519/39864469593 | **OPEN** (row Accept holds) | the shipped game never loads through `PatchedSource`; `git grep` in `crates/game` and `crates/web` finds no hit |
| F25 | ENG16 | `f27f6f9c8`; `engine-perf` `crash_receipt` 6 passed | **OPEN** (row Accept holds) | the shipped bin is `arena.exe` (targets: `arena`, `staged_fight`) and installs only `crash::install_run_hook`; the run receipt sits in `windowed.rs`, which no shipped target builds |

Reading: 1 MET, 9 OPEN. Where the row Accept holds but the item does not
(E22, F24, F25), the gap is one call site or one CI step, not new machinery.

Cheapest closes, in proof order: B8 (capture web and potato tiers from the
recipes that already pass), C13 (one debug-draw call in the live submit),
F24 and F25 (one call site each), D19 (needs a live board row: PW22 is
parked). B7 and E22 need a real run: EG38 (a) wiring and a GPU adapter in
the ship job.
