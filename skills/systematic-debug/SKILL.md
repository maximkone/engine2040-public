---
name: systematic-debug
description: Use when facing any bug, test failure, unexpected behavior, perf dip, or build/integration break, before proposing fixes. Root-cause-first gate with evidence.
---

# Systematic Debug

Iron law: NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST. Finish Phase 1 before any fix. Every step is read-only until Phase 4.

## Phase 1: Root Cause Investigation

1. Read errors fully. Copy the full message and the failing file:line. Never paraphrase from memory.
2. Reproduce consistently. Record the smallest repeatable steps with seeds and inputs. Note what changes the symptom.
3. Scan recent changes. Check the diff, recent commits, and dep or env changes since the last green run.
4. Instrument boundaries. At each component layer log in and out data, config, and state. Run once and name the failing layer.
5. Trace data flow to the source. Walk backward from the symptom to its origin. Fix at the source, never at the symptom.

## Phase 2: Pattern Analysis

Find one working example that should behave the same. Read its reference fully. List every difference between the working and broken cases: inputs, paths, deps, and assumptions.

## Phase 3: Hypothesis and Smallest Change

Write one single hypothesis that names the failing component and file:line. Change one variable only, smallest change first. Verify the result. A failed check means a new hypothesis, never a second change on the old one.

## Phase 4: Implementation With Test Gate

Write the failing test first (see test-driven-development), then apply the single fix. Verify with fresh evidence (see verification-before-completion): rerun the failing test plus the nearest suite. Keep shell calls pwsh-safe and read-only except the one fix.

3-strikes stop: after 3 failed fixes, stop and question the architecture. No fourth fix without a new design question.

Report line: component plus file:line plus evidence plus hypothesis plus fix plus fresh proof.
