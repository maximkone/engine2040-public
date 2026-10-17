# Obsolete board rows (2026-10-17)

Found by the backlog wave while finishing the oldest Empire inbox drain rows.
Each row below names a target that is gone, or a premise the owner retired.
Nothing here is ticked or deleted from the board: the owner closes them in one go.

Format: ID, why obsolete, what replaced it.

- E4: target `kaggle.mjs` (center mesh step) is gone; the center pipeline was retired by the standalone-repo decision 2026-10-17. Replaced by: nothing in this repo.
- E6: target `tools/lanes.mjs` (land/prune of lanes) is gone; lanes retired 2026-10-17. Replaced by: one worktree on master and commit-by-path (CLAUDE.md, one-worktree-main).
- E12: target `tools/ab-bench.mjs` is gone. Replaced by: the brief chain proofs in `crates/playtest` brief_eval (EV2) and `crates/endpoint` build_game tests.
- E17: target `tools/lanes.mjs` is gone and the forge undo journal it ported from is retired. Replaced by: one worktree on master and commit-by-path (CLAUDE.md).
- E21: target `tools/status-sync.mjs`, the standing seats and `target/queue/` went with the OpenCode loop (retired 2026-10-17). Replaced by: lead commits by path, no claims or queue.
- E22: target `tools/look-gate.mjs` is gone and no replacement exists in the repo. Replaced by: nothing in the repo.
- E26: premise is the cargo lane memo, and cargo lanes are retired. Replaced by: `node tools/gh-check.mjs` runs each cargo proof once on GitHub (heavy-cargo-on-github, 2026-10-17).
