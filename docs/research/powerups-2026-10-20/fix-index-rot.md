# Fix index rot (2026-10-21)

Scope: the eight index files an AI or a newcomer reads first.
`llms.txt`, `README.md`, `docs/README.md`, `docs/INDEX.md`, `skills/SKILLS.md`,
`skills/START-HERE.md`, `AGENTS.md`, `CLAUDE.md`.

Method: every backticked path, every markdown link target and every
`node tools/...` / `cargo ...` command in those eight files was resolved
against the tree today (a script plus reads of every tool's own usage
block). Sizes were measured with `Get-Item`. Row ids were checked against
`docs/SPRINT.md` and the `docs/SPRINT-*.md` shards. Skill names were
checked against the folders in `skills/`.

Nothing was committed, staged or pushed. Only the eight owned files changed
(plus this report).

## Fixes

| file | what was wrong | the truth with its proof | the fix |
|---|---|---|---|
| docs/INDEX.md | `ARCHITECTURE.md` was given as "(17.4KB)" | the file is 21,551 bytes = 21.0 KiB today (`Get-Item docs/ARCHITECTURE.md`); at the commit that wrote the number (590e7082) it was 18,185 bytes, so the number was true once and drifted | "(17.4KB)" -> "(21.0KB, checked 2026-10-21)" |
| docs/INDEX.md | `research/LADDER.md` was given as "(199 KB)" | the file is 232,363 bytes = 227 KiB today (`Get-Item docs/research/LADDER.md`); it was 203,029 bytes at 590e7082 (199 KiB), so the number was true once and drifted | "(199 KB)" -> "(227 KB, checked 2026-10-21)" |
| docs/INDEX.md | the `fixtures/` row pointed at `docs/fixtures/`, a directory that does not exist and never did | `Test-Path docs/fixtures` = False; `git log --all -- docs/fixtures` returns nothing. The fixture dir is the repo root one: `fixtures/` with `fixtures/README.md` (the same cell already says `../fixtures/README.md`) | row label "fixtures/" -> "../fixtures/" |
| docs/INDEX.md | `docs/compaction-baseline.md` existed but no index named it (a doc nobody can find) | the file is there, dated 2026-10-20, "Measured on 2026-10-20 at HEAD 77789c2a"; it holds the four drift-gate lines (crates 42, src lines, lock names, dead flags). `crates: 42` matches the 42 crate dirs under `crates/` today | added one row after `CRATE-MAP.md` |
| docs/INDEX.md | `docs/receipts/` and `docs/evidence/` existed, were tracked in git, and no index named them | `git ls-files docs/receipts docs/evidence` lists 23 tracked files; both dirs are dated 2026-10-18..20 proof receipts and one hashed frame file | added two rows after `CAPTURE.md` (the `evidence/` row says where the provenance ledger `EVIDENCE.md` lives, so the two are not confused) |
| docs/README.md | same two gaps as above: `compaction-baseline.md`, `receipts/`, `evidence/` were not on the map | same proof as the rows above | added two map rows ("Tree and context size", "Dated proof receipts") |
| skills/SKILLS.md | the `rust-quality-ratchet` row named the tool as `rust-quality.mjs`, a path that exists nowhere (it is `tools/rust-quality.mjs`) | `Test-Path rust-quality.mjs` = False, `Test-Path tools/rust-quality.mjs` = True; the skill's own text says "`tools/rust-quality.mjs` counts debt per crate", and the same file's `research` row writes `tools/books.mjs` with the prefix | "`rust-quality.mjs`" -> "`tools/rust-quality.mjs`" |

Fixes per file: docs/INDEX.md 5 (2 stale size numbers, 1 dead path, 2 added
rows covering `compaction-baseline.md` and `receipts/`+`evidence/`),
docs/README.md 2 (added map rows for the same three), skills/SKILLS.md 1
(tool path), llms.txt 0, README.md 0, skills/START-HERE.md 0, AGENTS.md 0,
CLAUDE.md 0. Total 8 edits in 3 files.

## Proof after the fix

- `node --test docs/INDEX.test.mjs tools/index-check.test.mjs tools/skills-index.test.mjs tools/for-ai.test.mjs`
  -> 23 tests, 23 pass, 0 fail. The index truth line now reads
  "INDEX truth: 28/28 paths exist, 72 lines" (was 27/27, 69 lines).
- `node --test AGENTS.test.mjs tools/pages-workflow.test.mjs` -> 12 tests, 12 pass.
- `node --test tools/plan-index.test.mjs` -> 1 FAIL, and it is not mine:
  `build-cpu.md: proof 'tooling' must be from Look, Bodies, AI door, Multiplayer, all four`.
  The plan and its index line landed in commit 6d79a5dd ("plan: build CPU
  pool (OPT37-OPT42) ...") while this job ran; the test reads
  `docs/plan/README.md` and `docs/plan/*.md`, none of which this job touches.

## Left alone (could not verify, or a judgment call)

1. "Proof game: a multiplayer Roman arena" (`llms.txt:3`) and "The proof game
   is a multiplayer Roman arena on Windows" (`README.md:6`). `docs/FOCUS.md`
   decision D10 (owner 2026-10-21) moved the game to `docs/plan/rome.md` (one
   legionary, six chapters, first person through the helmet), but the same
   FOCUS.md line says "The arena duel stays the first proof and the golden",
   and the wave brief `docs/research/powerups-2026-10-20/RULES.md:16` still
   calls the arena the proof game. The claim as written is still true about
   the proof; only the game framing moved. Left, because re-writing it is an
   owner call, not a fact fix.
2. `docs/README.md:17` research row names only `research/camera-mods-combat-2026-10-19/`
   and `research/finish-line-2026-10-17/` as dated dives. Newer dated dives
   exist (`finish-line-2026-10-20`, `powerups-2026-10-20`, `ryl1-2026-10-20`,
   `faction-war-2026-10-21`, `spatial-2026-10-18`, `quality-2026-10-17`). The
   claim in the cell is true as written (`plan/finish-line.md:42` does cite
   `research/finish-line-2026-10-17/`), so nothing was deleted; adding the
   newer folders is a scope call for the lead.
3. `docs/README.md:16` and `docs/INDEX.md:44` write the two system specs as
   bare `ai-door.md` and `observability-heal.md`. They live in `docs/systems/`.
   `tools/index-check.test.mjs` resolves bare names by looking in
   `docs/systems/`, so the repo's own convention treats this as correct.
4. `AGENTS.md:22` and `README.md:9` say the hash runs "x2". `tools/proof.mjs`
   runs the 600-tick golden verify ONCE on purpose (`GOLDEN_VERIFY_RUNS = 1`,
   "a second 600-tick verify is duplicate work") and runs each of four seeds
   twice and compares. The shorthand is not wrong (the seed hashes do come
   out twice) but it is thinner than the tool. Left, because the honest
   rewrite changes the meaning of the sentence the owner wrote.
5. `docs/archive/INDEX.md` does not list the dated folder
   `docs/archive/2026-10-16-rot`, and `docs/SPRINT.md` does not list the
   shard `docs/SPRINT-26-motion.md`, which still holds one open row (AM28).
   Both files are not owned by this job. Report only.
6. `docs/CRATE-MAP.md` is stale: `node tools/crate-map.mjs --check` exits 1
   ("STALE docs/CRATE-MAP.md"). All 42 crate rows are present and correct;
   some field (dep or job line) drifted. The generator file is not owned by
   this job. The pointer in `docs/INDEX.md` and `docs/README.md` is correct.
7. `docs/plan/ATLAS.md` was generated on 2026-10-17 and the plan dir has
   newer files (`rome.md`, `touch-and-pov.md`, `final-picture.md`,
   `build-cpu.md`), so the atlas is behind. Not owned by this job.
8. `node tools/workspace-check.mjs` exits 1 today on crate facts (an
   undocumented `engine-arena -> engine-nav` edge, 16 files not declared by
   any module, float math in `anim/src/pose_c1_test.rs`, three unlisted
   genre assumptions). None of these are doc rot and none are in the eight
   owned files.
9. `node tools/doctor.mjs` reports the same workspace-audit failure, and
   "114 uncommitted paths (commit your own paths)" - that count includes
   other sessions' work.
10. `docs/plan/frontier.md` calls its pillars "five" and lists six
    (`docs/plan/frontier.md:112` vs bullets at 141-146). Not an owned file;
    the `docs/INDEX.md` row for it is a summary and was left as it is.

## One race to watch

While this job ran, another session rewrote `docs/INDEX.md` and dropped two
of the edits made before that write (the two size numbers). They were
re-applied after re-reading the file, and the final diff above was taken
from the tree after that. Re-read `docs/INDEX.md` before committing it.
