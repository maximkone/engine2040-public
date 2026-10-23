# START HERE: route to one skill

New session, any model. `AGENTS.md` wins over every skill. Read this router, then open one skill that matches the task. The core map below explains common choices; it is not a mandatory reading list. The full map is `skills/SKILLS.md`. Open a skill as `skills/<name>/SKILL.md`.

## Core map

| # | skill | why useful |
|---|---|---|
| 1 | `repo-orientation` | where things are; the narrow search that does not die on the glTF |
| 2 | `commit-and-board` | one shared tree: commit by path, never `git add -A`; how a row is written and closed (`Landed <sha7>`, evidence) |
| 3 | `github-check` | heavy cargo runs on GitHub (`node tools/gh-check.mjs ...`), never the local lock; what exit 2 or 3 means |
| 4 | `test-and-proof` | what counts as proof; never write DONE or green without it |
| 5 | `debug-triage` | root cause before fix, stop after two identical failures |
| 6 | `engine-code` | how Rust is written here: reuse first, typed errors, no stubs |
| 7 | `rust-quality-ratchet` | the pre-commit hook and the debt counts that only go down |
| 8 | `wiring-a-feature` | much of the plan is code that exists and nothing calls: wire before you write |

## By task

| you are about to | read, in this order |
|---|---|
| fix a bug or a red check | `debug-triage`, `github-check`; a hash moved: `determinism-debug` |
| land a row | `commit-and-board`, `test-and-proof`; a refused commit: `rust-quality-ratchet` |
| add a feature | `repo-orientation`, `wiring-a-feature` (does it already exist?), `engine-code`, then the area skill below |
| review a diff or audit an area | `review`: depth score, rust bar, callers, captures, API review |
| write unsafe, statics, atomics, locks, threads, real-time callbacks, sim numeric casts | `rust-safety` |
| change the fixed tick, live window loop, frame budget, asset handles, startup, crash hook | `tick-and-runtime` |
| read a research card, donor repo, paper or spike | `research` |
| work on the web build | `web-wasm` (section 8: the browser pane check of the published page), `github-check` (web-run) |
| work on physics, breaks, ragdolls | `physics-bodies`, `determinism-debug`, `door-and-headless-runbook` (arena receipt lines), `wiring-a-feature` |
| work on look, render, lights | `wgpu-render`, `render-pipeline-math`, `render-lighting`, `door-and-headless-runbook` |
| work on fighters, anim, feel | `anim-combat`, `door-and-headless-runbook`, `design-and-playtest` |
| work on the shell, HUD, menus, sound | `game-shell-audio-ui` |
| work on sight, hearing, AI placement | `spatial-perception` |
| add content or assets | `assets-content`, then `render-cull-vram` if it costs VRAM |
| online play | `net-rollback`, `determinism-debug` |
| the MCP door or AI players | `ai-door`, `door-and-headless-runbook` |
| a file seems to be missing | `recover-deleted-code` |
| edit docs or the board | `commit-and-board`; then the docs rules below |
| measure speed | `perf-bench` |
| run the game without a person | `door-and-headless-runbook` |

## Five rules that apply to every task

1. Start: `git pull --no-rebase --no-edit origin master`, then `git status --short`. Lines you did not make are another session's: never revert them.
2. Board work: `node tools/board-slice.mjs --focus`, then `node tools/board-slice.mjs <ID>`. A direct user request needs no new board row. Never read the board whole.
3. Iterate with `cargo check -p <crate>` only. Every test and clippy proof goes through `node tools/gh-check.mjs cargo ...`.
4. The golden hash is `d57b5c8ae3096614`. If it moves you have a bug or a receipt to write; never re-pin it.
5. Review `git diff --check -- <paths>` and `git diff -- <paths>`. Commit only your paths: `git commit -F <msgfile> -- <paths>` (`git add -N` for a new file). Pull, then push `master`. No rebase, amend or force-push.

## Docs tasks

- A new doc gets one line in the index that lists it, in the same commit: `docs/README.md` map, `docs/INDEX.md` contracts, `docs/plan/README.md` for a plan, `skills/SKILLS.md` for a skill.
- After a board or doc edit run `node tools/workspace-check.mjs`. For agent instructions or skills: `node --test AGENTS.test.mjs tools/skills-index.test.mjs`.
- Retire a doc with `git mv` into `docs/archive/<date>/` and fix every link to it.
- Generated files (`docs/CRATE-MAP.md`, `docs/plan/ATLAS.md`) are never hand-edited.
- A skill: folder `skills/<name>/SKILL.md`, `name` equals the folder, `description` starts "Use when" and is a task you can match, under 300 characters, file under 250 lines, no `file:line` cites.

## Report like this

Changed paths, the checks you ran with exit codes and counts, the hash if sim code changed, open gates, and a commit sha only if you made one. Short plain English.
