---
name: review
description: "Use when you review a diff or audit an area: depth 0-5 score, rust-quality bar, a pub item with no caller, a GPU test that skipped, captures you must open, API review, dead-code audit with ranked findings."
---

# Review

Review a diff you did not write. For your own diff run section 1; add section 2 for Rust. For docs or agent instructions run the checks in `AGENTS.md` instead of the Rust bar.
Verdict form: `PASS | FAIL - <reason> | BLOCKED - <reason>`. Every FAIL carries its reason on the same line. A bare FAIL is rejected.
Rules: `AGENTS.md`, `docs/CODELAW.md`, `docs/HASH-LAW.md`, `docs/ARCHITECTURE.md`.
Detail: `reference.md` (API review, area audit, integrated-tree gate, full Rust bar).

## 1. Quick pre-check (big or multi-crate diffs; small ones go straight to section 2)

Large means more than 3 files or more than 200 added lines, or more than one crate. Check only these, each with path and line:

1. Every file the report claims changed exists in the diff. Nothing outside the row's `Owns` paths.
2. Acceptance commands have real output (numbers, test counts), not "should pass" (skill `test-and-proof`).
3. Every new `pub` item has a production caller: `node tools/wiring-scan.mjs crates --diff`.
4. No `Unsupported`, `GpuUnsupported`, `todo!`, `unimplemented!`, empty handler, placeholder return, or TODO without a board id.
5. Render and GPU rows: the submit path records real passes, not a refusal.
6. Rust bar: section 2 passes before you score depth.

Reply `CHECK: PASS` or `CHECK: FAIL - <reason>`, then at most five findings with path and line. Do not rule on design or taste in a CHECK.

## 2. Rust quality bar

- `node tools/rust-quality.mjs --crate <crate>` must print `RESULT PASS` with 0 ratchet regressions; run it targeted, never workspace-wide. A `rust-*` FAIL or ratchet regression is FAIL before you read further (`--no-cache` forces a fresh answer). To fix or accept one: skill `rust-quality-ratchet`.
- Check by hand what the tool cannot see: `unwrap`, print in libraries, `allow`, `dbg!`, nondeterminism in sim, copies of probe helpers, board ids in comments, string errors, hot-path allocation, a fn over 100 lines, a file over 1,500 lines. New casts are warnings (skill `rust-safety`); rules in `docs/RUST-QUALITY.md`.
- Never a new `#[allow(`. The exception is `#[expect(lint, reason = "...")]` you would defend.
- Score 0 to 5 for what tools cannot see: ownership without needless clones, layout fit for the hot path, typed errors, the smallest `pub` surface, invariants in comments, tests that fail on a plausible bug. Name the file per deduction; under 3 is FAIL. Full list: `reference.md`.
- Report `RUST: <n>/5` beside `DEPTH: <n>/5`. The writer-side checklist is skill `engine-code`; this is the review-side bar.

## 3. Review (depth)

Check: ownership, API and error compatibility, unwired code, dependency direction, fixed-tick allocations, test weakening, golden drift.

Keep the levels apart in the verdict: proposed, compiled, unit-tested, integrated, GPU-run, visually reviewed. Name both lists: F2P (fails before, passes after) and P2P (passes before and after).

- Depth score 0 to 5. 0 is a slice. 3 is finished and wired. 5 is best in class with edges, scale and docs. Check the row's best-in-class statement. Depth under 3 is `FAIL - <reason>`, whatever the tests say.
- Render and GPU rows: read the owned module's submit path. One that can still return `Unsupported`, `GpuUnsupported` or any typed refusal on a real device is `FAIL - <reason>`; tests, llvmpipe parity and a CPU reference do not change that. Name the file.
- Eyes: for any visual or content row follow the Eyes rule in skill `test-and-proof`.

Return: `VERDICT: ...`, findings ordered by severity with path, line and repro, the missing gates, and the smallest fix.

API review of one crate or MCP tool family (guideline ids, semver, agent-friendliness, scorecard), the read-only audit of one coverage area with at most five ranked findings, and the gate on the integrated tree with owned-path repairs are all in `reference.md`.
