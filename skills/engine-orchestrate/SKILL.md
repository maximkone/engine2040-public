---
name: engine-orchestrate
description: Use when planning, dispatching, reviewing or verifying engine2040 work - tasks, packets, diffs, CI, replay, hash, perf, roadmap decisions.
---

# Plan, dispatch, verify

Read the board row and the code it names. Check code and entrypoints before you accept a status.

## Choose the work

1. A task is ready when it is open, its prerequisites are ticked, its text has no `BLOCKED:` or `LATER:`, and it is not an operator task. Take ready tasks up to the wave width in `.opencode/knobs.json`, one builder per crate at a time.
2. Name each task's consumer: the binary, editor, fixture or test that calls the new code. Unwired code does not tick a task.
3. Ideas in `docs/RESEARCH-VERDICT.md` marked LATER or DROP need the owner's explicit request.

## Dispatch

| Packets | Seat | Owns |
|---|---|---|
| CODE, MAINT, DOCS, CONTENT | builder | named owned files; one crate per CODE packet |
| BENCH, PROVIDER | pilot or builder | bench or adapter files, paired numbers |
| LIVE, PLAY | pilot | runtime outputs under `target/` only |
| REVIEW, CHECK, API-REVIEW | judge | no writes; `VERDICT: PASS`, `FAIL - reason` or `BLOCKED - reason` |
| PLAN | planner | one `docs/plan/` file |
| RES-*, EXPLORE | researcher | one note or one board row |

- Send one message of Task calls per batch, each prompt `packet: <name>`. Packet shape: `skills/engine-orchestrate/packet-template.md`.
- Every packet names its owned paths, its proof command, its consumer and a Stop rule. Gate shared contracts and dependencies before dispatch.
- One writer per file. The board and the handoff have one writer: the lead. `target/queue/claims.txt` is append only, one line `<run id> | <UTC yyyy-mm-ddTHH:MMZ> | <item ID> <exact files>`.

## Review and integrate

- Review `git status --short -- <owned paths>` (read each `??` new file) and `git diff -- <owned paths>`, the diff piped to `node tools/packet-gate.mjs --patch - --owned <path>` (one `--owned` per path). A FAIL means no landing.
- Reject edits outside the owned paths, weakened tests, mirrored types and golden drift.
- A review PASS lands itself (`tools/land.mjs`); only a REFUSED land is yours (`engine-land`). Tick the row and write its receipt only after acceptance and review.
- Re-read the artifacts, not the summaries. Golden check once per wave: `cargo run -p engine2040 --release` (final_hash).

## Evidence rules

- Trace: command, then entrypoint, then code. GPU work needs a GPU run. A mock, a counter or a layout hash proves nothing about behaviour.
- Levels: proposed, compiled, unit, integrated, visual, measured. Report the level you reached.
- Determinism: same inputs give the same checkpoints. A covered change must diverge; report the first divergent tick.
- Perf: machine, profile, warm-up, N, p50/p95/p99. A speedup claim needs an A/B run.
- Unrun gates are unverified. Never invent a receipt. The golden hash `d57b5c8ae3096614` moves only with a receipt (`docs/HASH-LAW.md`).

## Failure

Diagnose and retry once with a concrete fix. The same failure twice goes to `docs/BLOCKERS.md` with owner, repro and output; continue independent work. Respect an operator halt.

Plan prefixes and their files live under `docs/plan/`.
