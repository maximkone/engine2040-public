# WORKFLOW — deliver integrated capabilities

1. Read current status and the active `ROADMAP.md` milestone. Inspect existing
   work before editing. Research catalogs are candidate lists, not marching orders.
2. State outcome, consumer, owned paths and acceptance evidence. Small work can
   be direct; multi-step work gets a short task list. No required wave IDs.
3. Commit shared contracts (types and signatures) before parallel
   implementation. Every task wires into the consumer the `SPRINT.md` board names.
4. Code goes to `builder` workers from the `SPRINT.md` board, plans to
   `planner`: as many code packets per wave as there are free crates (no RAM cap: owner 2026-10-14), one per crate, each
   on its owned paths in the one tree `C:\engine2040` on master (no lanes, no
   worktrees; owner 2026-10-10). One sprint per working tree
   (`tools/sprint-lock.mjs`); `tools/sprint-loop.mjs` runs waves autonomously.
   Do not fake tools or depend on another repository's task service. The owner's floor and width in .opencode/knobs.json are the objective: always at least the floor, different kinds of work. When waiters queue on the build lock, run a narrow wave first and land finished work when
   the lock is free.
5. Run applicable gates. Report compiled/unit-tested/integrated/visual/perf status
   separately. Keep missing backend coverage visibly unverified.
6. Review the diff. `/sprint` commits each green task with explicit paths;
   otherwise commit only when asked. `/sprint` pushes each green wave;
   other work pushes only when asked. Never force-push.
   For a commit, inspect status/diff/recent log, run the gate with the hash
   twice, and stage scoped paths only.

## Task packet for work that needs coordination

Goal; active milestone; owner; base revision; owned/forbidden paths; public
contract; dependencies; acceptance commands/fixtures; artifact; known risks.
Prefer the smallest complete vertical change over artificial two-file fragments.

## Failure handling

Diagnose, retry once with a concrete correction. A repeated identical failure
gets owner, fingerprint, reproduction and actual gate output in `BLOCKERS.md`.
Quarantine that task and continue independent green work. Do not suppress the
failure or change the golden without a justified simulation change. Halt when
the operator asks. A final handoff includes unresolved failures, not just wins.

## Receipt

Changed paths + checks run/results + fixture/hash + relevant artifacts + remaining
gates. Add SHA only if a requested commit exists. Never invent a benchmark,
snapshot, rendered capture, external service check or full-engine coverage.
