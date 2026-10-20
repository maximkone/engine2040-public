# Plans — the one index, and how a plan is written

## Plan index (owner 2026-10-10: one index for every file in `docs/plan/`)

The order of work is `../FOCUS.md` "## Now", not this file. Tag: NOW = feeds a
"## Now" row; NEXT = feeds a Core proof, not this round; LATER = everything
else. Loop work reads NOW files only. Core proofs: Look, Bodies, AI door,
Multiplayer. Format: file — tag — proof it feeds — scope. A new plan file gets
one line here in the same commit (`tools/plan-index.test.mjs` checks it).

- `finish-line.md` — NOW — all four — the finish line as numbers the loop prints itself (gates G1-G8, product check), the order of the LF steal rows, research in `docs/research/steals-2026-10-11/`
- `kam2-rust.md` — NOW — Look — the owner's 2026-10-20 order: a KaM-economy RTS in Rust with a RA2-look 2.5D renderer, its crates KAM2-SIM/DATA/RENDER/UI/AI/GAME, milestones KAM0-KAM5 and the free-art legal line
- `kam2-style-lock.md` — NOW — Look — the measured art lock for KAM2: 2:1 diamond at 64x32, 8 facings, turret layer, palette, light rig, team mask, cameo and manifest rules
- `ai-native-steals.md` — NOW — AI door — the owner's 2026-10-20 seat/warn report: what landed (the live `--agent-seat` socket and self-warn floors) and the five ranked steals with sources and rows AIN1-AIN6
- `status-2026-10-17.md` — NOW — all four — one-screen truth check: what is verified green this week, what code was cut, which rows are unbuilt, and the owner's open decisions; read it before trusting a status cell in any plan
- `web-and-realism.md` — NOW — Look, Bodies, Multiplayer — heavy realistic games on Windows and in the browser: the wasm build, the physical layer (ragdoll, severed limbs, debris), the lit and textured arena, browser multiplayer, and an AI-driven project setup (rows WEB, RZ, VR, AIS)
- `web-plays-the-match.md` — NOW — Look, Multiplayer — the browser page runs the same match the native window runs (`App` + fixed tick + the fight draw helpers) and draws it with the procedural ring until the cooked kit assets land (row WEB15)
- `parry-and-limb.md` — NOW — Look, Bodies — the v2 exchange (parry, riposte, clash) made readable in the page, and a lost guard costing a limb that falls into the debris layer (row WEB16 with QR12)
- `web-real-bodies.md` — NOW — Look — the page draws the real CT7 fighters and the arena kit ring through the window's own look path (`FighterLook` + `FightRing`) from a cooked bundle instead of stand-in boxes (row WEB17)
- `wiring-triage-2026-10-17.md` — NEXT — all four — what exists but nothing calls: the cut verdicts, 29 unwired features with their missing call sites, ten wirings with paste-ready rows
- `2026-10-18-wiring-sweep.md` — NEXT — all four — the 20 most valuable unwired pub items in today's crates (endpoint, playtest, render, game, net, assets, web) with path:line, what each serves and its smallest real caller, the 5852 island-count context, and the scan's `#[cfg(test)]` over-reporting caveat
- `2026-10-18-handoff.md` — NEXT — all four — what is landed, what is unproven, the hard rules and the ordered next steps for a weaker agent after the weekly token limit
- `2026-10-18-deleted-code-triage.md` — NEXT — Look, AI door — verdict (replaced, dead, later, restore) for each of the 54 files the rot cuts removed, the rows that name them, and the order to bring back the look judge, display modes, clip_hold and texture streaming
- `2026-10-18-cleanup-report.md` — LATER — none — conservative repo cleanup: git state and stashes, branches and the stale `origin-tags` ref, `target/` sizes, the three stale private dirs deleted by exact path, disk space before/after, and what was left with reasons
- `2026-10-18-row-audit.md` — LATER — none — read-only board audit: 801 open rows across 23 shards (96 parked), open rows that name a gone file or test, the 30 most-blocked rows, and the owner decisions still open
- `2026-10-18-docs-audit.md` — LATER — none — docs/plan pointer sweep: the one missing index line and one gone `systems/` path fixed, with the pinned stale refs, EMPIRE refs and future paths left and listed with reasons
- `2026-10-19-owner-queue.md` — NOW — all four — the rows only a person can close (a run or a decision) with the exact command or question, and the record of the owner's delegated calls of 2026-10-19 (every A item decided, the 32 obsolete and the gone-code ticks resolved)
- `2026-10-19-progress.md` — NOW — all four — the dated progress log (pass one to five): rows landed and judged with run ids, the janitor sweeps, and the two tool fixes (`nonstop` timeout classification, `docs-coverage` multi-line attributes)
- `design-review-2026-10-17.md` — NEXT — all four — 13 ranked design findings (DSN1-DSN13) with a board row each
- `shady-knight-port.md` — NEXT — Look — the owner's own game as donor: what is ported (walk bob, SG6), the numbers each row uses, and why the rope swing (SG4) waits on a fighter height channel (SG5)
- `render-look.md` — NOW — Look — moving fighter captures on the RTX 3050 stand beside the references: arena kit, window build, Q1 reference captures
- `body-and-break.md` — NOW — Bodies — cuts, limbs, ragdoll, collapse and fire give one hash on every PC and in replay (physics base inside)
- `hard-systems.md` — NOW — AI door — published, reproducible AI-door scores beside rivals, and the proofs behind them
- `multiplayer.md` — NOW — Multiplayer — the arena duel played by 2+ people on LAN and internet, server-authoritative with rollback, one hash
- `seats.md` — NOW — all four — seats (counts are knobs) that each land output or end NOOP with proof (loop tools in the AW part)
- `README.md` — NOW — all four — this index, the plan template and the prefix table
- `content.md` — NEXT — Look — licensed characters, clips and props cooked to game data, one provenance row per file
- `animation.md` — NEXT — Bodies — fighters move with ground feel and LOD at crowd scale without a second runtime
- `ai-native.md` — NOW — AI door — any model drives the engine through one typed door; every change checked, undoable, receipted
- `ai-door.md` — NOW — AI door — the five FOCUS Now AI-door slices (AL1, DP1, RB1, LF16, HS20) plus the describe ledger (AID1)
- `spatial.md` — NOW — AI door — the engine understands a scene as numbers before any model is asked
- `spatial-entry-points.md` — NEXT — AI door — which `crates/spatial` function answers which agent question, for the `scene.query` and `scene.spatial` verbs
- `spatial-llm-ab.md` — NEXT — AI door — how to get the real-model spatial number (SP20, SPR4): the cassette harness is done, no model figure exists yet
- `spatial-names.md` — NEXT — AI door — names, tags and sizes for door actors instead of `prop_NNNN`, the smallest change that keeps the golden
- `quality.md` — NOW — Look, Bodies, Multiplayer — the engine stays fast and honest: measured budgets, loud faults, no faked green
- `potato.md` — NOW — all four — every game looks great and runs on a weak PC: wiring gates, the potato tier (gate G9), content fetch and small steals (rows GS1-GS33)
- `landscape-ideas.md` — LATER — none — the idea backlog from 74 scouts (not rows yet); a planner run promotes one cluster at a time
- `frontier.md` — NEXT — Look, AI door — every research idea earns a spike (one test, one baseline, a kill rule) before it becomes a row
- `spear-grapple.md` — NOW — Multiplayer — a throwable spear tethers to an enemy or a wall and drags the fighters with integer physics; opt-in `MatchSpec::spear`, old digests unchanged
- `2026-10-17-obsolete-rows.md` — LATER — none — seven board rows (E4 E6 E12 E17 E21 E22 E26) that no longer match the tree, listed for the owner to close
- `gameplay.md` — NOW — AI door — the duel feels right and AI fills empty slots fairly
- `living-world.md` — NOW — AI door — NPC minds and live AI lines run off-tick; only validated, tick-stamped actions enter the tick
- `runtime.md` — LATER — none — one runtime and one world model under the hash law
- `urban-war.md` — LATER — none — a second, different game proves the engine is general (frozen by FOCUS D7)
- `silicon-city.md` — LATER — none — a future game: build a CPU like a city and a factory, ride one instruction in first person, compete on a performance/power/area skyline with humans and bots, and check the best designs in real open simulators (rows SIL1..SIL45, not on the board yet)
- `silicon-world.md` — LATER — none — the look and world half of Silicon City: the chip as a walkable, drivable GTA-style city (RAM harbor, ghost town, subway), zoom with no load screen, time dial, radio stations as workloads, missions, every visual tied to a sim counter (rows SIL46..SIL59, not on the board yet)
- `ryl1.md` — LATER — none — the RYL1 soul as a faction-war game: two asymmetric races, aimed combat, three capture points, 6v6 with AI seats (rows RY1-RY10, not on the board yet; owner door 2026-10-20)
- `os-game.md` — LATER — none — the headless "You're the OS" game (`crates/ossim`: 4 CPUs, RAM and swap, one I/O queue, starvation, 12 stages), the window and web shells still to build, and the GPL idea-only note (rows OS1..OS5, not on the board yet)
- `parked.md` — LATER — none — parked areas (world, audio, editor, shipping UI) stay small; each names the Core proof that un-parks it
- `forge-adoption.md` — NEXT — AI door, Bodies, Look — what the engine takes from Forge and the old Flax plugin: 34 board rows (AD1..AD29, PX40, AX23, EA6, PF24, SP66), 30 next items, 26 engine defects, 14 owner decisions, a 90-line ranked second-pass catalog
- `steals.md` — LATER — none — open donor steals per bar, at most 3 per bar (data, not a plan)
- `rome-spec.md` — LATER — none — shared Rome arena spec for two engines, with the open parity gaps digest
- `BOOK-GAPS-2026-10-12.md` — NEXT — all four — book review 1 (Gregory GEA, Lengyel, Modern Game Testing) vs code: wired-but-unused W1-W18, render order, decisions D1-D6 (a review, not rows)
- `BOOK-GAPS-2-2026-10-12.md` — NOW — all four — book review 2 (30 books: engine, math, Rust, architecture, MCP, game design) rows N1-N40; boarded as GP27-GP42, GA13-GA15 and the engine rows in `../SPRINT-20-game.md`
- `BOOK-GAPS-3-2026-10-19.md` — NOW — all four — book review 3 (Dunsky, Beuken, GPG8, Rust official docs, shelf) rows D1-D2 on AM14/AM15, G1-G4 as NB3-NB5/AM13, R1-R2 as NB1-NB2 in `../SPRINT-23-books.md`
- `ATLAS.md` — LATER — none — generated plan map (`node tools/plan-atlas.mjs --write`); never hand-edited
- `DP1-deepdive.md` — NOW — AI door — stuck-gate deep dive for DP1 endpoint door net plus perf packs
- `EG8-deepdive.md` — NOW — Look — stuck-gate deep dive for EG8 live pass-truth leg
- `EG18-deepdive.md` — NOW — Look — stuck-gate deep dive for EG18 animation from sim state: bridge caller, compiled machine, graph sample
- `GS30-deepdive.md` — NOW — Look — stuck-gate deep dive for GS30 spatial isovist: lock the budget (GS38), then tick the row
- `SP63-deepdive.md` — NOW — Look — stuck-gate deep dive for SP63 draw-id target: land the two foreign fixes first, then slice 1
- `lf10-pose-hold-rethink.md` — NOW — Bodies — stuck-gate rethink for LF10 pose-hold hitstop: slice A land plus slice B defer
- `fps-view.md` — NEXT — Look — a first-person eye on a fighter body: eye pose, look channel beside the tape, camera feed, no-clip rule (rows FV1-FV6)

## How a plan is written

A plan turns one subsystem into board rows a coder can finish in a session or
a few. `PLN` rows on `../SPRINT.md` produce these files through the `planner`
agent; the orchestrator reviews each file and moves its accepted rows onto the
board. Plans are execution specs, not research catalogs.

Find any plan, row or crate in `ATLAS.md` (generated: `node tools/plan-atlas.mjs
--write`; queries `--row ID`, `--crate NAME`, `--priority look|bodies|aidoor|speed|proof|other`,
`--plan NAME`, `--find TEXT`). Its section 5 lists what is no longer true in each
plan and section 6 is the refresh queue: the weakest Core plan is the next deep-dive.
Status 2026-10-16: disk shows `docs/plan/ATLAS.md` (142 lines) has headings only at `:13 ## 1.`, `:52 ## 2.`, `:107 ## 3.` and no section 5 or 6, and `tools/plan-atlas.mjs:227,240,250` writes exactly those three, so the "section 5 lists what is no longer true" and "section 6 is the refresh queue" halves are stale; the old shape lives in git history only (`git log -p -- docs/plan/ATLAS.md` shows `## 5. Rot: what is no longer true` and `## 6. Refresh queue: the weakest Core plans first`), and today the section-1 health column carries that signal (`N section-7 row(s) off the board`).

## Rules

0. If `../systems/<area>.md` exists, read it first and carry over what the
   code still confirms; your plan replaces it after review.

1. Code beats docs. Read the subsystem's crates (module headers, key types,
   tests) and state what is implemented, unit-tested, integrated, and GPU- or
   visually verified. A CPU reference is not a GPU feature; a stub is a stub.
2. Plain language first: each section opens with one or two sentences a
   newcomer understands, then the precise detail for coders.
3. Depth over breadth (`../FOCUS.md`, `../EMPIRE.md`): engine systems are
   general (any genre could be built; headless fixture games prove it), the
   game we ship is Rome at war. CUT items get no Rome content, never a
   genre assumption baked into an engine crate.
4. The slim law (`../CODELAW.md`): every row names its consumer, replaces or
   deepens existing code, states its budget, and adds no mirrored types.
5. Donors: port or adapt donor code from any source (AGENTS rule 4, owner 2026-10-10). List each donor in `steals.md` with the pinned commit, the link, and what we take.
6. Determinism (`../HASH-LAW.md`): authoritative simulation uses only
   `+ - * /`, `sqrt`, integer or fixed math and stable orders; presentation
   may use anything.
7. No builds while planning; `cargo metadata` or `cargo tree` only.

8. Keep it true: after editing a plan, run `node tools/plan-atlas.mjs --plan <name>`
   and fix what it lists (gone evidence, a base commit that is not a sha, section-7
   rows missing from the board, an Accept with no number), then `--write`.

## Template (exact headings, 250–450 lines)

```
# <Subsystem> — plan to the VISION.md bar
Plain summary. Owner lanes. Prefix. Base commit.
## 1. Where it stands        capability | status | evidence (file or commit)
## 2. Finish line            Tier A alpha, Tier B v1, Tier C VISION bars, measurable
## 3. Target design          data flow, ownership, threads, determinism, budgets, API sketch
## 4. Hard problems          problem → technique → why → reference → donor
## 5. Donors                 one line: `Donors: steals.md; porting policy is AGENTS rule 4.`
## 6. Slim                   delete, merge, freeze, and what we will not build
## 7. Board tasks            rows in board format; then "Row changes" for existing rows
## 8. Quality gates          tests, benchmarks, captures, budgets kept green forever
## 9. Session estimate       sessions per tier A/B/C
```

## Board row format

One task per bullet, continuation lines indented two spaces, `needs:` last
and ending with a period, every ID in `needs:` already on the board or in the
same plan:

    - [ ] RL3 render: GTAO ambient occlusion at half resolution. Tier A.
      Size M. Owns `crates/render/src/gtao.rs`. Accept: llvmpipe parity
      with a CPU reference; 0.8 ms on the reference GPU at 1080p. needs:
      RA1, RL1.

Sizes are agent sessions: S = 1, M = 2–4, L = 5–10 (split anything bigger).
Mark acceptance that needs the operator's RTX 3050 as "(reference GPU)". Rows
already on the board with your prefix are yours: keep their IDs, put text
changes under "Row changes", and number new rows after the highest ID.

## Files and prefixes

Which Scorecard rows of `../VISION.md` each plan serves lives in its "Plans" map (the check
FAILs on a plan missing there); a new plan adds its row there in the same commit.


| Row | File | Prefixes |
|---|---|---|
| PLN1 | `runtime.md` | RC, ARC, GE |
| PLN2 | `render-look.md` | RL, RP, RA, GX, VS, WIN, GM |
| PLN3 | `body-and-break.md` | BX, PX, DX |
| PLN4 | `animation.md` | AM, LM, TV |
| PLN5 | `multiplayer.md` | MP, ON |
| PLN6 | `gameplay.md` | GP, GA, NV, CB |
| PLN7 | `ai-native.md` | AX, MCP, EA, EV, GW |
| PLN8 | `hard-systems.md` | HS |
| PLN9 | `content.md` | CT, AS |
| PLN10 | `quality.md` | PF, AH, SL, CQ, OPT, FB |
| PLN11 | `parked.md` | WD, SN, ED, PLF, UIX |
| PLN12 | `living-world.md` | LW, AB, LAI |
| PLN13 | `urban-war.md` | UW, UC |
| PLN14 | `frontier.md` | FU, NS |
| PLN15 | `spatial.md` | SP |
| PLN16 | `seats.md` | AW |
| PLN30 | `potato.md`, `landscape-ideas.md` | GS |
| — | `rome-spec.md` | none (Rome arena spec; `rome-spec.json` is its machine copy) |
| — | `steals.md` | none (scout list, not a plan) |
| — | `EDITOR-INDIE-2026-10-19.md` | EI (SPRINT-24) |

## Facts to start from

Run `node tools/finish.mjs`, then read the evidence of the bar your plan moves.
`../PLAN-REVIEW.md` is a revision-specific review, not today's baseline.
Every proposed row names the vision outcome, measured baseline (or UNKNOWN),
target, real consumer, proof command, dependencies and a bounded next task.
Read `../VISION.md` Research contract before extending a plan from outside work.

## Indexing reusable pieces

Every plan indexes its reusable pieces in `../library/index.md`
(index pointers, not copies). Before writing anything reusable,
check the library index. If it does not exist, add it there
first, then reference it in the plan.

| Reusable piece | Library entry | Plans that reference it |
|---|---|---|
| Counting-allocator floors | [counting-allocator-floors.md](../library/counting-allocator-floors.md) | `quality.md` (PF, SL), `body-and-break.md`, `runtime.md` |
| Buffer reuse with counting-allocator floors | [buffer-reuse.md](../library/buffer-reuse.md) | `quality.md` (SL18–SL20), `animation.md`, `body-and-break.md` |
| Log-before-pop journaling | [log-before-pop-journaling.md](../library/log-before-pop-journaling.md) | `parked.md` (ED8) |
| Contract-test patterns | [contract-test-patterns.md](../library/contract-test-patterns.md) | `quality.md` (SL10, SL20), `animation.md`, `body-and-break.md` |
| Cross-crate harness wiring | [cross-crate-harness-wiring.md](../library/cross-crate-harness-wiring.md) | `quality.md` (all proof rows) |
