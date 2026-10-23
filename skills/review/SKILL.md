---
name: review
description: "Use when you review a diff or audit an area: depth 0-5 score, rust-quality bar, a pub item with no caller, a GPU test that skipped, captures you must open, API review, dead-code audit with ranked findings."
---

# Review

Review a diff you did not write. For your own diff, run section 1; add section 3 for Rust. For docs or agent instructions, run the checks named in `AGENTS.md` instead of the Rust bar.
Verdict form: `PASS | FAIL - <reason> | BLOCKED - <reason>`. Every FAIL carries its reason on the same line. A bare FAIL is rejected.
Rules: `AGENTS.md`, `docs/CODELAW.md`, `docs/HASH-LAW.md`, `docs/ARCHITECTURE.md`.

## 1. Quick pre-check (big or multi-crate diffs; small ones go straight to section 2)

Large means more than 3 files or more than 200 added lines, or more than one crate. Check only these, each with path and line:

1. Every file the report claims changed exists in the diff. Nothing outside the row's `Owns` paths.
2. The acceptance commands in the report have real output (numbers, test counts), not "should pass" or "done" (skill `test-and-proof`).
3. Every new `pub` item has a production caller. See the caller rule below.
4. No `Unsupported`, `GpuUnsupported`, `todo!`, `unimplemented!`, empty handler, placeholder return, or TODO without a board id in the new lines.
5. Render and GPU rows: the submit path records real passes, not a refusal.
6. Rust bar: section 3 passes before you score depth.

Reply `CHECK: PASS` or `CHECK: FAIL - <reason>`, then at most five findings with path and line. Do not rule on design or taste in a CHECK.

### The caller rule

Run `node tools/wiring-scan.mjs crates --diff`; the rule lives in `skills/wiring-a-feature`.

## 2. Review (depth)

Check: ownership, API and error compatibility, unwired code, dependency direction, allocations in fixed ticks, test weakening, golden drift (skill `determinism-debug`).

Keep these levels apart in the verdict: proposed, compiled, unit-tested, integrated, GPU-run, visually reviewed.
Name both lists: F2P (fails before the change, passes after) and P2P (passes before and still passes after).

- Depth score 0 to 5. 0 is a slice. 3 is finished and wired. 5 is best in class with edges, scale and docs. Check the row's best-in-class statement. Depth under 3 is `FAIL - <reason>`, whatever the tests say.
- Render and GPU rows: open the owned module and read its submit path. If it can still return `Unsupported`, `GpuUnsupported` or any typed refusal on a real device, the verdict is `FAIL - <reason>`. Tests, llvmpipe parity and a CPU reference do not change that. Name the file.
- Eyes: for any visual, render or content row, follow the Eyes rule in skill `test-and-proof`.

Return: `VERDICT: ...`, findings ordered by severity with path, line and repro, the missing gates, and the smallest fix.

## 3. Rust quality bar

- `node tools/rust-quality.mjs --crate <crate>` must print `RESULT PASS` with 0 ratchet regressions. Run it targeted, never as a whole-workspace suite. A `rust-*` FAIL or a ratchet regression is FAIL before you read further. Use `--no-cache` when you need a fresh answer (skill `github-check`). A FAIL to fix or accept: skill `rust-quality-ratchet`. A new pub item with no caller: skill `wiring-a-feature`.
- The old per-change gate script is retired. Check these rules by hand on the added lines: `unwrap`, print in libraries, `allow`, `expect` without a reason, `dbg!`, nondeterminism in sim, copies of Fnv or probe helpers, board ids in comments, string errors, allocation in hot fns, a new fn over 100 lines, a new file over 1,500 lines. New casts and growth are warnings (skill `rust-safety`). Rules live in `docs/RUST-QUALITY.md`.
- Never a new `#[allow(`. The exception is `#[expect(lint, reason = "...")]` you would defend.
- Read the diff for what tools cannot see, then score 0 to 5. 0 would not compile in review. 3 is idiomatic and owned. 5 is what a top engine team would copy.
  - ownership and borrowing without needless clones;
  - data layout fit for the hot path (SoA, slices, reused buffers);
  - typed, specific errors; explicit conversions; the smallest `pub` surface; names with units;
  - short single-purpose functions;
  - no re-derived helper an owner already exports: see skill `engine-code` section 3;
  - comments that state invariants, not history (no board ids or "was X");
  - tests that would fail on a plausible bug (oracle, negative case, property over seeds).
- Name the file for each deduction. A Rust score under 3 is FAIL.
- Report `RUST: <n>/5` beside `DEPTH: <n>/5`.
- The writer-side checklist lives in skill `engine-code`. This is the review-side bar.

## 4. API review (one crate or one MCP tool family)

Review like a paying user would, with path and line:

1. Rust API Guidelines: naming, conversions, `Debug`, `Clone`, `Default` where expected, no leaking internal types, typed errors over strings or panics, builders for wide config.
2. Units and ranges in names or types. No bare `f32` that could be two units.
3. Consistency with sibling crates: same verbs (`spawn`, `step`, `snapshot`), same error shape, same handle types. List every mismatch.
4. Dependency direction from `docs/ARCHITECTURE.md`. Nothing genre-specific leaks into a general crate.
5. Agent-friendliness: every capability an agent needs is reachable by an MCP tool with a typed schema, a clear refusal and a dry run. Names are searchable (skill `ai-door`).
6. Semver risk: what a change would break. Run `cargo semver-checks` or `cargo public-api` when installed, else say "not run".
7. Docs: public items without docs or examples (propose a docs row).
8. Pragmatic Rust Guidelines (microsoft/rust-guidelines): cite the guideline id per finding, such as `M-PUBLIC-DEBUG`, `M-ERRORS-CANONICAL-STRUCTS`, `M-STRONG-TYPES`, `M-INIT-BUILDER`, `M-AVOID-WRAPPERS`, `M-PANIC-ON-BUG`, `M-HOTPATH`, `M-MEM-REUSE`, `M-SINGLE-ITEM-PATH`, `M-DESIGN-FOR-AI`, `M-TAUTOLOGICAL-TESTS`.
9. Scorecard: start from `node tools/rust-quality.mjs --crate <crate>` and `--list <metric>`. A row proposal names the metric it lowers and by how much.

Return a score 0 to 10 per check, the ten highest-value fixes as row proposals (area, owned file, acceptance, needs; format in skill `commit-and-board`), and what you could not check.

## 5. Audit one area (read-only)

Audit one coverage area under `AGENTS.md` and `docs/CODELAW.md`.
- Search real callers, manifests, fixtures, runtime path reads and docs before you call anything dead. Include tools, CI, skills, content and examples, not only Rust crates.
- Report at most five findings. Each has evidence (path and line), impact, current owner, the minimal repair or removal, an acceptance command, and your uncertainty. Reuse known blockers.
- Rank Rust debt from the scorecard (`node tools/rust-quality.mjs --list <metric>`): duplicated bodies, re-derived core helpers, hot-path allocation and nondeterminism first. Each finding names the metric and crate count it lowers.
- Never delete caches or user work. Never rewrite code or docs during an audit. Accepted findings become board rows (skill `commit-and-board`).
- Return paths, checks with results, and unknowns.

## 6. Gate on the integrated tree, and owned-path repairs

- Run the row's targeted checks on the integrated tree and return every `[PASS]` and `[FAIL]` line. The full `node tools/proof.mjs` is heavy. Use `--skip-build`, or `node tools/gh-check.mjs` crate checks, for routine work.
- A repair of tools, CI, examples, content fixtures or build glue stays in the row's owned files. Check the script's behavior, not only its syntax: a syntax pass alone does not certify a workflow.
- No golden change and no finish-criteria change without a receipt or an owner call. Propose those in chat. Never run cleanup or workspace-wide autofixes.
