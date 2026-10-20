# Rules for the powerup wave (2026-10-20)

Shared brief for the 10 research agents of this wave. Read this file first, then your lane prompt.
Every agent writes exactly one file and commits nothing.

## The engine you are hunting for

`C:\engine2040` is a Rust game engine. Own renderer (`crates/render`, wgpu, forward PBR + RT slice),
own editor (`crates/editor`), own AI door (`crates/endpoint`, MCP stdio server `e2040-mcp`), deterministic
fixed tick with one hash per tick (golden `d57b5c8ae3096614`), `crates/physfx` (float Rapier world for ragdoll,
limbs, debris, blast), `crates/anim` (SoA clip sampler, blend, quant, state machine), `crates/assets` (retarget
cook, clip intake, provenance, CC0 gate), `crates/geo` (cook-time mesh tools), `crates/material`, `crates/web`
(browser shell), `crates/net` (rollback), `crates/rules` (Rhai, integers only), `crates/playtest` (verifiers).

Scope and decisions live in `docs/FOCUS.md` and `docs/plan/finish-line.md` ("What we still lack", 23 numbered
gaps A1-A6, B7-B9, C10-C15, D16-D20, E21-E23). The proof game is a multiplayer Roman arena on Windows and in a
browser tab. Decision D4: modelling, UV, LOD, collision, bake, weights and rig tools are door verbs or headless
Blender 5.2 cook tools, not editor panels.

## Hard rules

1. Read only, then write ONE file at the absolute path your lane prompt names. Do not touch any other file.
2. Never commit, never push, never run `git add`, never edit code, never run cargo (heavy cargo goes to CI).
   Shell use: `Get-ChildItem`, `Select-String`, `git grep`, `git log`, node one-liners from a temp file.
3. Every non-obvious claim needs a URL. A claim with no URL is written "unverified".
4. Read the donor file live (raw source, GitHub contents API, arXiv abstract page). Never invent a path.
   If a path 404s, write that it 404s today and move on.
5. License: read the LICENSE live and name it (MIT, Apache-2.0, BSD-3-Clause, Zlib are fine to port from).
   Proprietary (NVIDIA SDKs, Unreal, Unity) is research-only and must be marked PROPRIETARY. We never vendor
   donor bytes: we port the idea and write our own code (D5). Flag copyleft as a risk line.
6. Never invent a number. Unmeasured is "unknown", not a guess.
7. Check the tree before calling anything missing: `git grep` the symbol in `crates/`. The wave-3 lesson:
   "an hour of grepping beats a week of porting." If it already exists in the tree, it is not a gap.
8. Short plain English. One idea per row. No marketing words.

## Already mined, do not repeat these

Bevy (`bevyengine/bevy`: contact shadows, auto exposure, `bevy_remote`, schedule ambiguity detection),
`ggrs` rollback, `meshoptimizer` / `gltfpack`, `rust-av/ssimulacra2`, Kani, `modeling-udacity/OpenShape`,
`jamesmunns/postcard`, Bruneton atmosphere LUT, NVIDIA RTX SDKs (proprietary, excluded),
`moq-dev/web-transport`, `jrouwe/Jolt`, `dimforge` Rapier, `box3d-rust`, `wgpu` STRICT_WEBGPU_COMPLIANCE,
`lazyxeon/AstraWeave`, `uinosoft/zenfg`, `Themaister/Granite`, `tugcantopaloglu/godot-mcp`,
`ahujasid/mcp-for-blender`, `chongdashu/unreal-mcp`, `Coding-Solo/godot-mcp`, `promptfoo`, `ollama`,
`ozz-animation`, `glTF-Transform`, `KhronosGroup/glTF-Blender-IO`, `meshlab`, `NREL`, `Unreal`, `Unity`,
`Godot`, `O3DE`, `Flax`, `Stride`, `Filament`, `DeepMimic`, motion diffusion, `SayPlan`, `ConceptGraphs`,
`SPlisHSPlasH`-adjacent fluid notes if already cited in `docs/research/LADDER.md`.

Before you write a row, check `docs/research/LADDER.md` and `docs/research/steals-2026-10-*/` for the donor.
If it is already there, it is not a new find: either skip it or write it as a CORRECTION with the new fact.

## Row format for your file

Start with a one-paragraph answer to your lane question. Then one table, ranked best first, max 12 rows:

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |

- Verdict is one of: PORT (a builder can do it), RESEARCH-ONLY (read it, port later), REJECT (and why).
- Cost: S = a day, M = a week, L = bigger. Kill line = the exact condition that makes the row die.
- Fills: name a real row id from `docs/plan/finish-line.md` or from the board ids in `docs/SPRINT*.md`
  (LF, EG, GX, RP, VX, RT, CT, HS, AX, MP, ON, GS, AB, ED, OPT, PF, GA, AM, PX, DX, BX, GA). No row id: write
  "no row, propose a new row" and give it a name.
- After the table: "Rejections" (what you checked and dropped, one line each, so nobody re-mines it) and
  "Top 3" (the three strongest finds with their URL and the row they close).

## Evidence grades

Every claim you write is one of: OBSERVED (you read the file or page today), DERIVED (reasoned from what you read),
INFERRED (likely, not proven), UNKNOWN. Write the grade in the cell or in the prose. Do not present a paper's
benchmark as our runtime result.
