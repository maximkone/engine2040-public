# WORKFLOW — deliver integrated capabilities

Standalone repo (owner 2026-10-17): no sprints, no locks, no claims, no queue, no loop. `AGENTS.md` wins if this file differs.

1. Take the next ready row of `FOCUS.md` "## Now" (`node tools/board-slice.mjs --focus`, then `--ready`). Read its plan in `plan/`. Inspect existing work before editing.
2. State outcome, consumer, owned paths and acceptance evidence. Small work can be direct; multi-step work gets a short task list.
3. Commit shared contracts (types and signatures) before parallel work. Every change wires into the consumer the board row names.
4. Several AIs share the one tree `C:\engine2040` on `master` (no lanes, no worktrees). Pull first, re-read a file right before you edit it, never revert edits you did not make, commit only your own paths.
5. Prove it: `cargo check -p <crate>` locally; every test and clippy proof on GitHub with `node tools/gh-check.mjs cargo test -p <crate> ...`. Report compiled, unit-tested, integrated, visual and perf status separately. Keep missing backend coverage visibly unverified.
6. Before a commit: `node tools/rust-quality.mjs` PASS, `node tools/workspace-check.mjs` after board or doc edits, the hash twice when sim code changed. Stage scoped paths only, push `master`, never force-push.

## Task packet for work that needs coordination

Goal; active milestone; owner; base revision; owned/forbidden paths; shared
contract; dependencies; acceptance commands/fixtures; artifact; known risks.
Prefer the smallest complete vertical change over artificial two-file fragments.

## Failure handling

Diagnose, retry once with a concrete correction. A repeated identical failure
gets owner, fingerprint, reproduction and actual gate output in `BLOCKERS.md`.
Continue independent green work. Do not suppress the failure or change the
golden without a justified simulation change. A final handoff includes
unresolved failures, not just wins.

## Receipt

Changed paths + checks run/results + fixture/hash + relevant artifacts + remaining
gates. Add SHA only if a commit exists. Never invent a benchmark, snapshot,
rendered capture, external service check or full-engine coverage.
