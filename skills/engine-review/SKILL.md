---
name: engine-review
description: Use when reviewing engine2040 work - fast CHECK pre-check, REVIEW depth and evidence bar, Eyes on captures, API audit. Use for REVIEW/CHECK/API-REVIEW (evidence and captures; rust scoring stays in engine-rust-quality, never double-gate).
---

# Engine review

## CHECK pre-check (fast, literal, one change)

CHECK runs only for large (>3 files or >200 added lines) or multi-crate
results; small results skip it for a single REVIEW. Check only these,
with path:line:

1. Every claimed changed file exists in the patch; nothing outside owned paths.
2. The acceptance commands in the report have real output (numbers, test
   counts), not "should pass" or "done".
3. The new code has a caller outside tests (`git grep` the new public
   names). Zero callers outside tests = `CHECK: FAIL - <reason>` (CT7/PV4 class),
   unless the report names a follow-up consumer row already on the board.
4. No `Unsupported`, `GpuUnsupported`, `todo!`, `unimplemented!`, empty
   handlers, placeholder returns or TODO without a board ID in the new lines.
5. Render/GPU rows: the submit path records real passes, not a refusal.
6. Rust quality: owned by `engine-rust-quality` — require its `rust-quality`
   RESULT PASS and RUST 3+/5 before judging depth; its rules are not
   repeated here.

Reply `CHECK: PASS` or `CHECK: FAIL - <reason>`, then at most five findings with
path:line. Every FAIL carries its reason on the same line; a bare FAIL with no reason is rejected. Never judge design or taste in a CHECK.

## REVIEW

Follow `AGENTS.md`, `CODELAW` and `HASH-LAW`. Check ownership, API/error
compatibility, unwired code, dependency direction, allocations in fixed
ticks, test weakening and golden drift. Separate
proposed, compiled, tested, integrated, GPU-run and visually reviewed.
F2P = fails before the change and passes after; P2P = passes before and still passes after. Name both lists in a REVIEW.

- Depth (owner 2026-09-25): score 0-5 (0 slice, 3 finished and wired, 5
  best in class with edges, scale and docs); check the row's
  best-in-class claim. Depth under 3 is FAIL - <reason>, whatever the tests say.
- Render/GPU rows (operator 2026-09-24): open the owned module and read
  its submit path. If it can still return `Unsupported`,
  `GpuUnsupported` or any typed refusal on a real device, the verdict is
  FAIL - <reason>, whatever the tests, llvmpipe parity or CPU reference say. Name
  the file:line.
- Eyes: for any visual, render or content row, open
  the capture PNGs with the Read tool and write one line on what
  is really on screen before the verdict. Black, empty, magenta,
  NaN-speckled or T-posed frames are FAIL - <reason>. A visual claim with no capture
  you opened is unverified.

Return VERDICT: PASS | FAIL - <reason> | BLOCKED - <reason>, findings ordered by severity with
path:line and repro, missing gates, and the smallest fix. Every FAIL needs its reason on the same line; a bare FAIL is rejected.

## API-REVIEW

Follow `AGENTS.md`, `docs/CODELAW.md` and `docs/ARCHITECTURE.md`. Review
one crate (or one MCP tool family) like a paying user would, with
path:line:

1. **Rust API Guidelines**: naming, conversions, `Debug`/`Clone`/`Default`
   where expected, no leaking internal types, typed errors over strings
   or panics, builders for wide config.
2. **Units and ranges** in names or types; no bare `f32` that could be two
   units.
3. **Consistency** with sibling crates: same verbs (`spawn`, `step`,
   `snapshot`), same error shape, same handle types. List every mismatch.
4. **Dependency direction** from `docs/ARCHITECTURE.md`; nothing
   genre-specific leaking into a general crate (PLN18).
5. **Agent-friendliness:** every capability an agent needs is reachable by
   an MCP tool with a typed schema, a clear refusal and a dry-run; names
   searchable (API1 index).
6. **Semver risk:** what a change would break; run `cargo semver-checks`
   or `cargo public-api` when installed, else say not run.
7. **Docs:** public items without docs or examples (add a DOCS row).
8. **Pragmatic Rust Guidelines** (microsoft/rust-guidelines): cite the
   guideline ID per finding (M-PUBLIC-DEBUG, M-ERRORS-CANONICAL-STRUCTS,
   M-STRONG-TYPES, M-INIT-BUILDER, M-AVOID-WRAPPERS, M-PANIC-ON-BUG,
   M-HOTPATH, M-MEM-REUSE, M-SINGLE-ITEM-PATH, M-DESIGN-FOR-AI,
   M-TAUTOLOGICAL-TESTS).
9. **Scorecard:** start from `node tools/rust-quality.mjs --crate <crate>`
   and `--list <metric>`; a row proposal names the metric it lowers and
   by how much.

Return a score 0-10 per check, the ten highest-value fixes as board row
proposals (area, owned file, acceptance, needs) and what you could not
check.
