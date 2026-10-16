<!-- checked 6ec61796 2026-10-16T16:05Z -->
# SLICE — the arena duel scorecard (director seat)

One number per pillar. Commands copied from `docs/plan/finish-line.md` G1-G9 and
`docs/FOCUS.md` Now "Proof:". Numbers read from the newest `target/reports/` files;
no builds were run, so cargo rows stay NOT RUN unless a report prints them.

| Pillar (FOCUS Now 1-7) | Command that prints its number | Today's number (2026-10-16T14:37Z, HEAD f66ee186) | Bar | Stage | Exit test |
|---|---|---|---|---|---|
| Look | `node tools/finish.mjs A4 A5 A8` (exit 0, ran 2026-10-16T14:37Z) | A4 partial (10/12 open: Q1 GX1-GX4 GX6-GX10, GX5 dropped), A5 partial (8/10: GM3 WIN1 MP1 MP2 MP3 GP1 PX1 DX1), A8 open (1/1 GM3); last golden x2 MATCH `d57b5c8ae3096614` at c3431d30 per `run-c3431d30-20261016T1415Z.md` (exit 0 x2, no golden report at f66ee186); FC1 quiet triple exit 0 x3 at c3431d30 (300 frames empty 0 fewest 2 steepest 17.0); window at f66ee186 per `playtest-f66ee186-20261016T1424Z.md` 60 frames framing 60 empty 0 pops 76 validation_errors 0, 3 PNGs fighters; newest art `art-f1fb6311.md` sky 0/5 shadow 0.000ms pops 379 sunk 0 | product-bar targets: live passes each change pixels (EG8), look score at/above Q1 bar | Preproduction | framing empty 0/300, validation_errors 0, live pass ms above 0 |
| Bodies | `cargo test -p engine-proof break_corpus` x2 + HS2 matrix (NOT RUN, no builds) | UNKNOWN (no fresh corpus or matrix report; no report at f66ee186; staged sim stable: `playtest-f66ee186-20261016T1424Z.md` 600 ticks cues 30 hits 27 knockdowns 3 hash 2935468d byte-identical to 11:49Z e3093e5 and 14:02Z c3431d3 runs, no drift) | G6 same hash: break corpus x2 + HS2 matrix; pops 0, sunk under 2 cm | Preproduction | corpus x2 green, HS2 matrix green |
| AI door | `cargo test -p engine-playtest --lib mirror` x2 (NOT RUN this run, no builds) | Last green stands, no newer report today: HS29 green at 1d2c63ba per `qa-hs29-1d2c63ba-20261014T2152Z.md` (newest qa report on disk); HS20 green at 37a7894d (30/30 parity, mirror 44 passed x2); HS30 green at 1bcd59c (mirror_craft 7 passed x2); finish A6 open 8/8 (ran 14:37Z exit 0); no report at f66ee186 | G7 same task, same result on mock, local, remote (LF16) | First Playable | mirror x2 green, parity holds |
| Feel | each row's Accept (FOCUS Now 5; NOT RUN, no builds) | Newest C4 at f66ee186 per `playtest-f66ee186-20261016T1424Z.md`: staged 600 ticks cues 30 hash 2935468d identical cross-head (11:49Z e3093e5 / 14:02Z c3431d3 / 14:24Z f66ee186, no drift); window 60 frames framing 60 empty 0 pops 76 validation_errors 0, 3 PNGs opened with fighters; C4 idle question FAIL (600-tick cap, 0 idle inputs, unrunnable 30 s tape) | G5: 0 pops per 300 frames, feet planted, hits land (hold, shake, trail) | Preproduction | `arena view:` pops 0, sunk_cm_max under 2 |
| Multiplayer | `cargo test -p engine-net --lib snapshot` (NOT RUN, no builds) | UNKNOWN (no snapshot report on disk today, re-verified 2026-10-16T14:37Z; newest net file `res-ioq3-net-20261015.md` is research notes only, no test evidence; no report at f66ee186) | G8: the duel on LAN and internet, one hash (MP6, ON30 played bouts) | Preproduction | snapshot green, played bouts MP6 ON30 |
| Potato | `node tools/potato-check.mjs` (NOT re-run this run: newest on-disk report 14:13Z stands) | adapter UNKNOWN (NO_IGPU), p50 UNKNOWN, p99 UNKNOWN, peak_ws 1500 MB PASS (<= 3072) per `target/reports/potato-2026-10-16.md` (14:13Z, arena ran and exited); no report at f66ee186 | G9: Intel UHD p50 50 ms, p99 80 ms, working set 3 GB; heap peak flat; assets inside potato caps | Preproduction | potato-check green on the Intel UHD |

Row verdicts, cut log and landscape dry-run are below. Judge verify for this run: `node tools/board-slice.mjs --focus --open` exit 0 (ran 2026-10-16T16:05Z, first 20 IDs FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64 unchanged in the same order) plus `node tools/workspace-check.mjs` exit 1 (ran 2026-10-16T16:05Z: FAIL lines only outside this run's paths, engine-nav bake.rs undeclared plus stale modmap perf/tests/it/core_bench.rs, paths this run never touched; no new FAIL; cargo rows stay NOT RUN by seat rule). Scorecard table left untouched this run (slice last claimed 14:37Z 10-16, inside 2 h); row-verdicts left untouched (rowverdicts last claimed 15:54Z 10-16, inside 2 h; one-item rule, so only the dry section refreshed). Verdict source: `node tools/board-slice.mjs --focus --open` exit 0 (open rows listed, HEAD now 6ec61796).

## Row verdicts (first 20 open FOCUS rows, 2026-10-16, re-verified at 918e5370)

Every row must answer CONCEPT line 1 (readable decisive arena duel) through one of its eight values. A row serving no value is PARK or CUT. All 20 below serve a value, so all KEEP (same 20 IDs in the same order as the 13:13Z check: FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64; re-verified `node tools/board-slice.mjs --focus --open` exit 0 at 918e5370, ran 2026-10-16T15:54Z). Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Scorecard table and dry sections above and below left untouched this run (slice claimed 14:37Z 10-16, inside 2 h; one-item rule, so only this section refreshed).

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

No cuts this run (director, 2026-10-16T15:54Z: all 20 serve a value, so no PARK or CUT; re-confirmed at 918e5370, first 20 IDs unchanged in the same order). Board untouched by seat rule.

## Landscape dry-run (first 10 LI rows, 2026-10-16)

Dry run of `docs/plan/landscape-ideas.md` section A (LI1-LI10) against the eight CONCEPT values. Default PARK: nothing here is a row yet (Tag LATER in `docs/plan/landscape-ideas.md`); each idea needs one spike (one test, one baseline, a kill rule) before it becomes a GS row. Kill rule for every cluster: no change in the number it targets, or more than 1 ms / 50 MB on the potato tier, deletes the idea. A row serving no value is PARK or CUT; all 10 below serve at most one value and stay PARK. Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Source: `docs/plan/landscape-ideas.md:13-22`, CONCEPT eight values in `docs/design/CONCEPT.md:3-10`. Re-verified 2026-10-16T16:05Z at 6ec61796: same 10 IDs LI1-LI10 in the same order, lines still on disk (`docs/plan/landscape-ideas.md:13-22`) so none promoted to a GS row (Grep `\| LI` in `docs/SPRINT.md` prints 0), all stay PARK (`node tools/board-slice.mjs --focus --open` exit 0, ran 2026-10-16T16:05Z, first 20 FOCUS IDs unchanged in the same order as the 15:54Z check; `node tools/workspace-check.mjs` exit 1, ran 2026-10-16T16:05Z, FAIL lines only outside this run's paths, no new FAIL, no builds). Scorecard table and row-verdict sections above left untouched this run (slice claimed 14:37Z and rowverdicts claimed 15:54Z, inside 2 h; dry last claimed 13:24Z 10-16, outside 2 h, so this section refreshed).

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
