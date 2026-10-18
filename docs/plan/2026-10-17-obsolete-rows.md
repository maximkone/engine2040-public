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

## Added 2026-10-18 (triage of the old E rows)

Each row below was checked against the tree on 2026-10-18. Not ticked, not deleted: the owner closes them.

- E5: the premise was a render panic count above its baseline. `node tools/rust-quality.mjs` shows render panic_sites 21 equal to the baseline 21 (RESULT PASS) and `tools/drift-check.mjs` is gone. Replaced by: the rust-quality ratchet (it holds the count) and the CQ pay-down rows.
- E8: a port of the Forge receipt/tape cases. The gap check is recorded in `docs/plan/forge-adoption.md`; the typed ABSTAIN leg lives on as AD1 (SPRINT-18-gaps.md); the ratchet counts-down is `rust-quality.mjs --tighten`. Replaced by: AD1 and the rust-quality ratchet.
- E11: split the 4th big render file. Render now has 6 files over the line cap and the whole monolith split is CQ9 (render and game files, one packet each). Replaced by: CQ9.
- E18: weight cut of lane worktrees, 2 spike crates and blobs. Lanes and worktrees are retired (one worktree on master), `crates/` holds no spike crate, history was slimmed 2026-10-02. Replaced by: nothing needed.
- E19: archive done text and shrink board and inbox. The inbox file is archived (`docs/archive/EMPIRE-INBOX-2026-10-16.md`) and the board is sharded; `docs/SPRINT.md` is 6 KB. Replaced by: the sharded board.
- E20: target is the center "empire check" (drift-check.mjs plus vision). `tools/drift-check.mjs` is gone. Replaced by: `node tools/workspace-check.mjs` and the rust-quality ratchet.
- E24: intake of the vault V5 and V6 manifests. `content/ADOPT-VAULT-V7.md` records V7 (508/508, V6 507/507 plus one pack) and supersedes both. Replaced by: V7.
- E25: deep-dives for LM2, LM3 and RL5, asked for by the drift stuck-row line. `tools/drift-check.mjs` is gone, LM2 and LM3 are ticked, RL5 is a normal open row. Replaced by: nothing needed.
- E28: avian take into `crates/physics/src/solver.rs`. That file was deleted (3d6ee4a1); physics now runs on Rapier. Replaced by: nothing in this repo.
- E33: evidence appendix in `target/lead2.md`, the retired center "Lead 2" scoreboard (last written 2026-10-13, no reader left). Replaced by: dated evidence in the row notes.
- E34: queue width rules in `target/queue/batch.md`. The queue and standing seats went with the OpenCode loop. Replaced by: lead commits by path, no queue.
- E35: queue drain (claims, running, win6-top lane, CT42 intake). Same: queue and lanes are retired. Replaced by: lead commits by path, no queue.
- E36: "rating 3 checks" (empire check drift, vision, arsenal). The empire check and `drift-check.mjs` were center tools and are gone. Replaced by: `node tools/workspace-check.mjs` and the rust-quality ratchet.
- E41: casts, solver and ragdoll in physics. `broadphase.rs` and `solver.rs` were deleted (3d6ee4a1); physics casts are now 5 and the ratchet holds them. Replaced by: the rust-quality ratchet.
- E45: LICENSE, steal-next, jams and bigdocs. `tools/steal-next.mjs` and `docs/jams/` are gone; the COMPARE and LADDER part split is covered by `tools/compare-split.test.mjs` and `tools/ladder-split.test.mjs`. Replaced by: those two tests.
- E47: drift-check speed. `tools/drift-check.mjs` is gone. Replaced by: nothing.
- E53: pool-wait ETA in `tools/lanes.mjs`, which is gone with the lanes. Replaced by: `node tools/gh-check.mjs`.
- E54: the premise "src/ never imports 9 root deps" is wrong. The root package's `examples/` and `tests/` import engine_core, ecs, physics, nav, audio, net, assets and ai, and `Cargo.toml` keeps them as the bare-build library set on purpose. Dropping them breaks `cargo test -p engine2040`. Replaced by: nothing (keep the deps).
- E60: `.opencode/agents/researcher.md` is gone (13 of 14 agents cut 2026-10-17) and center's warden is gone. Replaced by: nothing.
- E83: cap `target/queue/done` and park stale ready packets. The queue is retired. Replaced by: nothing.
- E84: `tools/handoff.mjs` from the sprint lock. `tools/sprint-loop.mjs` is gone. Replaced by: the lead's wrap-up.
- E85: one live claim per path in `tools/lane-board.mjs`, which is gone. Replaced by: lead commits by path, no claims.
- E88: opencode.jsonc points arxiv and github at shared bridges. The OpenCode loop is retired (2026-10-17), so its MCP config has no reader. Replaced by: nothing.
- E89: use the shared skills shelf in opencode.jsonc and drop 8 shelf copies. `.opencode/skills` was folded into `skills/` (0b6daaf0) and the OpenCode loop is retired. Replaced by: `skills/` in the repo.
- E94: make `drift-check.mjs` print RESULT PASS by writing 3 deep-dives. The tool is gone and EG11, OPT24 and EG18 are ticked. Replaced by: nothing needed.
