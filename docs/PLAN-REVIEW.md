# Plan review — verdict and evidence

Background only since 2026-10-10: the order of work is `FOCUS.md` "## Now".

Current review: 2026-10-07 at `19dabe4` (board: 1,031 tasks, 327 ticked,
341 ready per `node tools/sprint-loop.mjs --dry-run`; workspace: 36
packages). The 2026-10-01 review is kept below; earlier ones live in git
history (`git log -p -- docs/PLAN-REVIEW.md`). The live scoreboard is
`node tools/finish.mjs` (FS1).

## Verdict 2026-10-07

The design is right; the finish is behind. engine2040 already is the
"neuro-symbolic" engine that the owner's pasted plan (2026-10-07) asks for:
models propose, code decides, replay never calls a model. What it lacks is
closure on the four Core proofs, not more ideas.

## Scoreboard (`finish.mjs`, run 2026-10-07)

5 met, 16 partial, 5 open. On 2026-10-01 it was 7 met: A9 and A10 went back
to partial when new rows were mapped to them (OB3; DOC1 and FX3). Met: A1,
A2, A3, A6, A7. Open: A8 (GM3), B, C3 (VG1–VG4), P8 (SL1–SL3), P10 (FS1).

## The four Core proofs (`FOCUS.md`)

| Proof | Row | State 2026-10-07 |
|---|---|---|
| Look | Q1 | open on the board |
| Bodies | BX28 | open |
| AI door | HS20 | open (A6 75/75 is met; the public mirror is not) |
| Multiplayer | MP, ON | C4 partial: ON6, ON7, ON8 open |

## What moved since 2026-10-01

- The spine started: ARC1–ARC4 ticked (`crates/ecs/src/spine.rs`,
  `crates/arena/src/spine_step.rs` behind the `spine` feature). ARC5–ARC19
  stay open. Risk 1 and the "no `spine.rs`" cell below are stale.
- DT5 (dataset provenance) ticked; HS15 (TOP) is still open.
- Owner D8 and D9: no Bevy viewer, no Bevy dependency; the own renderer in
  `crates/render` is the only picture.

## What the plans get right

1. One law: models propose, deterministic code decides, replay never calls a
   model (`AI-NATIVE.md`, `HASH-LAW.md`).
2. One door: every edit is typed, checked, undoable and receipted; A6 is met
   on mock, local, remote and an external MCP client.
3. Proof culture: golden hash x2, tapes, forks, causal replay.
4. Generation that verifies: brief to level, character, encounter and slice
   landed (AB1–AB4, AB9) with repair rounds and fork balance.

## What is weak

1. All four Core proofs are still open.
2. Too many files set the order: VISION Priorities, FOCUS Core, TOP,
   Program and D1–D9, this file's order list, ROADMAP M0–M6, the
   NORTH-STAR pillars and EMPIRE. A writer can find two different "firsts".
3. Weight: 50 plan files, a 600 KB board, 341 rows ready at once, and
   three games plus AXIOM planned before game one ships.
4. Drift: `plan/frontier.md` still says "Bevy becomes the body"
   (S47), which D9 killed. `AI-NATIVE.md` pillar 5 said "not implemented"
   after VF1 and AB1–AB4 landed (fixed in this pass).
5. Generation stops at one room: `level.build` makes one 10 m room with at
   most 24 props. Nothing joins rooms into a level with gates and keys.

## Owner paste 2026-10-07 ("ForgeAI" world compiler)

Full verdict and rows: `RESEARCH-VERDICT.md`, "World compiler paste". Short
form: 17 ideas we already have; 6 gold rows proposed and not boarded
(AB17–AB22: a level graph with locks and keys, multi-room briefs, a bench
over seeds, experience search, changes with keep-rules, function-first asset
briefs); 8 later; 7 drop (rename and restart, `bevy_ecs` as core, runtime
generation in the shipped game, and more).

## Restructure proposals (owner call; not done)

1. One order source: FOCUS Core plus Program. The other files link to it
   instead of restating it.
2. One planner pass to bring `plan/frontier.md` in line with D9.
3. Board AB19 now: Size S, it measures what exists. Board AB17, AB18, AB21
   and AB20 behind HS20 as Core 3 work. No new plan file (D7).
4. Keep the board small: archive ticked chains and the BLOCKED BV rows.

The order of work stays FOCUS Program; nothing here reorders it.

## Review 2026-10-01 (kept; spine lines stale)

Current review: 2026-10-01 at `b47886f` (board: 1,113 tasks; workspace:
30 crates plus the root package). Earlier reviews live in git history
(`git log -p -- docs/PLAN-REVIEW.md`). The task list is the lane board
in `SPRINT.md`; the live scoreboard is `node tools/finish.mjs` (FS1).

This supersedes the 2026-09-23 verdict ("no window, ~117k lines, 1,220
tests, nothing playable"). That baseline is stale: executables now
exist, 7 of 26 finish gates are met, and the window runs forward PBR
with GPU skinning. What follows is true of the code at this HEAD.

### Scoreboard (from `finish.mjs`, re-verified 2026-10-01)

7 met, 16 partial, 3 open. Met: A1 (pinned toolchain, golden
`d57b5c8ae3096614` x2, seeded tapes with negative tests), A2 (recorded
editable-scene run, save/reopen byte-identical, offscreen captures),
A3 (recorded editor run: pick, gizmo, undo/redo, crash recovery), A6
(75/75 authoring tasks across mock/local/remote plus external MCP
client), A7 (seeded-bug tape replays, causal replay names the input),
A9 (chaos: healed 3, quarantined 9, fuzz 20/20 clean), A10 (tuning edit
0–1 ms, bad edits refused). Open: A8 (GM3 clean-folder run), B
(UW39 second game), C3 (virtual geometry). Everything else is partial.

### What is real vs stubbed

| Area | Real today | Still stubbed / open |
|---|---|---|
| Executables | `arena.exe` (`game/src/bin/arena.rs`, WIN1), `staged_fight`, `e2040-editor`, `e2040-mcp`, `bundle_cook`, `perf`, proof CLI | GM3 clean-machine run (A8) |
| Renderer | windowed forward PBR, GPU skinning, captures | shadows, IBL, AO, TAA, post, sky, particles as CPU twins (SL3); GX1–GX10 ticked but the C1 evidence receipt (PQ19/PQ20) open; Q1 reference captures open |
| Simulation | headless arena, checkpoints, forks, replays; LAN + dedicated + lobby (MP1–MP3) | ECS spine: ARC1–ARC19 open, no `spine.rs`, arena still owns its own state and never imports `engine-ecs` |
| AI loop | command service, 20-task eval, spatial checks, seeded-bug playtest agent | P7/P10/C1 evidence lines (PQ12–PQ20 chain); latency GPU leg (PQ18) |
| Content | glTF cook path, fixture genres (GE1), second-game checklist (GP5) | arena kit/hero/crowd content (CT7–CT8); urban-war game rows UW34–UW38 |
| Hardening | supervisor catalog, chaos, museum, bug tapes | hour soak (PQ3), WAN fault soak hour run, GM3 |

### Top 5 risks

1. **The spine migration is unstarted.** 19 open ARC rows; arena, net
   rollback, save and editor all run on the legacy path. Every new sim
   feature deepens the migration debt (plan: `runtime.md`, spine section).
2. **The look is still mostly CPU math.** SL3 twins stay reachable from
   the window build; Q1 captures and PQ19/PQ20 decide whether GX rows
   were integration or new stubs.
3. **Nothing is proven shippable.** GM3 open means no clean-folder run;
   B open means no second game. A5 stays partial until both land.
4. **Evidence hygiene lags the board.** P3/P7/P10/C1 print partial with
   zero open rows mapped; the PQ12–PQ20 chain exists to close that, and
   7 met lines still lack commit SHAs (PQ14/PQ15).
5. **Board weight vs lane throughput.** 580 KB / 1,113 tasks with ~450
   ticked; packet context and merge cost grow while builds queue on the
   same cores. Keep waves narrow until GM3 and Q1 move.

### Order and cost (sessions, revisit after GM3 lands)

1. Prove shippable — GM3, Q1, PQ19/PQ20, PQ12–PQ17: ~15.
2. Spine — ARC1–ARC7 with the parity harness first: ~40–60.
3. Look — RL/RP tails, CT7–CT8 content, V2–V3 suite: ~120–200.
4. Move and fight — LM/TV/PX tails, GP depth, DX runtime: ~100–150.
5. Online and world — ON6–ON8, WD3/WD5, UW34–UW39: ~80–120.
6. Harden — PQ3 soak, PF4/PF5, SL3/SL5/SL16 retirement: ~60–100.
Total from here about 420–650 sessions, inside the remaining budget if
waves stay narrow and every packet names its consumer and proof.

Copy freely but only from the `deny.toml` allow-list, with a
`PORT(owner/repo@rev:path)` marker and an `EVIDENCE.md` row; verify the
LICENSE file first. Never Unreal, Flax, Unity or Blender source, Epic
or GASP content outside Unreal, GPL/LGPL/AGPL/MPL code, or
non-commercial data such as LAFAN1.
