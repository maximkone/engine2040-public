---
name: engine-proof
description: Use when doing engine2040 proof work - owned-path MAINT repairs, integrated-tree gates, read-only JAN audits with ranked findings. Loaded by the builder or researcher for PROOF/JAN packets.
---

# Engine proof (from builder, researcher)

## PROOF: owned repair (from builder)

Implement one MAINT packet in owned files:
tools, CI, examples, content fixtures or build glue. The packet names
the consumer, allowed files, workdir, targeted check and rollback
boundary. No root manifests, lockfiles, board, model pins,
permissions, golden hashes or finish criteria; propose those to the
orchestrator. Never run cleanup or workspace-wide autofixes. Check
script syntax plus the behavior; a syntax pass alone does
not certify a workflow.

## PROOF: gate on the integrated tree

After collection, run the packet's targeted checks on the integrated
tree and return every `[PASS]`/`[FAIL]` line. Full `node tools/proof.mjs`
is lead-only, once per wave; other seats use `--skip-build` or crate
checks, never full proof. Never author on
a gate packet; never gate your own diff.

## JAN: read-only audit (from researcher)

Audit one coverage area under AGENTS.md and docs/CODELAW.md. Search real
callers, manifests, fixtures, runtime path reads and docs before
calling something dead. Include tools, CI, skills, content and
examples, not only Rust crates. Report at most five findings:
evidence path:line, impact, current owner, minimal repair/removal,
acceptance command and uncertainty. Reuse known blockers. Rank Rust
debt from the scorecard the orchestrator hands you (`node
tools/rust-quality.mjs`, `--list <metric>`): duplicated bodies,
re-derived core helpers, hot-path allocation and nondeterminism
first; each finding names the metric and crate count it lowers.
Never delete caches, user work or halt files. Never rewrite
code or docs. The orchestrator turns accepted findings into owned
work packets.

Return paths, checks/results and unknowns.
