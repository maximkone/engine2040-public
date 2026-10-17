---
name: debug-triage
description: "Use when facing a bug, failing test, crash or panic stack, several linked errors at once, or a bug report to file: root cause before fix, 3-strikes stop, stack fingerprint, primary vs cascade clusters, report fields (steps, expected, actual, build, proof)."
---

# Debug triage

Iron law: NO FIX WITHOUT ROOT CAUSE FIRST. Finish phase 1 before any edit. Every step is read-only until phase 4.

## 1. Four phases

**Phase 1: investigate**
1. Read the error fully. Copy the whole message and the failing file and line. Never paraphrase from memory.
2. Reproduce it. Record the smallest repeatable steps with seed and inputs. Note what changes the symptom.
3. Scan recent changes: the diff, recent commits, dependency or env changes since the last green run.
4. Instrument the boundaries. At each layer log data, config and state in and out. Run once and name the failing layer.
5. Trace the data backward from the symptom to its origin. Fix at the source, never at the symptom.

**Phase 2: pattern.** Find one working example that should behave the same. Read it fully. List every difference: inputs, paths, deps, assumptions.

**Phase 3: hypothesis.** Write one hypothesis that names the failing component and where. Change one variable only, smallest change first. A failed check means a new hypothesis, never a second change on the old one.

**Phase 4: fix.** Write the failing test first (skill `test-and-proof`), then apply the single fix. Verify with fresh evidence: rerun the failing test plus the nearest suite. Keep shell calls pwsh-safe and read-only except the one fix.

**3-strikes stop.** After 3 failed fixes, stop and question the design. No fourth fix without a new design question.
A second identical failure ends the step: one row in `docs/BLOCKERS.md`, never green (AGENTS.md).

Report line: component, where, evidence, hypothesis, fix, fresh proof.

A hash or replay divergence has its own method: skill `determinism-debug`. A red gate or link error: skill `github-check`.

## 2. One variable per rerun

- Change exactly one of: fixture, build, capture flags, machine load. A new binary plus new flags plus a busy GPU sold as one result proves nothing.
- Name the layer in the report: replay, capture path, or perf. A report that blames all three and proves none is rejected.

## 3. Crash stacks

1. Copy the top 10 frames word for word with crate, file and line. Not a summary, not just the panic message.
2. Map each frame to one owning crate with the head table of `docs/CRATE-MAP.md`. Do not guess an owner. Do not blame the whole workspace.
3. Rule out the environment first: state build hash, seed, OS and device. Do not blame code while build or device is unknown.
4. Find the smallest command (seed, tick count) that repeats the crash. Give a repeat rate.
5. File one report per stack. It carries a fingerprint of the top 3 frames, the owner crate and the repro command. Never two reports for one fingerprint.
6. An env-only crash (no local frame) ends in a rerun on a clean build. Never propose a code revert for it.

## 4. Many errors at once

Use this when three or more errors arrive together. Do it before any fix.

1. Cluster by boundary: time, subsystem, recent-change surface. One cluster per cause, not per message text.
2. Split primary from cascade. A primary explains the rest. A cascade clears when its primary lands. Cite evidence beyond the message text: a source line, a log, a rebuild.
3. Order by impact times information gain. Fix what blocks most first. Break ties by what one fast test answers first.
4. Each unsure cluster gets one fast disproof test that kills it when wrong. Mark guesses "inferred", not "confirmed".
5. Hold cascades until the primary lands. Then rerun the proofs.
6. Edit only paths you own. A red in a file you do not own is its owner's.

## 5. Writing a bug report

A `docs/BLOCKERS.md` row or a bug row on the board carries these fields. Write facts, not opinions or fixes.

| field | rule |
|---|---|
| Title | one line: platform, area, symptom. Not "broken". |
| Row | the open board row id, if one exists |
| Failing command | the exact command with crate and filter. Not "run tests". |
| Build | build hash, date, OS and device. Not "latest". |
| Expected / actual | one line each |
| Steps | numbered, one action each, with preconditions |
| Proof | a capture or log plus repro rate x/5 (not 1/1) |
| Severity | by player impact. Leave priority to the owner. |

Do not write "must fix" in the report.
