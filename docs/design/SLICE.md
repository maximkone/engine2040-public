<!-- checked 77e6ff76 2026-10-15T21:02Z -->
# SLICE — the arena duel scorecard (director seat)

One number per pillar. Commands copied from `docs/plan/finish-line.md` G1-G9 and
`docs/FOCUS.md` Now "Proof:". Numbers read from the newest `target/reports/` files;
no builds were run, so cargo rows stay NOT RUN unless a report prints them.

| Pillar (FOCUS Now 1-7) | Command that prints its number | Today's number (2026-10-15T21:02Z, HEAD 77e6ff76) | Bar | Stage | Exit test |
|---|---|---|---|---|---|
| Look | `node tools/finish.mjs A4 A5 A8` (exit 0, ran 2026-10-15T21:02Z) | A4 partial (10/12 open: Q1 GX1-GX4 GX6-GX10, GX5 dropped), A5 partial (8/10: GM3 WIN1 MP1 MP2 MP3 GP1 PX1 DX1), A8 open (1/1, GM3); no look frames at HEAD 77e6ff76: newest `run-77688d7f-20261015T2003Z.md`: golden x2 MATCH `d57b5c8ae3096614` on the stale engine2040 binary (rebuilds red: tier.rs E0658 plus presenter E0432/E0425), arena.exe MISSING so 0 PNGs, `arena framing:` NONE | product-bar targets: live passes each change pixels (EG8), look score at/above Q1 bar | Preproduction | framing empty 0/300, validation_errors 0, live pass ms above 0 |
| Bodies | `cargo test -p engine-proof break_corpus` x2 + HS2 matrix (NOT RUN, no builds) | UNKNOWN (no fresh corpus or matrix report; spam-path sim stable across 10 HEADs per `playtest-def08f9-20261015T2040Z.md` (140 spam inputs, t60/t600 digests identical 920df98 to def08f9); golden x2 MATCH on the stale binary per `run-77688d7f-20261015T2003Z.md`) | G6 same hash: break corpus x2 + HS2 matrix; pops 0, sunk under 2 cm | Preproduction | corpus x2 green, HS2 matrix green |
| AI door | `cargo test -p engine-playtest --lib mirror` x2 (NOT RUN this run, no builds) | Last green stands, no newer report today: HS29 green at 1d2c63ba per `qa-hs29-1d2c63ba-20261014T2152Z.md` (newest qa report on disk); HS20 green at 37a7894d (30/30 parity, mirror 44 passed x2); HS30 green at 1bcd59c (mirror_craft 7 passed x2); finish A6 open 8/8 (ran 21:02Z exit 0) | G7 same task, same result on mock, local, remote (LF16) | First Playable | mirror x2 green, parity holds |
| Feel | each row's Accept (FOCUS Now 5; NOT RUN, no builds) | UNKNOWN (no feel Accept run today; newest C5 at def08f9 per `playtest-def08f9-20261015T2040Z.md`: stamina-out feedback FAIL in pixels (no bar, flash, or number on 3 opened PNGs), 140 spam inputs consumed and stable; newest C1 at 298c05bf per `playtest-298c05bf-20261015T2050Z.md`: first-hit visible FAIL, timing NOT proven on the MCP path, sim stable to 600 ticks) | G5: 0 pops per 300 frames, feet planted, hits land (hold, shake, trail) | Preproduction | `arena view:` pops 0, sunk_cm_max under 2 |
| Multiplayer | `cargo test -p engine-net --lib snapshot` (NOT RUN, no builds) | UNKNOWN (no snapshot report on disk today; newest net file `res-ioq3-net-20261015.md` is research notes only, no test evidence) | G8: the duel on LAN and internet, one hash (MP6, ON30 played bouts) | Preproduction | snapshot green, played bouts MP6 ON30 |
| Potato | `node tools/potato-check.mjs` (NOT re-run this run: arena.exe MISSING per `run-77688d7f-20261015T2003Z.md` and the 10-min arena run exceeds the tool stop; newest on-disk report 18:02Z stands) | adapter UNKNOWN (NO_IGPU), p50 UNKNOWN, p99 UNKNOWN, peak_ws UNKNOWN, arena MISSING per `target/reports/potato-2026-10-15.md` (18:02Z) | G9: Intel UHD p50 50 ms, p99 80 ms, working set 3 GB; heap peak flat; assets inside potato caps | Preproduction | potato-check green on the Intel UHD |

Row verdicts, cut log and landscape dry-run are below. Judge verify for this run: `node tools/board-slice.mjs --focus --open` exit 0 (ran 2026-10-15T21:02Z, first 20 IDs FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64 in the same order as the 17:32Z check) plus `node tools/workspace-check.mjs` exit 0 (ran 2026-10-15T21:02Z, PASS all lines, no FAIL) plus `node tools/finish.mjs A4 A5 A8` exit 0 (ran 2026-10-15T21:02Z, scorecard refreshed this run), all with no builds; cargo rows stay NOT RUN by seat rule. Scorecard table refreshed this run (slice claimed 21:00Z 10-15, eligible: last slice claim 17:11Z outside 2 h; rowverdicts claimed 17:32Z and dry claimed 17:47Z, both outside 2 h but deferred by the one-item rule, so the row-verdict and dry-run sections below are left untouched). Verdict source: `node tools/board-slice.mjs --focus --open` exit 0 (open rows listed, HEAD now 77e6ff76).

## Row verdicts (first 20 open FOCUS rows, 2026-10-15, re-verified at 43c1084)

Every row must answer CONCEPT line 1 (readable decisive arena duel) through one of its eight values. A row serving no value is PARK or CUT. All 20 below serve a value, so all KEEP (same 20 IDs in the same order as the 17:32Z check: FC1 EG8 SP63 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 EG18 RL21 VS1 FC3 EG45 EG19b HS2 SP61 SP64; re-verified `node tools/board-slice.mjs --focus --open` exit 0 at 43c1084, ran 2026-10-15T21:09Z). Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Scorecard table above left untouched this run (slice claimed 21:00Z, inside 2 h; dry claimed 17:47Z, outside 2 h but deferred by the one-item rule, so this section refreshed).

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

No cuts this run (director, 2026-10-15T21:09Z: all 20 serve a value, so no PARK or CUT; re-confirmed at 43c1084, first 20 IDs unchanged in the same order). Board untouched by seat rule.

## Landscape dry-run (first 10 LI rows, 2026-10-15)

Dry run of `docs/plan/landscape-ideas.md` section A (LI1-LI10) against the eight CONCEPT values. Default PARK: nothing here is a row yet (Tag LATER in `docs/plan/landscape-ideas.md`); each idea needs one spike (one test, one baseline, a kill rule) before it becomes a GS row. Kill rule for every cluster: no change in the number it targets, or more than 1 ms / 50 MB on the potato tier, deletes the idea. A row serving no value is PARK or CUT; all 10 below serve at most one value and stay PARK. Type: creative = player-facing value, process = gate or measurement, business = owner decision (none this run). Source: `docs/plan/landscape-ideas.md:13-22`, CONCEPT eight values in `docs/design/CONCEPT.md:3-10`. Re-verified 2026-10-15T21:19Z at 43c10846: same 10 IDs in the same order, lines still on disk so none promoted to a GS row, all stay PARK (`node tools/board-slice.mjs --focus --open` exit 0, ran 2026-10-15T21:18Z, first 20 FOCUS IDs unchanged in the same order as the 21:09Z check; `node tools/workspace-check.mjs` exit 0, ran 2026-10-15T21:18Z, PASS all lines, no FAIL, no builds). Scorecard table and row-verdict sections above left untouched this run (slice claimed 21:00Z and rowverdicts claimed 21:09Z, inside 2 h; dry claimed 17:47Z, outside 2 h, so this section refreshed).

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
