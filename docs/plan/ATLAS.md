# Plan atlas - where everything is planned, and how true each plan is today

Generated 2026-10-13 by `node tools/plan-atlas.mjs --write` from the plans, `docs/SPRINT.md` and the VISION Plans map. Never edit by hand: fix the plan, then regenerate.

Find things fast:

- `node tools/plan-atlas.mjs --row RL25` - which plan owns a row, its status and its plan entry
- `node tools/plan-atlas.mjs --crate render` - every plan that touches a crate
- `node tools/plan-atlas.mjs --priority look` - the reading order for one Core priority (look, bodies, aidoor, speed, proof, other)
- `node tools/plan-atlas.mjs --plan render-look` - one plan health card: rows, crates, section-7 rows off the board
- `node tools/plan-atlas.mjs --find ragdoll` - plans whose text names a word, with the heading it sits under

## 1. Every plan: rows, crates and health

| Plan | Open / ready / done rows | Crates | Serves | Health |
|---|---|---|---|---|
| [`ATLAS`](ATLAS.md) | 0 / 0 / 0 | - | none: the generated map of every plan, its rows, crates and health | ok |
| [`BOOK-GAPS-2-2026-10-12`](BOOK-GAPS-2-2026-10-12.md) | 0 / 0 / 0 | ai, arena, game, playtest | AI in gameplay; The three games; Look on the RTX 3050; Sim speed; Headless proofs and tests; AI builds the game | ok |
| [`BOOK-GAPS-2026-10-12`](BOOK-GAPS-2026-10-12.md) | 0 / 0 / 0 | anim, arena, ecs, physics, playtest, render | Look on the RTX 3050; Sim speed; Determinism and exact replay; Online, 64 players; Headless proofs and tests; Shipping a clean build | ok |
| [`DP1-deepdive`](DP1-deepdive.md) | 0 / 0 / 0 | - | - | ok |
| [`EG8-deepdive`](EG8-deepdive.md) | 0 / 0 / 0 | proof | - | ok |
| [`README`](README.md) | 0 / 0 / 0 | render | none: how a plan is written | ok |
| [`ai-door`](ai-door.md) | 0 / 0 / 0 | ai, arena, endpoint, playtest | - | 1 section-7 row(s) off the board (AID1) |
| [`ai-native`](ai-native.md) | 5 / 3 / 0 | ai, editor, endpoint, game, playtest, proof, provider, spatial | AI builds the game | ok |
| [`animation`](animation.md) | 0 / 0 / 0 | anim, arena, game | Look on the RTX 3050; AI in gameplay | ok |
| [`body-and-break`](body-and-break.md) | 33 / 26 / 0 | arena, combat, destruction, physics, proof | Dismemberment and wounds; Destruction: structure, fracture, fire; AI breaks and balances it; Physics and destruction | ok |
| [`content`](content.md) | 0 / 0 / 0 | assets, audio, endpoint, game, perf, spatial | Look on the RTX 3050; The three games; Shipping a clean build | ok |
| [`finish-line`](finish-line.md) | 19 / 8 / 0 | perf | Look on the RTX 3050; Sim speed; Determinism and exact replay; AI builds the game; Online, 64 players; Shipping a clean build; Headless proofs and tests | ok |
| [`forge-adoption`](forge-adoption.md) | 34 / 17 / 0 | assets, editor, endpoint, observe, physics, playtest, provider, rules, save, scene, spatial, world | AI builds the game; AI in gameplay; Spatial understanding for AI; Dismemberment and wounds; Physics and destruction; Look on the RTX 3050; Determinism and exact replay; Headless proofs and tests; Editor and tools | ok |
| [`frontier`](frontier.md) | 116 / 45 / 0 | ai, anim, assets, audio, destruction, editor, endpoint, game, physics, playtest, render, spatial, world | Spatial understanding for AI; AI builds the game; Look on the RTX 3050; Sim speed | ok |
| [`gameplay`](gameplay.md) | 50 / 36 / 21 | ai, combat, game, playtest | AI in gameplay; The three games | ok |
| [`hard-systems`](hard-systems.md) | 0 / 0 / 0 | playtest, proof | AI builds the game; AI in gameplay; Determinism and exact replay; Headless proofs and tests; Sim speed | 2 section-7 row(s) off the board (HS33, HS34) |
| [`landscape-ideas`](landscape-ideas.md) | 0 / 0 / 0 | - | none: the research idea backlog, promoted to GS rows by the planner | ok |
| [`living-world`](living-world.md) | 15 / 2 / 0 | ai, endpoint, game | AI in gameplay; The three games; Online, 64 players; AI builds the game | ok |
| [`multiplayer`](multiplayer.md) | 2 / 1 / 0 | audio, net | Online, 64 players; Determinism and exact replay | 1 section-7 row(s) off the board (MP23) |
| [`parked`](parked.md) | 0 / 0 / 0 | assets, audio, editor, game, nav, world | Editor and tools; The three games; Spatial understanding for AI | ok |
| [`potato`](potato.md) | 29 / 21 / 6 | assets, perf, render | Look on the RTX 3050; Sim speed; Shipping a clean build; Headless proofs and tests; Determinism and exact replay | ok |
| [`quality`](quality.md) | 27 / 19 / 1 | arena, ecs, nav, proof, render | Headless proofs and tests; Sim speed; Shipping a clean build; Online, 64 players; Determinism and exact replay | ok |
| [`render-look`](render-look.md) | 12 / 10 / 6 | anim, assets, game, input, material, render | Look on the RTX 3050; Shipping a clean build | ok |
| [`rome-spec`](rome-spec.md) | 0 / 0 / 0 | combat, core, game | The three games; Determinism and exact replay; AI in gameplay | ok |
| [`runtime`](runtime.md) | 0 / 0 / 0 | app | Sim speed; Determinism and exact replay; Headless proofs and tests | ok |
| [`seats`](seats.md) | 22 / 1 / 0 | - | none: the 40-seat sprint's seats, inputs, outputs and first seeds | ok |
| [`spatial`](spatial.md) | 0 / 0 / 0 | game, spatial | Spatial understanding for AI | ok |
| [`steals`](steals.md) | 0 / 0 / 0 | arena, bevy_anti_alias, destruction, endpoint, net, perf, physics, render | none: the scout seat's list of steals that land on a finish bar, open until landed | ok |
| [`urban-war`](urban-war.md) | 62 / 12 / 0 | proof | The three games | ok |

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
| `ai` | `frontier` (116), `gameplay` (50), `living-world` (15), `ai-native` (5), `BOOK-GAPS-2-2026-10-12` (0), `ai-door` (0) |
| `anim` | `frontier` (116), `render-look` (12), `BOOK-GAPS-2026-10-12` (0), `animation` (0) |
| `app` | `runtime` (0) |
| `arena` | `body-and-break` (33), `quality` (27), `BOOK-GAPS-2-2026-10-12` (0), `BOOK-GAPS-2026-10-12` (0), `ai-door` (0), `animation` (0), `steals` (0) |
| `assets` | `frontier` (116), `forge-adoption` (34), `potato` (29), `render-look` (12), `content` (0), `parked` (0) |
| `audio` | `frontier` (116), `multiplayer` (2), `content` (0), `parked` (0) |
| `bevy_anti_alias` | `steals` (0) |
| `combat` | `gameplay` (50), `body-and-break` (33), `rome-spec` (0) |
| `core` | `rome-spec` (0) |
| `destruction` | `frontier` (116), `body-and-break` (33), `steals` (0) |
| `ecs` | `quality` (27), `BOOK-GAPS-2026-10-12` (0) |
| `editor` | `frontier` (116), `forge-adoption` (34), `ai-native` (5), `parked` (0) |
| `endpoint` | `frontier` (116), `forge-adoption` (34), `living-world` (15), `ai-native` (5), `ai-door` (0), `content` (0), `steals` (0) |
| `game` | `frontier` (116), `gameplay` (50), `living-world` (15), `render-look` (12), `ai-native` (5), `BOOK-GAPS-2-2026-10-12` (0), `animation` (0), `content` (0), `parked` (0), `rome-spec` (0), `spatial` (0) |
| `input` | `render-look` (12) |
| `material` | `render-look` (12) |
| `nav` | `quality` (27), `parked` (0) |
| `net` | `multiplayer` (2), `steals` (0) |
| `observe` | `forge-adoption` (34) |
| `perf` | `potato` (29), `finish-line` (19), `content` (0), `steals` (0) |
| `physics` | `frontier` (116), `forge-adoption` (34), `body-and-break` (33), `BOOK-GAPS-2026-10-12` (0), `steals` (0) |
| `playtest` | `frontier` (116), `gameplay` (50), `forge-adoption` (34), `ai-native` (5), `BOOK-GAPS-2-2026-10-12` (0), `BOOK-GAPS-2026-10-12` (0), `ai-door` (0), `hard-systems` (0) |
| `proof` | `urban-war` (62), `body-and-break` (33), `quality` (27), `ai-native` (5), `EG8-deepdive` (0), `hard-systems` (0) |
| `provider` | `forge-adoption` (34), `ai-native` (5) |
| `render` | `frontier` (116), `potato` (29), `quality` (27), `render-look` (12), `BOOK-GAPS-2026-10-12` (0), `README` (0), `steals` (0) |
| `rules` | `forge-adoption` (34) |
| `save` | `forge-adoption` (34) |
| `scene` | `forge-adoption` (34) |
| `spatial` | `frontier` (116), `forge-adoption` (34), `ai-native` (5), `content` (0), `spatial` (0) |
| `world` | `frontier` (116), `forge-adoption` (34), `parked` (0) |

