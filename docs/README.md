# docs/ — the map (one screen)

Order of work: `FOCUS.md` "## Now". Rules for AIs: root `../AGENTS.md` (wins) and `../CLAUDE.md`. The contract table is `INDEX.md`.

| Need | Go to |
|---|---|
| What to do next | `FOCUS.md` "## Now", then `node tools/board-slice.mjs --focus` / `--ready` / `<ID>` |
| Rows (the board) | `SPRINT.md` is the index; rows live in the `SPRINT-NN-*.md` shards. Done rows: `SPRINT-ARCHIVE.md`. Slice, never read whole |
| Plans (one subsystem each) | `plan/README.md` indexes them; `node tools/plan-atlas.mjs --row <ID>` finds a row's plan; `plan/status-2026-10-17.md` is the truth check |
| Where the engine is going | `VISION.md`, `ROADMAP.md`, `PLAN-REVIEW.md`, `AI-NATIVE.md` (tables: `VISION-TABLES.md`) |
| Code layout | `ARCHITECTURE.md` (layers), `CRATE-MAP.md` (generated), `../.opencode/modmap.md` (files per crate, Grep only) |
| Code rules and gates | `CODELAW.md` (guides), `RUST-QUALITY.md` (enforces), `HASH-LAW.md` (golden hash), `WORKFLOW.md` (how a change flows) |
| Blockers, evidence, deps | `BLOCKERS.md` (never read whole), `EVIDENCE.md`, `DEPENDENCIES.md`, `DECISIONS-M1.md`, `adr/` |
| Dated proof receipts | `receipts/` (capture frames and run receipts, one folder per date), `evidence/` (hashed frame digests) |
| How-to for users and AIs | `book/` (mdBook: quick start, AI door, recipes); `library/` (reusable patterns) |
| Game design notes | `design/` (index `design/README.md`) |
| Specs left from the old systems set | `systems/` (`ai-door.md`, `observability-heal.md`) |
| Research | `research/COMPARE.md` (rubrics), `research/LADDER.md` (read one rung), `research/steals-*`, `research/web-*`, `research/hunt-2026-10-21/` (the 2026-10-21 GitHub hunt: `00-README.md` one screen, `10-donor-map.md` donor -> open row), `research/camera-mods-combat-2026-10-19/` and `research/finish-line-2026-10-17/` (dated dives; the last one feeds the gap list in `plan/finish-line.md`); old catalogs are in git history only (`archive/INDEX.md`) |
| Skills | `../skills/START-HERE.md` (read first), then `../skills/SKILLS.md` |
| Archive (no authority) | `archive/INDEX.md`; dated folders `archive/2026-10-NN/` are moved history, kept in git |

## Rules of thumb

- Plans say how, the board says what is open, `FOCUS.md` says in what order. If two docs differ, the newer owner line in `FOCUS.md` and `../AGENTS.md` win.
- A new plan file gets one line in `plan/README.md` in the same commit (`tools/plan-index.test.mjs` checks it).
- Generated files are never hand-edited: `CRATE-MAP.md` (`node tools/crate-map.mjs`), `plan/ATLAS.md` (`node tools/plan-atlas.mjs --write`).
- Retire a doc with `git mv` into `archive/<date>/`, fix every inbound link, add one line to `archive/INDEX.md`.
- After board or doc edits: `node tools/workspace-check.mjs`. Pinned docs: `node --test docs/INDEX.test.mjs tools/index-check.test.mjs tools/plan-index.test.mjs`.

## Known leftovers of the retired loop (owner 2026-10-17: no sprints, locks, claims, queue, keeper)

Not removed because a live test, tool or board row still names them: `plan/seats.md` (loop seats, AW rows), `../team/` (`tools/frame-moved.mjs` reads `team/look.md`), `../opencode.jsonc`, `../arsenal.json`, and loop wording inside `FOCUS.md` and the dated `SPRINT-*.md` shards. Read them as history.
