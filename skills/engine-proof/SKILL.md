---
name: engine-proof
description: Use when doing engine2040 proof work - owned-path MAINT repairs, integrated-tree gates, read-only JAN audits with ranked findings. Use for PROOF/JAN rows.
---

# Engine proof

## PROOF: owned repair

Implement one MAINT row in owned files:
tools, CI, examples, content fixtures or build glue. The row names
the consumer, allowed files, workdir, targeted check and rollback
boundary. No golden hash change or finish criteria without a receipt
or an owner call; propose those in chat. Never run cleanup or workspace-wide autofixes. Check
script syntax plus the behavior; a syntax pass alone does
not certify a workflow.

## PROOF: gate on the integrated tree

Run the row's targeted checks on the integrated
tree and return every `[PASS]`/`[FAIL]` line. Full `node tools/proof.mjs`
is heavy: use `--skip-build` or `node tools/gh-check.mjs` crate checks for
routine work. Never gate your own diff.

## JAN: read-only audit

Audit one coverage area under AGENTS.md and docs/CODELAW.md. Search real
callers, manifests, fixtures, runtime path reads and docs before
calling something dead. Include tools, CI, skills, content and
examples, not only Rust crates. Report at most five findings:
evidence path:line, impact, current owner, minimal repair/removal,
acceptance command and uncertainty. Reuse known blockers. Rank Rust
debt from the scorecard (`node
tools/rust-quality.mjs`, `--list <metric>`): duplicated bodies,
re-derived core helpers, hot-path allocation and nondeterminism
first; each finding names the metric and crate count it lowers.
Never delete caches, user work or halt files. Never rewrite
code or docs. Accepted findings become board rows (skill `board-row-writing`).

Return paths, checks/results and unknowns.
