<!-- checked 1d01a410 2026-10-16T06:44Z -->
# SLICE — the arena duel scorecard (director seat)

One number per pillar. Commands copied from `docs/plan/finish-line.md` G1-G9 and
`docs/FOCUS.md` Now "Proof:". Numbers read from the newest `target/reports/` files;
no builds were run, so cargo rows stay NOT RUN unless a report prints them.

| Pillar (FOCUS Now 1-7) | Command that prints its number | Today's number (2026-10-16T06:44Z, HEAD 1d01a410) | Bar | Stage | Exit test |
|---|---|---|---|---|---|
| Look | `node tools/finish.mjs A4 A5 A8` (exit 0, ran 2026-10-16T06:44Z) | A4 partial (10/12 open: Q1 GX1-GX4 GX6-GX10, GX5 dropped), A5 partial (8/10: GM3 WIN1 MP1 MP2 MP3 GP1 PX1 DX1), A8 open (1/1 GM3); no look frames at 5255d8f5: newest `run-5255d8f5-20261016T0436Z.md`: golden x2 MATCH `d57b5c8ae3096614` on FRESH engine2040, look exit 102 forward-pipeline Location[3] panic, 0 PNGs, framing NONE; `run-5255d8f5-20261016T0429Z.md`: golden x2 MATCH at 15a7d1c3 and 5255d8f5; `perf-2026-10-16T0405Z.md`: BLOCKED 3x exit 102 same panic; no report yet at HEAD 1d01a410 | product-bar targets: live passes each change pixels (EG8), look score at/above Q1 bar | Preproduction | framing empty 0/300, validation_errors 0, live pass ms above 0 |
| Bodies | `cargo test -p engine-proof break_corpus` x2 + HS2 matrix (NOT RUN, no builds) | UNKNOWN (no fresh corpus or matrix report; spam-path sim stable: `playtest-5255d8f5-20261016T0437Z.md` light/heavy digests identical c88d0ca to 5255d8f5, `playtest-5255d8f5-20261016T0424Z.md` t60/t600 stable, `playtest-15a7d1c3-20261016T0411Z.md` 140-spam stable ea1aa5a to 15a7d1c3; golden x2 MATCH at 5255d8f5 per `run-5255d8f5-20261016T0436Z.md` and `run-5255d8f5-20261016T0429Z.md`; no report at 1d01a410) | G6 same hash: break corpus x2 + HS2 matrix; pops 0, sunk under 2 cm | Preproduction | corpus x2 green, HS2 matrix green |
| AI door | `cargo test -p engine-playtest --lib mirror` x2 (NOT RUN this run, no builds) | Last green stands, no newer report today: HS29 green at 1d2c63ba per `qa-hs29-1d2c63ba-20261014T2152Z.md` (newest qa report on disk); HS20 green at 37a7894d (30/30 parity, mirror 44 passed x2); HS30 green at 1bcd59c (mirror_craft 7 passed x2); finish A6 open 8/8 (ran 06:44Z exit 0); no report at 1d01a410 | G7 same task, same result on mock, local, remote (LF16) | First Playable | mirror x2 green, parity holds |
| Feel | each row's Accept (FOCUS Now 5; NOT RUN, no builds) | UNKNOWN (no feel Accept run today; newest C6 at 5255d8f5 per `playtest-5255d8f5-20261016T0437Z.md`: small/big FAIL, light/heavy determinism PASS; newest C1 at 5255d8f5 per `playtest-5255d8f5-20261016T0424Z.md`: first-hit visible FAIL, sim to 600 PASS, no drift; newest C5 at 15a7d1c3 per `playtest-15a7d1c3-20261016T0411Z.md`: stamina-out feedback FAIL, 140 spam inputs stable) | G5: 0 pops per 300 frames, feet planted, hits land (hold, shake, trail) | Preproduction | `arena view:` pops 0, sunk_cm_max under 2 |
| Multiplayer | `cargo test -p engine-net --lib snapshot` (NOT RUN, no builds) | UNKNOWN (no snapshot report on disk today; newest net file `res-ioq3-net-20261015.md` is research notes only, no test evidence) | G8: the duel on LAN and internet, one hash (MP6, ON30 played bouts) | Preproduction | snapshot green, played bouts MP6 ON30 |
| Potato | `node tools/potato-check.mjs` (NOT re-run this run: newest on-disk report 04:27Z stands; the arena run exceeds the tool stop) | adapter UNKNOWN (NO_IGPU), p50 UNKNOWN, p99 UNKNOWN, peak_ws 278 MB PASS (<= 3072) per `target/reports/potato-2026-10-16.md` (04:27Z, arena ran and exited); no report at 1d01a410 | G9: Intel UHD p50 50 ms, p99 80 ms, working set 3 GB; heap peak flat; assets inside potato caps | Preproduction | potato-check green on the Intel UHD |

Row verdicts, cut log and landscape dry-run are below. Judge verify for this run: `node tools/board-slice.mjs --focus --open` exit 0 (ran 2026-10-16T06:44Z, first 20 IDs FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64 in the same order as the 01:06Z check) plus `node tools/workspace-check.mjs` exit 0 (ran 2026-10-16T06:44Z, PASS, no new FAIL) plus `node tools/finish.mjs A4 A5 A8` exit 0 (ran 2026-10-16T06:44Z, scorecard refreshed this run), all with no builds; cargo rows stay NOT RUN by seat rule. Scorecard table refreshed this run (slice claimed 06:44Z 10-16, eligible: last slice claim 01:06Z 10-16 outside 2 h; rowverdicts claimed 01:22Z and dry claimed 01:36Z, both outside 2 h but deferred by the one-item rule, so the row-verdict and dry-run sections below are left untouched). Verdict source: `node tools/board-slice.mjs --focus --open` exit 0 (open rows listed, HEAD now 1d01a410).

## Row verdicts (first 20 open FOCUS rows, 2026-10-16, re-verified at 0e393c0)

Every row must answer CONCEPT line 1 (readable decisive arena duel) through one of its eight values. A row serving no value is PARK or CUT. All 20 below serve a value, so all KEEP (same 20 IDs in the same order as the 01:06Z check: FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64; re-verified `node tools/board-slice.mjs --focus --open` exit 0 at 0e393c0, ran 2026-10-16T01:24Z). Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Scorecard table above left untouched this run (slice claimed 01:06Z, inside 2 h; dry claimed 21:18Z 10-15, outside 2 h but deferred by the one-item rule, so this section refreshed).

| Row | Value served | Verdict | Type |
|---|---|---|---|
| FC1 | Experience: frame cost keeps the duel readable at speed | KEEP | process |
| EG8 | Emotion: pass-truth stops spectacle hiding a tell | KEEP | process |
| SP63 | Experience: GPU numbers make readability measurable | KEEP | process |
| LF1 | Theme: sunlit sky over the Colosseum | KEEP | creative |
| LF2 | Theme: IBL sunlit material response | KEEP | creative |
| EG11 | Theme: sun shadow grounds fighters in the arena | KEEP | creative |
| EG12 | Theme: torches and maps keep night bouts readable | KEEP | creative |
| LF3 | Emotion: GTAO depth without hiding a tell | KEEP | creative |
| LF4 | Emotion: restrained bloom that never hides a tell | KEEP | creative |
| EG13 | Experience: MSAA keeps thin edges readable | KEEP | creative |
| EG14 | Experience: HUD reads decisive round state | KEEP | creative |
| EG18 | Experience: animation from sim keeps swings readable | KEEP | creative |
| RL21 | Context: twin path proves the arena duel first | KEEP | process |
| VS1 | Theme: arena kit builds the Colosseum | KEEP | creative |
| FC3 | Experience: 30 fps bar keeps the bout decisive | KEEP | process |
| EG45 | Challenge: balance verdicts punish spam | KEEP | creative |
| EG19b | Decisions: AttackStarted tell answers every attack | KEEP | creative |
| HS2 | Experience: one hash proves decisive on every PC | KEEP | process |
| SP61 | Experience: view truth without GPU keeps reads honest | KEEP | process |
| SP64 | Experience: props and light in the view truth keep reads honest | KEEP | process |

## Cut log

No cuts this run (director, 2026-10-16T01:24Z: all 20 serve a value, so no PARK or CUT; re-confirmed at 0e393c0, first 20 IDs unchanged in the same order). Board untouched by seat rule.

## Landscape dry-run (first 10 LI rows, 2026-10-16)

Dry run of `docs/plan/landscape-ideas.md` section A (LI1-LI10) against the eight CONCEPT values. Default PARK: nothing here is a row yet (Tag LATER in `docs/plan/landscape-ideas.md`); each idea needs one spike (one test, one baseline, a kill rule) before it becomes a GS row. Kill rule for every cluster: no change in the number it targets, or more than 1 ms / 50 MB on the potato tier, deletes the idea. A row serving no value is PARK or CUT; all 10 below serve at most one value and stay PARK. Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Source: `docs/plan/landscape-ideas.md:13-22`, CONCEPT eight values in `docs/design/CONCEPT.md:3-10`. Re-verified 2026-10-16T01:36Z at b663f924: same 10 IDs in the same order, lines still on disk so none promoted to a GS row, all stay PARK (`node tools/board-slice.mjs --focus --open` exit 0, ran 2026-10-16T01:36Z, first 20 FOCUS IDs unchanged in the same order as the 01:22Z check; `node tools/workspace-check.mjs` exit 1, ran 2026-10-16T01:36Z, pre-existing FAILs only, no new FAIL, no builds). Scorecard table and row-verdict sections above left untouched this run (slice claimed 01:06Z and rowverdicts claimed 01:22Z, inside 2 h; dry last claimed 21:18Z 10-15, outside 2 h, so this section refreshed).

| LI | Value served | Verdict | Type | Why PARK |
|---|---|---|---|---|
| LI1 Temporal upscaler (720p render, EASU/RCAS plus temporal) | Experience: frame cost keeps the duel readable at speed (G2) | PARK | process | Backlog; needs spike proving fps gain with no tell loss on potato |
| LI2 Babylon.js WGSL shaders (clustered lights, OpenPBR, ssao2, ssr2, tonemap, volumetric) | Theme: sunlit arena material and light response (G3 G4) | PARK | creative | Backlog, C tier; needs on-vs-off pixel-change spike per pass |
| LI3 PlayCanvas WGSL chunks (aniso GGX, sheen, LTC area, VSM) | Theme: armor reads as metal and cloth under sun (G4) | PARK | creative | Backlog, C,M; needs material-chart spike |
| LI4 Hair strands Kajiya-Kay plus Marschner | Emotion: gladiator hair adds spectacle without hiding a tell (G4) | PARK | creative | Backlog, C,M; needs tell-readability spike |
| LI5 Clear coat and iridescence for lacquered armor | Theme: lacquered armor response (G4) | PARK | creative | Backlog, P,S; needs on-vs-off spike |
| LI6 Pre-integrated skin and eye shader | Experience: fighters read as people, not boxes (G4) | PARK | creative | Backlog, C,M; needs look-score spike |
| LI7 Auto exposure by 64-bin histogram | Experience: keeps tells readable across sun and tunnel (G3) | PARK | process | Backlog, P,S; needs exposure-stability spike |
| LI8 FXAA single pass (SMAA already in `aa_scale.rs`) | Experience: thin edges stay readable at speed (G2) | PARK | process | Backlog, P,S; needs edge-readability vs cost spike |
| LI9 Chromatic aberration, vignette, grain in one pass | Emotion: grade must never hide a tell (G3) | PARK | creative | Backlog, P,S; needs restrained-grade spike proving tells stay readable |
| LI10 Bokeh depth of field by fast filter spreading | Emotion: cinematic focus must never hide a tell (G3) | PARK | creative | Backlog, C,M; needs tell-readability spike, comfortable tier only |
