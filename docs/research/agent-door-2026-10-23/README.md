# The shape of agent tooling for a game engine

Read 2026-10-23, live from the default branch, every file pinned by commit SHA
below. One question: what do shipped engine-MCP repos have that our door does
not? Our door is already undoable, refusing and journaled, so the table only
counts a delta where we can point at a missing field or a missing number.

Candidates were found with one narrow GitHub search per idea (unity / godot /
unreal / blender + mcp, awesome-claude-skills, game-dev agent skills). Only
repos whose files I actually read are in the table.

## Where we stand today (facts from our tree, not prose)

| Fact | Value | Where |
|---|---|---|
| refusal sites / distinct codes | 348 sites, 177 codes | `McpToolError::new("...")` across `crates/endpoint/src` |
| codes carrying a structured advice field | 0 (advice is inside the message string) | `crates/endpoint/src/mcp_refusal.rs` |
| discovery budget / default list / biggest pack | 75,000 / 58,112 / 70,454 chars | `crates/endpoint/src/mcp.rs` budget comment |
| agent instructions | `AGENTS.md` 9,288 bytes, 30 skills | repo root, `skills/` |
| cold-start build tasks with a pass rate | none | `crates/playtest/src/eval.rs` rows are engine playtests, not agent builds |
| per-result cap with a recovery hint | none | no `max_chars` / `RESULT_TOO_LARGE` in the door |
| undo, receipts, revisions | present | `tx.preview`/`tx.apply`/`tx.undo`, `receipts.rs`, `watch_ledger.rs` |

So four of the five things the market measures we already do. The missing ones
are measurement (a task ladder), a machine-readable error shape, and coaching
of the model inside the result.

## The candidates

| repo @ sha | shape | verbs | refusal | undo / records | what our door does not have |
|---|---|---|---|---|---|
| `SummerEngine/summer@88d0a5b6` | engine built for AI: MCP server + CLI + skills + a library | 130 tools, 132 skills, 19 templates, all counted in generated `registry/generated/counts.json` | schema rewritten for strict providers (tuples, `$ref`, `type` arrays) so one tool cannot 400 the whole request; toolsets/profiles with a cap | `.summer/project.json` project memory; outcome replay of agent trajectories against pristine fixtures; canary proxy that hides one tool per arm and enforces a call budget | **the eval ladder E0-E5** (fixed prompts, binary gates, per-rung cost/tool-calls), **count as one generated file that docs claims are checked against**, **result cap that tells the model how to get the rest**, **repeat-single-edit nudge** |
| `ChiR24/Unreal_mcp@8d7e1495` | one public `unreal` gateway tool (`search`/`describe`/`execute`/`configure`) over 23 internal parents | 23 parents behind 1 tool; capability records are the single source; `UNDECLARED_PARAMETER` refuses a param the C++ handler does not read | typed discriminated error algebra: 12 kinds, `code`, `retryable`, `suggestions[]`, `currentRevision`/`expectedRevision`, `resultChars` on `RESULT_TOO_LARGE`, `grantedScopes` | `compensation.ts` maps each mutating capability to an inverse capability, with a test folding ids to real records and an honest note that an inverse is not a rollback; receipts with redaction + a receipt contract test | **fields on the refusal** (`kind`, `retryable`, `suggestions[]`, `next`), **a per-verb inverse table with a test**, `registry:check` + `manifest:check` drift gates |
| `CoplayDev/unity-mcp@ed4e205d` | 50 focused entrypoints, editor bridge | `manifest.json` lists 50 tools; `manage_tools` shows/hides tool groups; `batch_execute` | `validate_script` / `script_apply_edits` validate C# (Roslyn) before writing; `read_console` after | `manage_editor` undo/redo; multi-instance routing (`set_active_instance`) | **a skill shipped next to the tools** (`unity-mcp-skill/SKILL.md`, 16.7 kB + `references/`), validation before the write instead of after, one batch verb as the default path |
| `AnkleBreaker-Studio/unity-mcp-server@a669788c` | 268 tools, one editor queue per process | 268; serialized writes, up to 5 reads batched per update | refuses while `isCompiling: true` and names the check; empty error list during compile is called out as not a pass | named undo groups + per-agent history; "which supported actions can be undone safely" | **undo eligibility as a question the agent can ask**, compile-in-progress as a first-class refusal |
| `IvanMurzak/Unity-MCP@fc059d5` | plugin inside the built game + 70+ tools in 4 categories | any C# method becomes a tool in one line; project-local custom tools | tool surface generated from the project, so refusals name real project types | generates skills per OS / engine version / installed plugins | **skills generated from the project**, not written once by hand |
| `hi-godot/godot-ai@b82b5c5` | 46 tools, 120+ operations | tool domains can be excluded per session; scripts are parse-validated and hot-reloaded | C# compile errors returned as rows | play loop with screenshots and outlines | domain-level exclusion as a config, not a code change |
| `ahujasid/mcp-for-blender@5a3618f` | one addon, ~20 tools | viewport screenshots back to the model; Poly Haven asset fetch | unknown attribute errors by name | `record_trajectory_feedback`, telemetry with an off switch | **the trajectory is recorded for later training**, with consent |
| `abagames/agentic-gamedev-skills@4ed8d75` | skills only, no server | none | n/a | n/a | **the layout**: `.agents/skills/<name>/SKILL.md` + `references/`, a 3.2 kB `AGENTS.md`, and a script that fails when README and skills drift |

Two shapes I checked and rejected as already ours: profiles/toolsets (our packs
plus the `collapse` ledger already hide verbs and keep them callable by name),
and strict-provider schema rewriting (our schemas are hand-authored from
`schemars` with no tuples, `$ref` or `type` arrays, so there is nothing to
rewrite today; a test that keeps it that way is the cheap part, not the rewrite).

## Steals, ranked

### P1 - the make-a-game ladder: six fixed prompts, binary gates, cost per rung

Donor: `SummerEngine/summer@88d0a5b6` `evals/end-to-end/README.md` and
`evals/README.md`.

What it is: E0 "start an empty project and run it", E1 "a ball I roll with
WASD", E2 "collect 10 coins and win", E3 "an FPS with one enemy that chases and
attacks", E4 "a small dungeon crawler, art spooky, with sound", E5 "new session,
no history, add a second level". Each rung has a fixed verbatim prompt, a fixed
starting state, and gates that are all-or-nothing: *a run that mostly works
fails*. Scoring is pass/fail plus wall-clock, tool-call count and cost. One
flake retry, two failures is red. The ladder definition is binding before the
runner exists, so skills are written to terminate in E2.

Integration home: `crates/playtest/src/eval.rs` (the `EvalTask`/`EvalRow`
vocabulary already exists) plus one runner `tools/agent-ladder.mjs`; prompts and
gates written down in `docs/plan/ai-door.md`. This is row #18 of
`docs/plan/finish-line.md`, but #18 asks for 10 referee-scored tasks and Play@k;
the ladder adds the horizon, the binary-gate rule and the per-rung cost columns.

Bounded reproduction: three rungs only (E0, E1, E2), one model, two runs each,
the same prompt string. Gates read door receipts that already exist: the play
receipt is clean, `world.query` counts the coins, the win is reachable by a
scripted input tape, the scene is saved.

Kill criterion: a rung whose gates pass on a build a person can see is broken
(the negative control), a gate that needs a human eye, or two runs that
disagree on unchanged code.

Number it moves: agent build pass rate 0 of 3 measured today to k of 3, with
tool calls and wall-clock committed beside it.

### P2 - the refusal is a record, not a sentence

Donor: `ChiR24/Unreal_mcp@8d7e1495`
`src/tools/catalog/capabilities/semantic/errors.ts` (12 kinds, strict schema,
`retryable`, `suggestions[]`, revision pair, `resultChars`), plus
`semantic/errors.ts`'s `NextCall` and `records/compensation.ts`.

What it is: every failure crosses the boundary as a typed object, not a string.
The strict schema is itself the refusal: an unknown key or a missing field
cannot cross. `suggestions[]` is the actionable part, `retryable` tells the model
whether to try again or change the call, and `next` hands it the call it should
make. Ours names the cure in English inside `message`, which a model can only
use by reading prose.

Integration home: `crates/endpoint/src/mcp_refusal.rs`, with the catalog row
added where `e2040://error-catalog` is assembled in
`crates/endpoint/src/resources.rs`.

Bounded reproduction: add `kind`, `retryable`, `suggestions: Vec<String>` and
`next: Option<NextCall>` to `McpToolError`; migrate the 12 most-hit codes
(`bad_ticks` 21, `invalid_tape` 13, `bad_extent` 11, `bad_fighters` 11,
`denied_op` 8, `too_many_inputs` 7, `bad_receipt` 7, `bad_id` 7,
`stale_revision` 6, `over_budget` 6, `lock_poisoned` 6, `io_failed` 5 = 133 of
348 sites); one test counts migrated codes with an empty `suggestions` and fails
above zero.

Kill criterion: the new fields push the advertised catalog past the 75,000-char
budget without a pack load, or a golden transcript changes, or ladder E1's
tool-call count does not fall once the fields are there.

Number it moves: refusal coverage 0 of 177 codes with machine-readable advice to
12 of 177 (133 of 348 sites), and the error-catalog resource lists them by kind.

### P3 - the result coaches the next call

Donor: `SummerEngine/summer@88d0a5b6` `src/mcp/tool-results.ts`.

What it is: text over a cap is cut with a note that says exactly how to get the
rest ("call again with maxCharacters: N, or narrow the request"), and the last
line - the feedback footer with the entry id - is kept across the cut on
purpose. A second mechanism counts successful single edits and returns one tip
on the 4th in a row, then every 6th: write one batch/script instead. Both are
answers to observed model behaviour, not documentation.

Integration home: `crates/endpoint/src/mcp_tools.rs` (where a tool result is
shaped before it leaves).

Bounded reproduction: cap any tool text at 8,000 chars; the note names the
narrower call the verb already has (`scene.read` row count, `log.read` byte
budget), so no schema and no budget change; keep the last line when it carries a
receipt or hash; a counter over single-edit verbs returns the tip at 4 and not
at 3. Two tests: the note appears once and names a real argument, the tip fires
at 4 only.

Kill criterion: a cap ever drops a line a gate reads (a hash or receipt), the
tip text reaches the advertised list, or a ladder rung's tool-call count does
not move.

Number it moves: worst-case chars returned per call (uncapped today, 8,000
cap) and tool calls per edit task.

## What I did not verify

- AnkleBreaker's 268 and IvanMurzak's 70+ tool counts come from their READMEs;
  I read the READMEs, not their registries.
- Summer's `evals/*` runners are TypeScript I read but never ran; its E0-E5
  ladder has no runner yet, its own README says so.
- Unreal's C++ handler side (`Public/`, the automation bridge) was not read;
  only the TypeScript catalog, errors and compensation.
- No repo in this table had a published pass rate for "an AI built a game" that
  I could confirm from a file in the repo. Summer's outcomes baseline is
  `baseline.json` and its ladder is unrun, so the pass-rate numbers are a shape
  to copy, not a number to cite.
- Nothing here was run against our door; every "we lack" claim is a source read
  of `crates/endpoint/src` plus the greps in the facts table.
