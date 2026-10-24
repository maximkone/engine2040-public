---
name: debug-triage
description: "Use when something is broken and you do not know why: a failing test, a panic or crash stack, several linked errors, or a bug to report. Root cause before fix, three strikes stop, primary versus cascade errors, report fields."
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

Words that mean stop: "quick fix", "just try", "probably X", "one more attempt", "I do not fully understand but". Each one sends you back to phase 1.
Two changes in one rerun is not an experiment. A third patch on the same cause needs a design question, not patch four.

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

Facts, not opinions or fixes. The field table for a `docs/BLOCKERS.md` row or a bug row is in `reference.md`. Do not write "must fix" in the report.
