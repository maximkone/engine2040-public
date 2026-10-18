# Plan atlas - where everything is planned, and how true each plan is today

Generated 2026-10-17 by `node tools/plan-atlas.mjs --write` from the plans, `docs/SPRINT.md` and the VISION Plans map. Never edit by hand: fix the plan, then regenerate.

Find things fast:

- `node tools/plan-atlas.mjs --row RL25` - which plan owns a row, its status and its plan entry
- `node tools/plan-atlas.mjs --crate render` - every plan that touches a crate
- `node tools/plan-atlas.mjs --priority look` - the reading order for one Core priority (look, bodies, aidoor, speed, proof, other)
- `node tools/plan-atlas.mjs --plan render-look` - one plan health card: rows, crates, section-7 rows off the board
- `node tools/plan-atlas.mjs --find ragdoll` - plans whose text names a word, with the heading it sits under

## 1. Every plan: rows, crates and health

| Plan | Open / ready / done rows | Crates | Serves | Health |
|---|---|---|---|---|
| [`2026-10-17-obsolete-rows`](2026-10-17-obsolete-rows.md) | 0 / 0 / 0 | endpoint, playtest | - | ok |
| [`ATLAS`](ATLAS.md) | 0 / 0 / 0 | - | none: the generated map of every plan, its rows, crates and health | ok |
| [`BOOK-GAPS-2-2026-10-12`](BOOK-GAPS-2-2026-10-12.md) | 0 / 0 / 0 | ai, arena, game, playtest | AI in gameplay; The three games; Look on the RTX 3050; Sim speed; Headless proofs and tests; AI builds the game | ok |
| [`BOOK-GAPS-2026-10-12`](BOOK-GAPS-2026-10-12.md) | 0 / 0 / 0 | anim, arena, combat, ecs, endpoint, game, nav, net, perf, physics, playtest, render | Look on the RTX 3050; Sim speed; Determinism and exact replay; Online, 64 players; Headless proofs and tests; Shipping a clean build | ok |
| [`DP1-deepdive`](DP1-deepdive.md) | 0 / 0 / 0 | - | - | ok |
| [`EG18-deepdive`](EG18-deepdive.md) | 0 / 0 / 0 | anim, game | - | ok |
| [`EG8-deepdive`](EG8-deepdive.md) | 0 / 0 / 0 | proof | - | ok |
| [`GS30-deepdive`](GS30-deepdive.md) | 0 / 0 / 0 | game, spatial | - | ok |
| [`README`](README.md) | 0 / 0 / 0 | render | none: how a plan is written | ok |
| [`SP63-deepdive`](SP63-deepdive.md) | 0 / 0 / 0 | - | - | ok |
| [`ai-door`](ai-door.md) | 0 / 0 / 0 | ai, arena, endpoint, playtest | AI builds the game; AI in gameplay | 1 section-7 row(s) off the board (AID1) |
| [`ai-native`](ai-native.md) | 1 / 1 / 4 | ai, editor, endpoint, game, playtest, proof, provider, spatial | AI builds the game | ok |
| [`animation`](animation.md) | 0 / 0 / 0 | anim, arena, game | Look on the RTX 3050; AI in gameplay | ok |
| [`body-and-break`](body-and-break.md) | 21 / 18 / 12 | arena, combat, destruction, game, physfx, physics, proof | Dismemberment and wounds; Destruction: structure, fracture, fire; AI breaks and balances it; Physics and destruction | ok |
| [`content`](content.md) | 0 / 0 / 0 | assets, audio, endpoint, game, perf, spatial | Look on the RTX 3050; The three games; Shipping a clean build | ok |
| [`design-review-2026-10-17`](design-review-2026-10-17.md) | 0 / 0 / 0 | ai, anim, app, arena, assets, audio, core, editor, endpoint, game, net, nn, physfx, physics, proof, provider, render, spatial, web, world | - | ok |
| [`finish-line`](finish-line.md) | 13 / 5 / 6 | arena, assets, game, perf, physfx, proof, render | Look on the RTX 3050; Sim speed; Determinism and exact replay; AI builds the game; Online, 64 players; Shipping a clean build; Headless proofs and tests | ok |
| [`forge-adoption`](forge-adoption.md) | 30 / 15 / 4 | assets, editor, endpoint, observe, physics, playtest, provider, rules, save, scene, spatial, world | AI builds the game; AI in gameplay; Spatial understanding for AI; Dismemberment and wounds; Physics and destruction; Look on the RTX 3050; Determinism and exact replay; Headless proofs and tests; Editor and tools | ok |
| [`fps-view`](fps-view.md) | 3 / 1 / 3 | game, input, render | Look on the RTX 3050; The three games | ok |
| [`frontier`](frontier.md) | 113 / 44 / 3 | ai, anim, assets, audio, destruction, editor, endpoint, game, physics, playtest, render, spatial, world | Spatial understanding for AI; AI builds the game; Look on the RTX 3050; Sim speed | ok |
| [`gameplay`](gameplay.md) | 26 / 22 / 47 | ai, arena, combat, game, playtest | AI in gameplay; The three games | 4 section-7 row(s) off the board (GP47, GP48, GP49, GP50) |
| [`hard-systems`](hard-systems.md) | 0 / 0 / 0 | playtest, proof | AI builds the game; AI in gameplay; Determinism and exact replay; Headless proofs and tests; Sim speed | 2 section-7 row(s) off the board (HS33, HS34) |
| [`landscape-ideas`](landscape-ideas.md) | 0 / 0 / 0 | - | none: the research idea backlog, promoted to GS rows by the planner | ok |
| [`lf10-pose-hold-rethink`](lf10-pose-hold-rethink.md) | 0 / 0 / 0 | combat, game | - | ok |
| [`living-world`](living-world.md) | 15 / 5 / 0 | ai, endpoint, game | AI in gameplay; The three games; Online, 64 players; AI builds the game | ok |
| [`multiplayer`](multiplayer.md) | 1 / 0 / 1 | anim, audio, net | Online, 64 players; Determinism and exact replay | 1 section-7 row(s) off the board (MP23) |
| [`parked`](parked.md) | 0 / 0 / 0 | assets, audio, editor, game, nav, world | Editor and tools; The three games; Spatial understanding for AI | ok |
| [`potato`](potato.md) | 9 / 9 / 27 | assets, perf, render | Look on the RTX 3050; Sim speed; Shipping a clean build; Headless proofs and tests; Determinism and exact replay | ok |
| [`quality`](quality.md) | 21 / 16 / 7 | ai, anim, arena, bevy_render, ecs, endpoint, game, nav, proof, render | Headless proofs and tests; Sim speed; Shipping a clean build; Online, 64 players; Determinism and exact replay | 6 section-7 row(s) off the board (CQ31, CQ32, CQ33, CQ34) |
| [`render-look`](render-look.md) | 8 / 7 / 10 | anim, assets, game, input, material, render | Look on the RTX 3050; Shipping a clean build | ok |
| [`rome-spec`](rome-spec.md) | 0 / 0 / 0 | combat, core, game | The three games; Determinism and exact replay; AI in gameplay | ok |
| [`runtime`](runtime.md) | 0 / 0 / 0 | app | Sim speed; Determinism and exact replay; Headless proofs and tests | ok |
| [`seats`](seats.md) | 22 / 1 / 0 | - | none: the 40-seat sprint's seats, inputs, outputs and first seeds | 6 section-7 row(s) off the board (AW24, AW25, AW26, AW27) |
| [`silicon-city`](silicon-city.md) | 0 / 0 / 0 | chipsim, core, endpoint, nn, observe, playtest, render, rules, ui, web | none: a future game (LATER), no Core proof | 45 section-7 row(s) off the board (SIL1, SIL2, SIL3, SIL4) |
| [`silicon-world`](silicon-world.md) | 0 / 0 / 0 | audio, chipsim, physfx, ui, web | none: a future game (LATER), no Core proof | 14 section-7 row(s) off the board (SIL46, SIL47, SIL48, SIL49) |
| [`spatial`](spatial.md) | 0 / 0 / 0 | game, spatial | Spatial understanding for AI | ok |
| [`spear-grapple`](spear-grapple.md) | 0 / 0 / 0 | arena, input, net | - | ok |
| [`status-2026-10-17`](status-2026-10-17.md) | 0 / 0 / 0 | ai, playtest | - | ok |
| [`steals`](steals.md) | 0 / 0 / 0 | arena, bevy_anti_alias, destruction, endpoint, net, perf, physics, render | none: the scout seat's list of steals that land on a finish bar, open until landed | ok |
| [`urban-war`](urban-war.md) | 62 / 12 / 0 | ai, combat, physics, proof | The three games | ok |
| [`web-and-realism`](web-and-realism.md) | 0 / 0 / 0 | arena, combat, destruction, game, physfx, physics, render | - | ok |
| [`wiring-triage-2026-10-17`](wiring-triage-2026-10-17.md) | 0 / 0 / 0 | arena, game, nav, physics, web | - | ok |

## 2. Row prefix -> plan

| Prefix | Plan(s) |
|---|---|
| AB | `living-world` |
| AH | `quality` |
| AM | `animation` |
| ARC | `runtime` |
| AS | `content` |
| AW | `seats` |
| AX | `ai-native` |
| BX | `body-and-break` |
| CB | `gameplay` |
| CQ | `quality` |
| CT | `content` |
| DX | `body-and-break` |
| EA | `ai-native` |
| ED | `parked` |
| EV | `ai-native` |
| FB | `quality` |
| FU | `frontier` |
| GA | `gameplay` |
| GE | `runtime` |
| GM | `render-look` |
| GP | `gameplay` |
| GW | `ai-native` |
| GX | `render-look` |
| HS | `hard-systems` |
| LAI | `living-world` |
| LM | `animation` |
| LW | `living-world` |
| MCP | `ai-native` |
| MP | `multiplayer` |
| NS | `frontier` |
| NV | `gameplay` |
| ON | `multiplayer` |
| OPT | `quality` |
| PF | `quality` |
| PLF | `parked` |
| PX | `body-and-break` |
| RA | `render-look` |
| RC | `runtime` |
| RL | `render-look` |
| RP | `render-look` |
| SL | `quality` |
| SN | `parked` |
| SP | `spatial` |
| TV | `animation` |
| UC | `urban-war` |
| UIX | `parked` |
| UW | `urban-war` |
| VS | `render-look` |
| WD | `parked` |
| WIN | `render-look` |

## 3. Crate -> plans

| Crate | Plans (open rows) |
|---|---|
| `ai` | `frontier` (113), `urban-war` (62), `gameplay` (26), `quality` (21), `living-world` (15), `ai-native` (1), `BOOK-GAPS-2-2026-10-12` (0), `ai-door` (0), `design-review-2026-10-17` (0), `status-2026-10-17` (0) |
| `anim` | `frontier` (113), `quality` (21), `render-look` (8), `multiplayer` (1), `BOOK-GAPS-2026-10-12` (0), `EG18-deepdive` (0), `animation` (0), `design-review-2026-10-17` (0) |
| `app` | `design-review-2026-10-17` (0), `runtime` (0) |
| `arena` | `gameplay` (26), `body-and-break` (21), `quality` (21), `finish-line` (13), `BOOK-GAPS-2-2026-10-12` (0), `BOOK-GAPS-2026-10-12` (0), `ai-door` (0), `animation` (0), `design-review-2026-10-17` (0), `spear-grapple` (0), `steals` (0), `web-and-realism` (0), `wiring-triage-2026-10-17` (0) |
| `assets` | `frontier` (113), `forge-adoption` (30), `finish-line` (13), `potato` (9), `render-look` (8), `content` (0), `design-review-2026-10-17` (0), `parked` (0) |
| `audio` | `frontier` (113), `multiplayer` (1), `content` (0), `design-review-2026-10-17` (0), `parked` (0), `silicon-world` (0) |
| `bevy_anti_alias` | `steals` (0) |
| `bevy_render` | `quality` (21) |
| `chipsim` | `silicon-city` (0), `silicon-world` (0) |
| `combat` | `urban-war` (62), `gameplay` (26), `body-and-break` (21), `BOOK-GAPS-2026-10-12` (0), `lf10-pose-hold-rethink` (0), `rome-spec` (0), `web-and-realism` (0) |
| `core` | `design-review-2026-10-17` (0), `rome-spec` (0), `silicon-city` (0) |
| `destruction` | `frontier` (113), `body-and-break` (21), `steals` (0), `web-and-realism` (0) |
| `ecs` | `quality` (21), `BOOK-GAPS-2026-10-12` (0) |
| `editor` | `frontier` (113), `forge-adoption` (30), `ai-native` (1), `design-review-2026-10-17` (0), `parked` (0) |
| `endpoint` | `frontier` (113), `forge-adoption` (30), `quality` (21), `living-world` (15), `ai-native` (1), `2026-10-17-obsolete-rows` (0), `BOOK-GAPS-2026-10-12` (0), `ai-door` (0), `content` (0), `design-review-2026-10-17` (0), `silicon-city` (0), `steals` (0) |
| `game` | `frontier` (113), `gameplay` (26), `body-and-break` (21), `quality` (21), `living-world` (15), `finish-line` (13), `render-look` (8), `fps-view` (3), `ai-native` (1), `BOOK-GAPS-2-2026-10-12` (0), `BOOK-GAPS-2026-10-12` (0), `EG18-deepdive` (0), `GS30-deepdive` (0), `animation` (0), `content` (0), `design-review-2026-10-17` (0), `lf10-pose-hold-rethink` (0), `parked` (0), `rome-spec` (0), `spatial` (0), `web-and-realism` (0), `wiring-triage-2026-10-17` (0) |
| `input` | `render-look` (8), `fps-view` (3), `spear-grapple` (0) |
| `material` | `render-look` (8) |
| `nav` | `quality` (21), `BOOK-GAPS-2026-10-12` (0), `parked` (0), `wiring-triage-2026-10-17` (0) |
| `net` | `multiplayer` (1), `BOOK-GAPS-2026-10-12` (0), `design-review-2026-10-17` (0), `spear-grapple` (0), `steals` (0) |
| `nn` | `design-review-2026-10-17` (0), `silicon-city` (0) |
| `observe` | `forge-adoption` (30), `silicon-city` (0) |
| `perf` | `finish-line` (13), `potato` (9), `BOOK-GAPS-2026-10-12` (0), `content` (0), `steals` (0) |
| `physfx` | `body-and-break` (21), `finish-line` (13), `design-review-2026-10-17` (0), `silicon-world` (0), `web-and-realism` (0) |
| `physics` | `frontier` (113), `urban-war` (62), `forge-adoption` (30), `body-and-break` (21), `BOOK-GAPS-2026-10-12` (0), `design-review-2026-10-17` (0), `steals` (0), `web-and-realism` (0), `wiring-triage-2026-10-17` (0) |
| `playtest` | `frontier` (113), `forge-adoption` (30), `gameplay` (26), `ai-native` (1), `2026-10-17-obsolete-rows` (0), `BOOK-GAPS-2-2026-10-12` (0), `BOOK-GAPS-2026-10-12` (0), `ai-door` (0), `hard-systems` (0), `silicon-city` (0), `status-2026-10-17` (0) |
| `proof` | `urban-war` (62), `body-and-break` (21), `quality` (21), `finish-line` (13), `ai-native` (1), `EG8-deepdive` (0), `design-review-2026-10-17` (0), `hard-systems` (0) |
| `provider` | `forge-adoption` (30), `ai-native` (1), `design-review-2026-10-17` (0) |
| `render` | `frontier` (113), `quality` (21), `finish-line` (13), `potato` (9), `render-look` (8), `fps-view` (3), `BOOK-GAPS-2026-10-12` (0), `README` (0), `design-review-2026-10-17` (0), `silicon-city` (0), `steals` (0), `web-and-realism` (0) |
| `rules` | `forge-adoption` (30), `silicon-city` (0) |
| `save` | `forge-adoption` (30) |
| `scene` | `forge-adoption` (30) |
| `spatial` | `frontier` (113), `forge-adoption` (30), `ai-native` (1), `GS30-deepdive` (0), `content` (0), `design-review-2026-10-17` (0), `spatial` (0) |
| `ui` | `silicon-city` (0), `silicon-world` (0) |
| `web` | `design-review-2026-10-17` (0), `silicon-city` (0), `silicon-world` (0), `wiring-triage-2026-10-17` (0) |
| `world` | `frontier` (113), `forge-adoption` (30), `design-review-2026-10-17` (0), `parked` (0) |

