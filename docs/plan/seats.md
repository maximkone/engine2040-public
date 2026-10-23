# Seats

Goal: every seat lands output or ends NOOP with proof.
Acceptance:
1. Every seat ends `RESULT:` with a proof line.
2. `node tools/queue-cap.mjs --check` passes for every packet.
3. `node tools/seat-ledger.mjs` prints DONE and NOOP per seat role.
Order: AW9, AW1, AW5, AW6, AW7, AW8, AW10, AW11, AW12, AW14, AW17, AW18, AW2, AW13, AW4, AW16, AW19, AW20, AW21, AW22, AW15, AW23, AW24, AW25, AW26, AW27, AW28, AW29
Owner crates: tools/, .opencode/.
Open board IDs: AW1, AW2, AW4, AW5, AW6, AW7, AW8, AW9, AW10, AW11, AW12, AW13, AW14, AW15, AW16, AW17, AW18, AW19, AW20, AW21, AW22, AW23, AW24, AW25, AW26, AW27, AW28, AW29
Pre-merge text: git show d96091cd:docs/plan/seats.md
Pre-merge text: git show d96091cd:docs/plan/agent-workspace.md

## 1. Where it stands

Seat counts are knobs (`node tools/queue-cap.mjs --mix`). What a seat does is in its own file in `target/queue/standing/`; `.opencode/seat-guard.mjs` keeps it inside its paths. Role files: `builder` (coder), `judge`, `planner`, `smith`, `runner`, `perf`, `porter`, `miner`, `skill-hunter`, `rot-hunter`, `walker`; `pilot` and `researcher` serve the lead's one-off packets. A seat with nothing to do ends `RESULT: NOOP - <why> | proof: ...` and writes no filler.

### agent-workspace (AW)

The loop works and ships waves; its costs and failure modes are mostly unmeasured or guarded only by words in agent files. Survey of 2026-09-26; numbers not refreshed.

| Gap | Open row |
|---|---|
| The board is too big; each agent that reads it swallows most of it | AW1, AW2 |
| Lead context grows with old tool output | AW12 |
| Helpers mostly idle; judges and writers share one model family | AW4, AW15, AW16 |
| A halt can leave a stale lock and no handoff | AW7 |
| Local commits pile up unpushed | AW10 |
| Most loop tools have no `node --test` suite | AW11 |
| Few skills; the book and loop skills are not in this repo | AW13 |
| Plan and board are checked ad hoc | AW14 |
| The same flake fingerprint recurs wave after wave | AW6 |
| Agent-file edits regress silently; only words guard them | AW5 |
| Global permission is allow-all; no deny list for destructive commands | AW8 |
| Plaintext keys sit in the OpenCode config folder | AW9 |

## 2. Finish line and size

The count of each seat is its knob. The limit is the cargo slots (`cargo_slots`, reason in `.opencode/knobs.json`) and one crate per coder: more coders only wait. Landing is one commit at a time by the keeper (`tools/land.mjs`) after a judge PASS. Every seat lands something or ends NOOP with proof.

### agent-workspace (AW)

Bars: board <= 120 KB; packet context median down 50%; 0 junk files for 10 waves; 0 halts without handoff; >= 1 test per loop tool; escaped defects per 100 ticks trending down over two retros.

## 3. Target design and ownership

Planned and underused, now owned (no numbers; counts were not checked today):

| what | where | seat |
|---|---|---|
| open steal lines, some already in code; `steal-land --gate` fails today | `docs/plan/steals.md` | porter lands lines, rot-hunter closes done ones |
| rot scans: stubs, unwired modules, leftovers, bad doc refs | `target/rot/` | rot-hunter |
| lane patches worth harvesting | `target/rot/lanes-game.csv` | rot-hunter, coder |
| book skills and loop skills in skillworks, none in this repo | `C:/empire/lab/skillworks/skills`, `work/b1-*` | skill-hunter, miner |
| paused seats: sentinel, bench, janitor, evolve, future, vision | `center/crews/engine2040/standing-paused/` | folded into the other seats |
| idle tools: `affected` `bench-compare` `desync-diff` `frame-triage` `mp-evidence` `part-score` `plan-atlas` `research-cap` `steal-land` `steal-next` `showcase-gate` `q1-check` `secret-gate` `flaky-quarantine` | `tools/` | judge, perf, runner, walker, planner, porter |
| Lead 2 journeys J1 to J5, walked by one seat | `center/crews/engine2040/lead2/journeys.md` | walker |

### agent-workspace (AW)

Plain version: agents read small, exact slices; tools do the fragile shell work; every loop rule that matters is checked by code.

- Board slices (AW1): `tools/board-slice.mjs <ID>` prints one row verbatim with its `needs:` rows, its plan excerpt and its owned paths. The tool exists; AW1 stays open for its acceptance.
- Board split (AW2, AW19, AW23): receipts older than three waves move to `docs/receipts/YYYY-MM.md`; ticked rows older than ten waves move to `docs/board-done.md`. Every tool resolves `needs:` across both files, so readiness never changes.
- Review diversity (AW4): the writer's model family never judges its own packet when another family is available; the retro measures it.
- Lifecycle (AW7, AW10): halt, then finish round, handoff, release lock, push per operator policy; drift-check fails any break in that chain.
- Checks as code (AW11, AW14): plan/board equality, `needs:`, IDs, size sums, junk files, stale locks and secrets in diffs are tool checks.
- Agent-file changes (AW5): replay 20 frozen historical packets before a level-2 change lands.
- Destructive commands (AW8): global deny patterns for destructive git and file commands, because `question` is denied and nobody can say no at night.

## 4. Hard problems

Books: 14 titles in `C:/Users/me/Downloads/BOOKS` feed the miner seats; the title-to-skill map is in the pre-merge text.

Land audit (2026-10-13, `tools/land.mjs` + `tools/hooks/pre-commit` + `tools/land.test.mjs`): three land REFUSED fingerprints repeat on the board (`docs/BLOCKERS.md`: cut-relay-testpin x7 ratchet-baseline-unstaged, smith-janitor-r3 x3 staged-deletion pathspec, loop-sp63-a2-repair x10 ratchet-baseline-unstaged), and the BOOK-GAPS 2026-10-14 note says the quality.md:38 landing-gate sentence no longer matches the land.mjs workspace audit. Technique: port the failure-body shape (refusal prints `fingerprint:` + `remedy:` cells) and the stage-before-commit shape (foreign staged hunks under the packet's paths refuse by name). Donors (pinned, paths verified live via the GitHub contents API at the commit, no bytes vendored): `nodejs/node@c96226ecf1a1f1f5161732ccdd35cdb5826f30ec` `tools/actions/commit-queue.sh` (commit_queue_failed posts the failure body and labels the PR, AW28 takes the shape); `bors-ng/bors-ng@ca725797e53a88e954998de0bbb14a8a5acb13ab` `FLOW_EXAMPLE.md` (Batcher merges to `staging`, waits for CI, then fast-forwards master, AW29 takes the shape). Porting policy is AGENTS rule 4.
Status 2026-10-21: this whole plan is history, not a worklist - the loop it plans for was removed on 2026-10-17 (commit `6bdf32bf` "cut the center link, the keeper, the queue tools and 13 of 14 agents"). `tools/land.mjs`, `tools/land.test.mjs`, `tools/queue-cap.mjs`, `tools/seat-ledger.mjs` and `tools/packet-gate.mjs` no longer exist in this repo (only `tools/hooks/pre-commit` does), and `docs/README.md:31` lists this file as a known leftover to "read as history". The AW24-AW29 land-audit rows are on the board in `docs/archive/SPRINT-done-14-agent-workspace.md` (moved 2026-10-23); nothing here is runnable.

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4.

## 6. Rules that cut across seats

Write seats (smith, miner, skill-hunter, planner) never commit (only the lead commits; `tools/land.mjs` commits reviewed work); they end with RESULT and paths, a judge reads `git diff`, and the keeper lands by path after review. Research seats (porter, rot-hunter, walker) write only `target/queue/` and `target/reports/`. At the queue cap (`node tools/queue-cap.mjs` says VERIFY) a research seat checks a queued packet instead of adding one. Every packet passes `node tools/queue-cap.mjs --check <file>`. Runners and perf need a fresh build: the lead builds once a wave (`cargo build --release --bins`); a binary older than HEAD is reported with the commits since. A shell write counts like an edit: `>`, `Set-Content`, `Copy-Item`, `Remove-Item`, `node -e` writeFileSync, `git rm` and the rest are checked against the same paths (research, run and judge seats: `target/` only; smith and coder seats: packet paths). A write with a path the guard cannot read is blocked for research and run seats. `git apply` and `git checkout` are blocked for every seat; a coder may only `git restore --source=HEAD -- <its packet paths>`.

## 7. Board tasks

Rows live in `docs/archive/SPRINT-done-14-agent-workspace.md` (moved 2026-10-23); ticked rows are dropped. `node tools/board-slice.mjs --ready --open` lists them. Full text of the archived plan: `docs/archive/2026-10-10/plans/agent-workspace.md`.

### agent-workspace (AW)

- [ ] AW9 tools: secret guard. `packet-gate` and the commit path scan staged diffs for key and token patterns and refuse; the handoff flags plaintext key files. Tier A. Size S.
  Accept: a seeded key in a diff is refused; no false positive on the last 50 commits.

| ID | title (12 words) | Tier | needs |
|---|---|---|---|
| AW1 | board slices | A | none |
| AW2 | board split | A | AW14 |
| AW4 | cross-family review | B | AW12 |
| AW5 | agent-file regression replay | B | none |
| AW6 | flake triage | A | none |
| AW7 | halt lifecycle | A | none |
| AW8 | destructive-command guard | A | none |
| AW10 | push cadence | A | none |
| AW11 | loop tool tests | A | none |
| AW12 | context meter in the handoff | B | none |
| AW13 | five focused skills | B | AW14 |
| AW14 | plan and board check | A | none |
| AW15 | model routing by measured yield | C | AW4, AW12 |
| AW16 | span-attributed tool-call ledger for the loop's helpers | B | AW12 |
| AW17 | ignore-aware parallel tree walk for loop scanners | B | none; LATER: park |
| AW18 | union board loader | B | none |
| AW19 | one-shot splitter (check-only) | B | AW18 |
| AW20 | loader adopt, gate pair | B | AW18 |
| AW21 | loader adopt, slice pair | B | AW18 |
| AW22 | loader adopt, loop triple | B | AW18 |
| AW23 | land the three-file split | B | AW19, AW20, AW21, AW22 |
| AW24 | staged-deletion landing by path | A | none |
| AW25 | ratchet-baseline rider on land refusal | A | AW24 |
| AW26 | workspace-check shape census for the land audit | A | AW25 |
| AW27 | land header and landing sentence match the audit | B | AW26 |
| AW28 | structured land refusal with fingerprint and remedy | A | AW27 |
| AW29 | foreign-staged guard before land commits | B | AW28 |

### land audit (AW24-AW29, 2026-10-13)

One file refuses every landing for one stray shape, so each row below pins one shape with a seeded test in `tools/land.test.mjs`. Rows land in needs order (all touch `tools/land.mjs`).

- [ ] AW24 tools: staged-deletion landing by path. Tier A. Size S.
  Owns the commit hunk in `tools/land.mjs` plus its test. Accept: a seeded
  packet whose only change is a staged deletion (file gone from disk and
  index, `git rm` staged) lands with `node tools/land.mjs <packet>` exit 0
  and `node --test tools/land.test.mjs` green; the smith-janitor-r3 pathspec
  fingerprint never recurs. needs: none.
- [ ] AW25 tools: ratchet-baseline rider on land refusal. Tier A. Size S.
  Owns the refusal text in `tools/land.mjs` beside `tools/hooks/pre-commit`.
  Accept: a seeded packet whose staged crates regress the ratchet is refused
  with `node tools/land.mjs <packet>` exit 1 naming the exact
  `node tools/rust-quality.mjs --accept <crate> --reason "<why>"` rider; the
  cut-relay-testpin and loop-sp63-a2-repair fingerprints print the rider, not
  a bare commit failure; `node --test tools/land.test.mjs` green. needs:
  AW24.
- [ ] AW26 tools: workspace-check shape census for the land audit. Tier A.
  Size S. Owns the sync test in `tools/land.test.mjs`. Accept: the census
  enumerates every `FAIL` line shape `tools/workspace-check.mjs` prints and
  asserts `failFiles` names its files, so a new shape fails the test instead
  of mis-landing or mis-warning; `node --test tools/land.test.mjs` green.
  needs: AW25.
- [ ] AW27 docs: land header and landing sentence match the audit. Tier B.
  Size S. Owns the header comment in `tools/land.mjs` and §6 of this plan.
  Accept: the header cites the workspace-audit and RESULT-FAIL refusal lines
  by number and matches disk, this section's landing sentence matches them,
  `node tools/plan-atlas.mjs --plan seats` clean, and the BOOK-GAPS stale-half
  note is closed. needs: AW26.
- [ ] AW28 tools: structured land refusal with fingerprint and remedy. Tier A.
  Size S. Owns the refusal lines in `tools/land.mjs` plus its test (donor shape:
  `nodejs/node@c96226ecf1a1f1f5161732ccdd35cdb5826f30ec`
  `tools/actions/commit-queue.sh` failure body). Accept: every
  `REFUSED <packet>:` line prints `fingerprint:` plus `remedy:` cells a second
  identical failure can file directly to `docs/BLOCKERS.md`;
  `node --test tools/land.test.mjs` green. needs: AW27.
- [ ] AW29 tools: foreign-staged guard before land commits. Tier B. Size S.
  Owns the staged-foreign check in `tools/land.mjs` plus its test (donor shape:
  `bors-ng/bors-ng@ca725797e53a88e954998de0bbb14a8a5acb13ab`
  `FLOW_EXAMPLE.md` staging branch). Accept: a seeded packet whose owned paths
  carry another writer's staged hunks is refused naming the files before any
  commit; `node --test tools/land.test.mjs` green. needs: AW28.

Row changes: none (AW1-AW23 untouched; AW24-AW29 are new, numbered after the highest board ID AW23).

## 8. Quality gates

Seat gates are the Acceptance list in the header.

### agent-workspace (AW)

- drift-check runs AW3, AW7, AW10 and AW14 checks every start and every five waves; FAIL is the first packet of the next wave.
- Every loop tool change lands with its test (AW11) green.
- AW2 never changes readiness: dry-run output compared before and after in the same receipt.
- Metrics rows (AW4, AW12, AW15) keep numbers in the handoff; a change without numbers is not a change.

## 9. Session estimate

Not kept here. The AW rows are S or M and mostly orchestrator-landed, so batch three or four per wave beside code lanes; none touches `crates/`.
