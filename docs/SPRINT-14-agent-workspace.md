### Agent workspace (owner 2026-09-26, `docs/plan/seats.md`)

The OpenCode loop itself: smaller contexts, shell-safe research,
cross-family review, halt lifecycle, destructive-command and secret
guards, tests for every loop tool. Rows are orchestrator-landed unless
they name another lane; none touches `crates/`.

- [ ] AW1 tools: board slices — `tools/board-slice.mjs <ID>` prints the
  row verbatim, the first lines of its prerequisite rows, its plan §3/§7
  excerpt and owned paths; packet cards carry the slice. Tier A. Size S.
  Owns `tools/board-slice.mjs` plus a test. Accept: slices for 10 ready
  rows ≤ 8 KB each with the row byte-identical; median packet context
  over the next wave down ≥ 50% on the context meter. needs: none.
  Wave 57 progress (stays open): `tools/board-slice.mjs` collected
  (--self-check 10/10 PASS on master); committed test suite + context
  meter still open.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW2 docs: board split — receipts older than three waves move to
  `docs/receipts/YYYY-MM.md`, ticked rows older than ten waves to
  `docs/board-done.md`; `sprint-loop`, `finish`, `drift-check`,
  `workspace-check` and `packet-gate` resolve prerequisites across both.
  Tier A. Size M. Owns the split and the tool hunks. Accept: SPRINT.md
  ≤ 120 KB; `sprint-loop --dry-run` ready list and `finish.mjs` output
  identical before and after. needs: AW14.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW4 ops: cross-family review — judge and checker packets run on a
  model family different from the writer's when one is available; the
  retro reports overturns and escaped defects (found later by ADV or
  janitor in ticked rows) per pairing. Tier B. Size S. Owns agent model
  fields and the retro columns. Accept: two retros of numbers; the
  pairing that finds more real defects per million tokens stays, logged
  in DIRECTION-LOG. needs: AW12.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW5 tools: agent-file regression replay — 20 frozen historical
  packets (inputs plus the known verdict) replay against a changed agent
  file before it lands; the pass rate may not drop. Tier B. Size M. Owns
  `tools/agent-replay.mjs` and `fixtures/agent-replay/`. Accept: a seeded
  edit that makes the judge approve a known-bad patch fails the replay.
  needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW6 tools: flake triage — proof and lane logs fingerprinted (test,
  error class, file:line), rerun once, classified flaky or real, one
  BLOCKERS draft per new fingerprint; the `-j4` fingerprint gets a
  root-cause spike. Tier A. Size M. Owns `tools/triage.mjs`. Accept: the
  last 10 proof logs classified; the `-j4` cause named with a repro or
  quarantined with evidence. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW7 ops: halt lifecycle — on `target/sprint.halt` the orchestrator
  finishes the round, writes the handoff, releases the lock and pushes
  per operator policy; drift-check fails on a halt with a held lock over
  30 minutes or a handoff older than the last board commit. Tier A.
  Size S. Owns the sprint command lines and the drift hunk. Accept: a
  halt drill ends with a fresh handoff and a free lock; a seeded stale
  pair fails. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW8 ops: destructive-command guard — global deny patterns in
  `opencode.jsonc` for force pushes, hard resets, `git clean -fdx`,
  recursive deletes outside `target/` and `.lanes/`, and history
  rewrites; per-agent allow-lists stay narrower. Tier A. Size S. Owns the
  permission block. Accept: a drill agent's attempts are refused; one
  normal wave runs unaffected. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW9 tools: secret guard — `packet-gate` and the commit path scan
  staged diffs for key and token patterns and refuse; the handoff flags
  plaintext key files in the OpenCode config folder for the owner. Tier
  A. Size S. Owns the scanner and its test. Accept: a seeded key in a
  diff is refused; no false positive on the last 50 commits. needs: none.
- [ ] AW10 ops: push cadence — drift-check warns at 10 unpushed commits
  and fails at 20 or 24 hours; the wave end pushes per the operator
  policy (`safe` runs never push). Tier A. Size S. Owns the drift hunk.
  Accept: seeded counts trip WARN and FAIL. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW11 tools: loop tool tests — `node --test` suites for
  `sprint-loop`, `sprint-lock`, `lanes`, `proof` argument parsing,
  `drift-check`, `workspace-check`, `sync-twins` and `showcase`, each
  with a seeded-bug case; they run in proof. Tier A. Size M. Owns
  `tools/*.test.mjs`. Accept: every loop tool has a suite; each seeded
  bug fails. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW12 tools: context meter in the handoff — per wave: tokens in and
  out per agent class, tool-output share, compactions, ticks per million
  tokens and idle helper minutes, from the proxy meter and the center's
  metrics. Tier B. Size S. Owns the meter hunk in the handoff writer.
  Accept: three consecutive handoffs carry the numbers. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW13 skills: five focused skills — `engine-render` (GPU pass live
  gate, GX6), `engine-perf` (measure, change, measure; PF11
  append-only), `engine-content` (license read, provenance row, cook),
  `engine-plan` (template plus row validator), `engine-research`
  (shell-safe search, card format, spike rule). Tier B. Size M. Owns
  `skills/engine-*/SKILL.md`. Accept: each loads in `opencode debug
  skill`; the retro shows first-try packet-gate passes up for packets
  that used one. needs: AW14.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW14 tools: plan and board check — `tools/plan-check.mjs`
  compares every plan's §7 rows with the board (byte-equal once
  boarded), resolves prerequisites, checks ID uniqueness and that §9
  sizes add up; drift-check runs it. Tier A. Size S. Owns the tool and
  its test. Accept: a seeded mismatch, a dangling need and a duplicate
  ID each fail; today's plans pass. needs: none.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW15 ops: model routing by measured yield — packet classes
  (S writer, L writer, research, judge, docs) map to models by ticks per
  million tokens and escaped defects from AW12 and AW4; knobs set through
  the center. Tier C. Size S. Owns the routing table. Accept: two retros
  show ≥ 15% more ticks per million tokens at no higher escape rate, or
  the change reverts. needs: AW4, AW12.
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW16 tools: span-attributed tool-call ledger for the loop's helpers
  (trace ids on every tool/MCP call, tokens per span; tracing shape,
  MIT) (READER-WILD wave 51). Tier B. Size S. Accept: 100% tokens
  attributed on a frozen session, sim hashes x2, overhead p50 ≤ +1%.
  needs: AW12. (WILD.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW17 tools: ignore-aware parallel tree walk for loop scanners
  (ripgrep walk/ignore shape, MIT) (READER-WILD wave 51). Tier B.
  Size S. Accept: identical file sets 3/3, wall p50 halved. needs:
  none. (WILD.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW18 tools: union board loader. Tier B. Size S. Owns new
  `tools/board.mjs` (`loadBoard`: `docs/SPRINT.md` split on the Receipts
  header + `docs/sprint/later.md` + `docs/sprint/done-*.md`, one row regex, one
  `needs:` parser, `source` tag `active`/`later`/`done`) +
  `tools/board.test.mjs`. Accept: union counts equal today's board;
  every `needs:` resolves via the union; no id twice across the 3 files;
  never unticks. needs: none. (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW19 tools: one-shot splitter (check-only). Tier B. Size M. Owns
  new one-shot `tools/board-split.mjs` (`--check` prints per-section
  done/ticked/open-active/open-deferred + bytes, writes nothing; `--write`
  keeps same header/`## Board`/`###` titles + byte-identical active rows
  + Done one-liner stubs). Accept: `--check` prints union rows +
  per-section bytes on the un-split tree; this row never writes. needs:
  AW18. (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW20 tools: loader adopt, gate pair. Tier B. Size S. Owns hunks in
  `tools/workspace-check.mjs` + `tools/sprint-loop.mjs`. Accept:
  `workspace-check` PASS + `sprint-loop --dry-run` ready list identical
  before/after on the un-split tree. needs: AW18.
  (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW21 tools: loader adopt, slice pair. Tier B. Size S. Owns hunks in
  `tools/board-slice.mjs` + `tools/finish.mjs`. Accept: `board-slice
  --self-check` + `finish.mjs` output identical before/after. needs:
  AW18. (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW22 tools: loader adopt, loop triple. Tier B. Size S. Owns hunks in
  `tools/drift-check.mjs` + `tools/ladder-sync.mjs` +
  `tools/rust-quality.mjs`. Accept: board sections, ladder-sync and
  rust-quality ids identical before/after. needs: AW18.
  (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
- [ ] AW23 ops: land the three-file split. Tier B. Size S. Owns the split
  commit only (`docs/SPRINT.md` active open rows only; `docs/sprint/later.md`
  deferred open rows full-text never ready; `docs/sprint/done-2026-10.md`
  all ticked rows verbatim, then monthly). Accept: `workspace-check` PASS
  before and after; `sprint-loop --dry-run` PASS before and after; union
  before == after; `docs/SPRINT.md` under 120 KB. needs: AW19, AW20, AW21,
  AW22. (audit-board-split 2026-10-03.)
  LATER: parked 2026-10-03 (circus verdict: loop-about-the-loop work, not a graphics/animation/ragdoll/AI-door bar (center runs the loop)); unpark by deleting this line.
