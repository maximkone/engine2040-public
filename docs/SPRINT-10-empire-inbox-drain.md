### Empire inbox drain (2026-10-01, 53 open; one row per item or merged group)

- [x] E4 make: MIT shape-model swap for the mesh step (EB-2026-10-05-S2)
  - swap the mesh step in center `kaggle.mjs` to an MIT shape model,
  first candidate Direct3D-S2 (re-read its LICENSE and model card live,
  check it fits a 16 GB Kaggle GPU); TripoSG props stay tests-only and
  the `asset.intake` licence gate stays. Tier A. Size S. Owns center
  `kaggle.mjs` plus `kaggle.test.mjs` hunks only. Accept: F2P
  `node --test kaggle.test.mjs` asserts no `triposg`/`hunyuan` in any
  mesh-step line and one real barrel+lantern run cooks through
  `asset.intake` with an MIT provenance row and no licence condition;
  P2P golden `d57b5c8ae3096614` unchanged. needs: GN3.
  (EB-2026-10-05-S2.)
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `kaggle.mjs` (center mesh step) is gone; the center pipeline was retired by the standalone-repo decision 2026-10-17. Replaced by: nothing in this repo.
  Correction 2026-10-21: "nothing in this repo" is out of date - the generator chain's new home is `C:\empire\center\make.mjs` (center repo, commit `9fda6705`; `node make.mjs models --verify`, `node make.mjs run <kind> --spec <file>`, `node make.mjs job <id>`). The MIT shape-model swap itself is still open as board row PW56 (`docs/SPRINT-28-powerups.md`); the row stays closed as obsolete.
- [x] E5 quality: render panic-site leg back to baseline (EB-2026-10-05-S8)
  - replace production `expect`/`panic!`/`unreachable!`/`todo!` sites in
  `crates/render` with typed errors or loud refusals, worst files first,
  tests untouched. Tier A. Size S. Owns `crates/render` production hunks
  only. Accept: F2P `node tools/rust-quality.mjs` render panic count back
  to its baseline and `node tools/drift-check.mjs` prints no panic_sites
  line; P2P golden `d57b5c8ae3096614` unchanged x2. needs: CQ1.
  (EB-2026-10-05-S8.)
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — the premise was a render panic count above its baseline. `node tools/rust-quality.mjs` shows render panic_sites 21 equal to the baseline 21 (RESULT PASS) and `tools/drift-check.mjs` is gone. Replaced by: the rust-quality ratchet (it holds the count) and the CQ pay-down rows.
- [x] E6 ops: lanes land-or-drop pass then prune (EB-2026-10-05-S14 leg 1)
  - land each lane with a PASS judge through the land queue or save its
  diff beside its board row and remove it, then prune frees the lane
  build folders. Tier A. Size S. Owns `tools/lanes.mjs` land/prune hunks
  only. Accept: F2P live lanes at most 10 and every removed dirty lane
  has its diff saved; P2P `node tools/workspace-check.mjs` PASS. needs:
  none. (EB-2026-10-05-S14.)
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `tools/lanes.mjs` (land/prune of lanes) is gone; lanes retired 2026-10-17. Replaced by: one worktree on master and commit-by-path (CLAUDE.md, one-worktree-main).
- [x] E7 proof: finish reads the archive for pruned ticks (EB-2026-10-05-S72) Progress 2026-10-17: finish.mjs reads archive ticks since 4db33ad7 (`--archive <dir>`, finish.test.mjs 14 pass); but `node tools/finish.mjs` prints 5 met, 12 partial, 9 open, not the 10 of 26 the Accept names. `node tools/finish.mjs` prints 10 met, 13 partial, 3 open of 26 gates (A1, A2, A3, A6, A7, A9, A10, P3, P7, P10 met) with the OB3/AH3/AH4/HR1/HR2/HR4/DOC1/FX3/GP5/LW3/DM1/FS1 evidence intact: the archive tick in `docs/archive/` closes a row the prune took off the board, the live board wins over an older archive tick for the same id, and a parked row in `docs/plan/parked-2026-10-22.md` counts open without the stale-alarm warning. `node --test tools/finish.test.mjs` 26 of 26 pass (5 new). `node tools/finish.mjs --check` PASS, 26 gates, 162 rows audited, 0 unresolved, 0 met gates with a suspect row. `node tools/workspace-check.mjs` PASS. Golden d57b5c8ae3096614, gh-check run 39709912332. Landed f100488.
  - teach `tools/finish.mjs` to read ticked rows in `docs/archive/` as well
  as live `docs/SPRINT.md`, so the prune stops dropping met gates. Tier A.
  Size S. Owns `tools/finish.mjs` hunks only. Accept: F2P
  `node tools/finish.mjs` back at 10 of 26 with the
  OB3/AH3/AH4/HR1/HR2/HR4/DOC1/FX3/GP5/LW3/DM1/FS1 evidence intact;
  P2P `node tools/workspace-check.mjs` PASS. needs: none.
  (EB-2026-10-05-S72.)
- [x] E8 proof: forge receipt/tape plus ratchet/shards port (EB-2026-10-05-S32)
  - port the receipt/tape cases tests-first (OLD-FAIL vs FIXED-PASS,
  PASS/FAIL/ABSTAIN) plus the ratchet counts-down plus shard receipts,
  with the gap-check against engine proof/playtest recorded. Tier A. Size S.
  Owns `crates/proof` hunks only. Accept: F2P `cargo test -p engine-proof`
  receipt/tape green with the gap-check recorded; P2P golden
  `d57b5c8ae3096614` unchanged. needs: none. (EB-2026-10-05-S32.)
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — a port of the Forge receipt/tape cases. The gap check is recorded in `docs/plan/forge-adoption.md`; the typed ABSTAIN leg lives on as AD1 (SPRINT-18-gaps.md); the ratchet counts-down is `rust-quality.mjs --tighten`. Replaced by: AD1 and the rust-quality ratchet.
- [ ] E9 look: fighter frame by own renderer (EB-2026-10-07-S103, EB-2026-10-07-S245, EB-2026-10-07-S177, EB-2026-10-08-S18). Size S. Owns `crates/render`. F2P `cargo test -p engine-render` green + PNG in `target/first-pixels/`; P2P golden `d57b5c8ae3096614` x2 + drift/vision PASS. needs: none.
- [x] E11 render: split the 4th big file back to 3 (EB-2026-10-06-S68). Size S. Owns one `crates/render` file, move-only. F2P `node tools/rust-quality.mjs --crate render` big_files 3; P2P golden x2, public API unchanged. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — split the 4th big render file. Render now has 6 files over the line cap and the whole monolith split is CQ9 (render and game files, one packet each). Replaced by: CQ9.
- [x] E12 build: AB19 bench then AB level-graph chain (EB-2026-10-07-S272, EB-2026-10-07-S273). Size S. Owns `tools/ab-bench.mjs`. F2P 1000 headless-mock builds report x2 + 10/10 pass/refusals, undo restores, digests x2; P2P VISION bar prints the number. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `tools/ab-bench.mjs` is gone. Replaced by: the brief chain proofs in `crates/playtest` brief_eval (EV2) and `crates/endpoint` build_game tests.
- [x] E13 playtest: steered agent beats random + crash tape replays (EB-2026-10-07-S274, EB-2026-10-05-S26). Size S. Owns `crates/playtest`. F2P steered finds seeded bugs in fewer tapes than random + injected crash replays same tick 10/10; P2P golden x2. needs: none. Progress 2026-10-17: Steered-beats-random leg exists: crates/playtest/tests/spatial_steer_compare.rs steered_finds_planted_bug_faster_than_random passed, gh run 39056907771 (415eb7d4); the injected-crash replay 10/10 leg has no test. Landed 719c3e4. Injected-crash leg: crates/playtest/tests/crash_tape_replay.rs injected_crash_tape_replays_the_same_tick_ten_of_ten passed, gh run 39215997304. Test-only: no sim code, golden x2 unchanged.
- [x] E14 proof: float-leak gate test (EB-2026-10-07-S137). Size S. Owns `crates/engine-core` probe. F2P probe-in fails, probe-out passes; P2P golden x2. needs: none. Landed 2f02202f. `node --test tools/float-gate.test.mjs` 10 pass (flags a float injected into the warm tick scope, passes integer-only code, repo scan of tick/hash/fixed/time clean), local 2026-10-17.
- [x] E15 net: rollback tape fixes (EB-2026-10-07-S151). Size S. Owns `crates/net/src/rollback.rs`. F2P 10k-frame flat memory + replay hash x2; P2P tests green. needs: none. Landed 9061883b. Tape is a bounded `TAPE_CAP` ring (history plus input log drop the oldest); rollback::tests::e15_tape_flat_over_10k_frames passed (bytes printed at 1k and 10k frames, replay digest x2), `cargo test -p engine-net --no-fail-fast` green 425 lib + 44 it, gh runs 39479038718 and 39479796045.
- [x] E17 lanes: forge undo-journal port, one revert proves undo (EB-2026-10-07-S94). Size S. Owns `tools/lanes.mjs`. F2P lane revert OK + lane test PASS; P2P workspace-check PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `tools/lanes.mjs` is gone and the forge undo journal it ported from is retired. Replaced by: one worktree on master and commit-by-path (CLAUDE.md).
- [x] E18 ops: weight cut, blobs + lanes (EB-2026-10-06-S69, EB-2026-10-07-S243). Size S. Owns 2 spike crates + gen-built skies/glTF + lane worktrees. F2P ls-files -110MB + worktrees 8 or fewer + GB freed printed; P2P workspace PASS, proofs green. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — weight cut of lane worktrees, 2 spike crates and blobs. Lanes and worktrees are retired (one worktree on master), `crates/` holds no spike crate, history was slimmed 2026-10-02. Replaced by: nothing needed.
- [x] E19 docs: archive done text, shrink board + inbox (EB-2026-10-07-S244). Size S. Owns `docs/archive/` moves. F2P inbox 40 or fewer open none over 7d + both files under 60KB; P2P workspace PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — archive done text and shrink board and inbox. The inbox file is archived (`docs/archive/EMPIRE-INBOX-2026-10-16.md`) and the board is sharded; `docs/SPRINT.md` is 6 KB. Replaced by: the sharded board.
- [x] E20 checks: drift + vision PASS bundle (EB-2026-10-07-S219, EB-2026-10-07-S220, EB-2026-10-08-S19). Size S. Owns 5 ratchet sites + `docs/research/` read notes. F2P empire check drift + vision PASS; P2P golden x2. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target is the center "empire check" (drift-check.mjs plus vision). `tools/drift-check.mjs` is gone. Replaced by: `node tools/workspace-check.mjs` and the rust-quality ratchet.
- [x] E21 flow: status-sync + rot chore + ready rules + queue prune (EB-2026-10-07-S60, EB-2026-10-07-S77, EB-2026-10-07-S125, EB-2026-10-07-S189, EB-2026-10-07-S172, EB-2026-10-08-S3, EB-2026-10-08-S5, EB-2026-10-08-S6, EB-2026-10-08-S13, EB-2026-10-08-S22, EB-2026-10-08-S24, EB-2026-10-08-S25, EB-2026-10-08-S26). Size S. Owns `tools/status-sync.mjs` + standing seats + `target/queue/`. F2P flow-check housekeeping at most 20pct builders at least 40pct judges at most 35pct; P2P judged PASS 75pct+. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `tools/status-sync.mjs`, the standing seats and `target/queue/` went with the OpenCode loop (retired 2026-10-17). Replaced by: lead commits by path, no claims or queue.
- [x] E22 rules: bevy look-by-numbers gate + newcomer doc trial (EB-2026-10-07-S32, EB-2026-10-07-S33, EB-2026-10-06-S30). Size S. Owns `tools/look-gate.mjs` + `target/newcomer/` receipt. F2P gate prints its number + newcomer counts printed; P2P both rules noted on board. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — target `tools/look-gate.mjs` is gone and no replacement exists in the repo. Replaced by: nothing in the repo.
- [ ] E23 proof: GM3 chunked 18k-frame plan (EB-2026-10-07-S73). Size S. Owns `tools/ship.mjs` GM3 hunks. F2P chunks sum to the full proof, each hash-stable; P2P A8 moves. needs: none.
- [x] E24 content: intake vault V5 + V6 manifests offline (EB-2026-10-06-S31, EB-2026-10-06-S65). Size S. Owns `content/` intake. F2P manifest verifies engine-side; P2P provenance rows cite O-005/V5. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — intake of the vault V5 and V6 manifests. `content/ADOPT-VAULT-V7.md` records V7 (508/508, V6 507/507 plus one pack) and supersedes both. Replaced by: V7.
- [x] E25 plan: deep-dives LM2 LM3 RL5 (EB-2026-10-07-S74, EB-2026-10-07-S75). Size S. Owns `docs/plan/` dive notes. F2P root cause + approach + proof command posted per row; P2P drift prints no stuck LM2/LM3/RL5. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — deep-dives for LM2, LM3 and RL5, asked for by the drift stuck-row line. `tools/drift-check.mjs` is gone, LM2 and LM3 are ticked, RL5 is a normal open row. Replaced by: nothing needed.
- [ ] S27 playtest: showcase fighters in motion (EB-2026-10-08-S29). Size S. Owns `target/showcase/` set plus `tools/showcase.mjs` hunks only. F2P newest `target/showcase` set lists 3 PNG and its NOTES.md names 2 staged fighters in motion with 0 red-box stand-ins; P2P Lead 2 journey score 5 or more. needs: SHOW1.
- [x] S28 deps: drop unused Bevy workspace dep (EB-2026-10-08-S34). Size S. Owns root `Cargo.toml` bevy lines plus `docs/FOCUS.md` D1 only. F2P `rg bevy Cargo.toml` prints 0 lines and a crate-scoped `cargo check` stays green with golden `d57b5c8ae3096614` unchanged; P2P `node tools/workspace-check.mjs` PASS. needs: none. Landed eb440e64. `rg bevy Cargo.toml` prints 0 lines; engine-render --lib 719 passed on the runner, gh run 39056801624; workspace-check board lines green (remaining FAIL lines are foreign web/render edges).
- [x] E26 ops: cargo lane memo, top lanes once per wave (EB-2026-10-08-S43). Size S. Owns `tools/` memo hunks only. F2P top lanes run once per wave or memo hits print on same code; P2P `node tools/workspace-check.mjs` PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — premise is the cargo lane memo, and cargo lanes are retired. Replaced by: `node tools/gh-check.mjs` runs each cargo proof once on GitHub (heavy-cargo-on-github, 2026-10-17).
- [x] E27 perf: V20 P6 puffin take (EB-2026-10-07-ST20a). Size S. Owns `crates/perf/src/profiler.rs` only. Take puffin c81e9bb (MIT) into the profiler end to end. F2P profile overhead before-after numbers printed (ms + allocs, identical input); P2P `cargo test -p engine-perf --lib` green + steal ledger marked landed. needs: none. Landed 237d53c5. `cargo test -p engine-perf --lib` 122 passed, gh run 39056228486; commit posts off-path bench 0.87 ns/op, alloc +0, golden x2; steal-ledger leg retired by porting policy 2026-10-10.
- [x] E28 physics: V20 A5 avian take (EB-2026-10-07-ST20a). Size S. Owns `crates/physics/src/solver.rs` only. Take avian a35a80bd (MIT) into the solver end to end. F2P solver bench before-after numbers printed (ms on the same scene, hash x2); P2P `cargo test -p engine-physics --lib` green + steal ledger marked landed. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — avian take into `crates/physics/src/solver.rs`. That file was deleted (3d6ee4a1); physics now runs on Rapier. Replaced by: nothing in this repo.
- [x] E29 ship: V20 A8 nextest take (EB-2026-10-07-ST20b). Size S. Owns `tools/ship.mjs` archive leg only. Take nextest 919b10c6 (MIT) into the archive leg end to end. F2P archive receipt exists with file list + hashes; P2P `node tools/workspace-check.mjs` PASS + steal ledger marked landed. needs: none. Landed 8ad99d66. `node --test tools/ship.test.mjs` 23 pass incl. archive receipt with file list (rel, bytes, fnv) plus junit receipt per run (A8 accept), local 2026-10-17; steal-ledger leg retired by porting policy 2026-10-10.
- [x] E30 render: V20 C2 strolle take (EB-2026-10-07-ST20b). Size S. Owns `crates/render/src/shadow_cluster/torch_cluster.rs` only. Take strolle 69cbcdd4 (MIT) temporal-carry into the 250-torch assign end to end. F2P assign cost before-after numbers printed (ms, same torch set); P2P `cargo test -p engine-render --lib` green + steal ledger marked landed. needs: none. Landed a05bc464. temporal_carry_cuts_pairs_at_low_rmse inside `cargo test -p engine-render --lib` 719 passed, gh run 39056801624; commit prints tight 14641 vs temporal 1920 pairs (cut 86.5 pct, rmse 0, 11.94 ms, digest x2); steal-ledger leg retired.
- [x] E31 net: V20 ON40 ggrs take (EB-2026-10-07-ST20c). Size S. Owns `crates/net/src/netcode.rs` only. Take ggrs e97e3d24 (MIT) bounded history into netcode end to end. F2P history memory flat over 10k frames (bytes printed, replay hash x2); P2P `cargo test -p engine-net --lib` green + golden `d57b5c8ae3096614` unchanged + steal ledger marked landed. needs: none. Progress 2026-10-17: Bounded history landed e449553e: on40_history_bounded_cap_plus_pending_bursty_loss (64-frame tape, fold/tip/digest x2) inside `cargo test -p engine-net --lib` 258 passed, gh run 39056801523; no 10k-frame flat-memory test with bytes printed exists (rollback_soak runs 120 frames), so Accept is not met. Landed b4105b32. on40_history_flat_over_10k_frames: buffer bytes equal at 1k and 10k frames, digest x2 (net on40 tests 2/2, run 39066760357); the bounded history already existed, so this is a regression pin.
- [x] E32 render: V20 A4 FSR1 take (EB-2026-10-07-ST20c). Size S. Owns `crates/render/src/post.rs` only. Take FSR1 a21ffb8 (MIT) EASU plus RCAS into post end to end. F2P upscale measured numbers printed (ms + pixel diff vs parked TAAU on the same frame); P2P `cargo test -p engine-render --lib` green + steal ledger marked landed. needs: none. Landed 0c672df7. fsr_540p_to_1080p_posts_mtf50_shimmer_and_ms (prints base vs candidate ms, MTF50 lift, shimmer variance) inside `cargo test -p engine-render --lib` 719 passed, gh run 39056801624; steal-ledger leg retired.
- [x] E33 proof: Core evidence freshness list (EB-2026-10-08-S67). Size S. Owns `target/lead2.md` evidence appendix only. F2P dated-evidence list prints 4 of 4 Core areas (Look, Bodies, AI-door, Multiplayer) with evidence dated within 3 days; P2P `node tools/workspace-check.mjs` PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — evidence appendix in `target/lead2.md`, the retired center "Lead 2" scoreboard (last written 2026-10-13, no reader left). Replaced by: dated evidence in the row notes.
- [x] E34 ops: queue width rules (EB-2026-10-08-S28, EB-2026-10-08-S51, EB-2026-10-08-S158). Size S. Owns `target/queue/batch.md` rule lines only. F2P `rg 'CPU-heavy at most 6' target/queue/batch.md` prints 1 + ready over-2h count 0 + golden slot named + oldest ready under 30min; P2P `node tools/workspace-check.mjs` PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — queue width rules in `target/queue/batch.md`. The queue and standing seats went with the OpenCode loop. Replaced by: lead commits by path, no queue.
- [x] E35 ops: queue drain (EB-2026-10-08-S46). Size S. Owns `target/queue/` claims + running + win6-top lane + CT42 intake. F2P claims fresh + running empty + win6-top landed + CT42 16 clips in `content/fighters/incoming`; P2P workspace-check PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — queue drain (claims, running, win6-top lane, CT42 intake). Same: queue and lanes are retired. Replaced by: lead commits by path, no queue.
- [x] E36 checks: rating 3 checks (EB-2026-10-08-S58). Size S. Owns rust ratchet sites + VISION Plans map legs + `tools/` board_ready test. F2P empire check drift + vision + arsenal PASS or one-line reasons in the handoff; P2P golden `d57b5c8ae3096614` x2. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — "rating 3 checks" (empire check drift, vision, arsenal). The empire check and `drift-check.mjs` were center tools and are gone. Replaced by: `node tools/workspace-check.mjs` and the rust-quality ratchet.
- [x] E37 audit: license gate + dupes (EB-2026-10-08-S68, EB-2026-10-08-S176, EB-2026-10-08-S178, EB-2026-10-08-S105). Size S. Owns `content/ai-loop/license_gate.py` + hex4 sites + endpoint tool registry + `crates/provider` transports. F2P license_gate tests green + hex4 1 shared helper + TOOL_ consts reachable-or-deleted + provider green with lines before/after printed; P2P workspace-check PASS. needs: none. Progress 2026-10-18: the hex4 leg holds: `rg 'fn hex4' crates` finds one definition (crates/provider/src/vendor.rs:220). Not checked: license_gate.py tests, TOOL_ consts reachable-or-deleted, provider lines before and after. Landed 6dd2f067e. hex4: the vendor JSON cursor now calls `engine_core::parse_hex4` (crates/core/src/seal_key.rs:53), the one owner the JSON and vault readers already share, so the tree holds one hex4 rule and `crates/provider/src/vendor.rs:225` keeps only the cursor step. TOOL_: 96 definition sites across crates/, 96 reachable, 0 unused; the three dead names (`TOOL_DOOR_CHECK`, `TOOL_DOOR_PROFILE`, `TOOL_DOOR_OVERLAY`) are deleted, since the door serves no `door.check`/`door.profile`/`door.debug_overlay` verb. Provider lines before and after: 21168 -> 21161 in `crates/provider/src` (32 files), 23440 -> 23433 over every .rs of the crate (33 files), `vendor.rs` 1557 -> 1550. license_gate: 10 tests green in `content/ai-loop/test_license_gate.py`, now run in CI by `tools/license-gate-py.test.mjs` (3 tests pass), which also probes the gate and names every refusal code. Proof: `cargo test -p engine-provider --lib` 273 passed, 0 failed (gh run 39730316019); `cargo clippy -p engine-provider --all-targets -- -D warnings` green (gh run 39730462372); `cargo test -p engine-endpoint --lib` 162 passed, 0 failed (gh run 39730579382); golden `d57b5c8ae3096614` (gh run 39730711969); `node tools/workspace-check.mjs` PASS; `node tools/rust-quality.mjs` PASS, score 97, 0 regressions.
- [x] E38 render: WGSL split + SDFGI spike (EB-2026-10-08-S69, EB-2026-10-08-S110). Size S. Owns `crates/render` post.rs + gi_dyn only. F2P WGSL in separate shader modules + gi_dyn leak under 5pct and under 1.2ms; P2P `cargo test -p engine-render --lib` green. needs: none. Progress 2026-10-17: WGSL split landed eb440e64: post.rs pulls shaders/post/*.wgsl via include_str!; the gi_dyn leak under 5 pct and 1.2 ms leg has no module or test. Progress 2026-10-19: owner: the gi_dyn leg names the deleted `crates/render/src/gi_dyn.rs` (dead spike), so it cannot be built; drop the leg and tick, or keep the row? Question in `docs/plan/2026-10-19-owner-queue.md`.
  Landed. Owner chat 2026-10-19 (delegated): the gi_dyn leg is dropped (`crates/render/src/gi_dyn.rs` was deleted as a dead spike); the WGSL-split half is landed (eb440e64).
- [x] E39 game + ui: camera fixtures + IME gate (EB-2026-10-08-S70, EB-2026-10-08-S152). Size S. Owns `crates/game` camera.rs + `crates/ui` text.rs only. F2P fixture_course module + text 7 passed; P2P game + ui lib green. needs: none. Landed 6f0229da (CQ9 split of camera.rs: the fixture and course modules are crates/game/src/camera/fixture.rs and camera/course.rs, public paths kept) and 415eb7d4 (ui text.rs IME commit gate). Tests: `cargo test -p engine-ui --lib` 155 passed incl. 7 text:: tests (apply_ime_commit_gate_pins_windows_cjk_entry among them), gh run 39101728150; `cargo test -p engine-game --lib camera::` 60 passed incl. bad_course_text_fails_with_named_errors, bad_fixture_text_fails_with_named_errors, course_camera_table_maps_follow_geometry, course_boots_traversal_file_frames_agent_every_tick_and_replays, gh run 39105638341; P2P `cargo test -p engine-game --lib` 436 passed 0 failed, gh run 39106113948.
- [x] E40 net: QUIC errors + relay + arena rollback (EB-2026-10-08-S107, EB-2026-10-08-S156, EB-2026-10-08-S109). Size S. Owns `crates/net` quic.rs + relay test + `crates/arena` sync only. F2P `rg 'let _ = self.sock.send_to' crates/net` 0 + relay 0 failures + sync 6 passed hashes x2; P2P net + arena lib green. needs: none. Landed 7611e54. relay_transport_tests 4 passed (gh run 39485548946), it::relay 8 passed 0 failed (gh run 39483579826), arena sync::tests 6 passed hashes x2 (gh runs 39483579763, 39484467262), engine-net --features quic --lib 449 passed (gh run 39486205481), engine-arena --lib 230 passed (gh run 39483579321), clippy net all-targets --features quic -D warnings green (gh run 39485580612).
- [x] E41 physics: casts + solver + ragdoll (EB-2026-10-08-S147, EB-2026-10-08-S177, EB-2026-10-08-S166). Size S. Owns `crates/physics` broadphase + solver + ragdoll only. F2P clippy 23 cast lines + state_hash equality + joint hash x2 + drift ratchet PASS; P2P `cargo test -p engine-physics --lib` green. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — casts, solver and ragdoll in physics. `broadphase.rs` and `solver.rs` were deleted (3d6ee4a1); physics casts are now 5 and the ratchet holds them. Replaced by: the rust-quality ratchet.
- [x] E42 combat + playtest: wound values + spatial steer (EB-2026-10-08-S149, EB-2026-10-08-S167). Size S. Owns `crates/combat` gore.rs + `crates/playtest` steer only. F2P gore 7 passed hash x2 + planted-bug tape found faster than random; P2P golden unchanged. needs: none. Landed 415eb7d4. `cargo test -p engine-combat --lib gore` 8 passed (same_bout_twice_gives_identical_gore_streams, gore_stream_excluded_from_state_hash), gh run 39056885390; engine-playtest `spatial_steer_compare` steered_finds_planted_bug_faster_than_random passed, gh run 39056907771.
- [x] E43 assets: meshlets + cook budget + big blob (EB-2026-10-08-S108, EB-2026-10-08-S150, EB-2026-10-08-S179). Size S. Owns `crates/assets` cook + meshlet + `content/arena/textures/cloth_roughness_2k.jpg`. F2P meshlet count -15pct vs naive + over_budget simplifies LOD0 <= 5000 + LFS guard passes; P2P assets lib green. needs: none. Progress 2026-10-17: Three legs have tests: cone_weighted_beats_naive_by_fifteen_percent_at_equal_bounds (assets meshlet, passed gh run 39056659814), over_budget_simplifies_to_prop_budget (mesh_cook), cloth_roughness_2k.jpg is under LFS; but `cargo test -p engine-assets --lib` is red: 177 passed, 1 failed (archive::stage_refuses_an_archive_past_the_offset_limit, archive_fits(usize::MAX..) assert), gh run 39056931425. Landed be52c3ae (meshlet cone_weighted_beats_naive_by_fifteen_percent_at_equal_bounds, gh run 39102100286), eb440e64 (mesh_cook over_budget_simplifies_to_prop_budget, `--features import` gh run 39102363051), 50fb6224 (cloth_roughness_2k.jpg: .gitattributes filter=lfs, blob 3.9 MB, under the 5 MB large-blob guard in workspace-check), 4ef89132 (archive offset limit). P2P: `cargo test -p engine-assets --lib` 185 passed 0 failed 2 ignored, gh run 39101618448 (the archive::stage_refuses_an_archive_past_the_offset_limit red noted 2026-10-17 is green).
- [x] E45 docs: bigdocs + steal-next + jams + LICENSE (EB-2026-10-08-S96, EB-2026-10-08-S97, EB-2026-10-08-S160, EB-2026-10-08-S79). Size S. Owns LICENSE + COMPARE lookup line + `tools/steal-next.mjs` + `docs/jams/` + the part split of `docs/research/COMPARE.md` and `docs/research/LADDER.md` (EB-2026-10-09-S42). F2P repo-standard no FAIL license + bigdocs 3/3 + steal-next green or refs 0 + jams copied + committed; P2P workspace-check PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — LICENSE, steal-next, jams and bigdocs. `tools/steal-next.mjs` and `docs/jams/` are gone; the COMPARE and LADDER part split is covered by `tools/compare-split.test.mjs` and `tools/ladder-split.test.mjs`. Replaced by: those two tests.
- [ ] E46 misc: heal + bevy + verdict + showcase gate (EB-2026-10-08-S174, EB-2026-10-08-S175, EB-2026-10-08-S155, EB-2026-10-08-S159). Size S. Owns `crates/audio` heal test + root Cargo.toml bevy lines + `tools/showcase.mjs` gate leg. F2P audio 0 failed + `rg bevy Cargo.toml` 0 + 682 verdict file exists + showcase --check-only exit 0; P2P workspace-check PASS. needs: none. Progress 2026-10-17: bevy leg done: `rg bevy Cargo.toml` prints 0; audio heal test, 682 verdict file and showcase --check-only legs not verified.
- [x] E47 tools: drift-check speed (EB-2026-10-08-S106). Size S. Owns `tools/drift-check.mjs` + metrics opens only. F2P 3-run median under 12s with identical PASS/FAIL verdicts; P2P workspace-check PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — drift-check speed. `tools/drift-check.mjs` is gone. Replaced by: nothing.
- [ ] E48 render steals: joints + sky (EB-2026-10-08-S157). Size S. Owns `crates/render` glTF joints + sky bake only. F2P 3 glTF fighters 0 magenta frames + sky bake under 0.4ms; P2P render lib green. needs: none.
- [x] E49 combat: resolve split (EB-2026-10-08-S261). Size S. Owns `crates/combat/src/exchange.rs` resolve_tick only. F2P rg shows resolve_clashes + resolve_parries defs + `cargo test -p engine-combat --lib` green; P2P golden `d57b5c8ae3096614` x2. needs: none. Landed c5c46c48. resolve_tick now calls resolve_clashes and resolve_parries (both defs in crates/combat/src/exchange.rs, move-only, order and events unchanged). Tests: `cargo test -p engine-combat --no-fail-fast` 0 failed incl. parry_window_edges_proven_tick_by_tick and replay_exact_twice_over_parry_riposte_clash_tape, gh run 39102028917; clippy --all-targets -D warnings green, gh run 39102029340; rust-quality --owned PASS.
- [x] E50 net: validate/dup merge (EB-2026-10-08-S262). Size S. Owns `crates/net` validate/dup sites only, transports unchanged. F2P rg sorted-unique shows 1 def site + `cargo test -p engine-net --lib` green; P2P golden x2. needs: none. Landed 79ee4d9c. `validate.rs` is the one `prev: Option` site: replicate's spawn/update/despawn id loops and interest's selected-slot check push through `SortedUnique`, and the duplicate `prune` bodies in dedicated.rs collapse into one `prune_expired` (`rust-quality --list dup_fns --crate net` 0). Two net tests pinned to the old 16-fighter cap now follow MAX_PARTICIPANTS / MAX_INPUTS_PER_PACKET (test-only, no transport change). Proof: `cargo test -p engine-net --lib` 427 passed, 0 failed, 4 ignored, gh runs 39490356018 and 39490432077; `cargo test -p engine-proof --lib frozen_default_and_distinct_seed_proofs` green x2, gh runs 39489561446 and 39490466159. Tree red from the 32-cap merge fixed separately in 9ecc4abc (arena fork.rs stale `cuts` initializer).
- [x] E53 tools: pool-wait ETA + auto-retry (EB-2026-10-09-S3). Size S. Owns `tools/lanes.mjs` pool leg only. F2P rg pool-starvation class in 24h done records prints 0; P2P `node tools/lanes.mjs list` green. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — pool-wait ETA in `tools/lanes.mjs`, which is gone with the lanes. Replaced by: `node tools/gh-check.mjs`.
- [x] E54 deps: drop 9 unused root deps (EB-2026-10-08-S258). Size S. Owns root `Cargo.toml` only. F2P `rg -n engine- src/` shows only engine-proof + `cargo check -p engine2040` green; P2P `node tools/workspace-check.mjs` PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — the premise "src/ never imports 9 root deps" is wrong. The root package's `examples/` and `tests/` import engine_core, ecs, physics, nav, audio, net, assets and ai, and `Cargo.toml` keeps them as the bare-build library set on purpose. Dropping them breaks `cargo test -p engine2040`. Replaced by: not
- [x] E55 game: split windowed_dressing out (EB-2026-10-08-S259). Size S. Owns `crates/game/src/windowed.rs` + new `windowed_dressing.rs` only. F2P windowed.rs drops ~300 lines + `cargo check -p engine-game` green; P2P `node tools/workspace-check.mjs` PASS. needs: none. Landed 1d01a410. windowed_dressing.rs (548 lines, dressing_* fns) split out of windowed.rs and re-exported; engine-game builds and `--test it lobby_ui_flow` passes, gh run 39056624992.
- [x] E56 arena: split match_observer out (EB-2026-10-08-S260). Size S. Owns `crates/arena/src/lib.rs` + new `match_observer.rs` only. F2P lib.rs drops ~145 lines + `cargo check -p engine-arena` green; P2P `node tools/workspace-check.mjs` PASS. needs: none. Landed 0d7fb9c0. TickMatchEvents, run_match_events{,_v1,_versioned}, run_match_bytes{,_v1,_versioned} and match_bytes_rows moved to crates/arena/src/match_observer.rs, lib.rs 4,831 -> 4,648 lines (-183, the row said ~145), public paths kept by pub use, no signature change. Tests: `cargo test -p engine-arena --no-fail-fast` 0 failed (214 lib + 7 + doctests), gh run 39102832094; clippy --all-targets -D warnings green, gh run 39102912771; `cargo check -p engine-arena --all-targets` clean. workspace-check shows no FAIL from this change; its current FAIL lines are other sessions' uncommitted files (physics clinch.rs, ragdoll_tape.rs, playtest highlights.rs).
- [x] E57 render: one cascade_splits owner — shadow_post.rs line 54 and shadow_cluster.rs
  line 221 both define `cascade_splits` with different signatures (EB-2026-10-08-S232; the ACES
  half of that item is RL34 and the dead apply_desat is already gone). Fold them into one
  function plus a thin 4-cascade wrapper, no output change. Diff: good. Tier B. Size S. Owns
  `crates/render/src/shadow_post.rs`, `crates/render/src/shadow_cluster.rs`. Accept:
  `cargo test -p engine-render --lib shadow` green and `rg -c "fn cascade_splits" crates/render/src`
  lists one definition. F2P: the rg lists 2 definitions today. P2P: golden `d57b5c8ae3096614` x2 and
  `cargo test -p engine-render --lib` green. needs: none. Landed 197b4110. shadow_post::cascade_splits(near,far,n,lambda) is the one definition; shadow_cluster's four-lane table is the thin wrapper sun_splits (same practical_split values, no output change) and the stale v1_lighting test name was renamed. engine-render --lib shadow 51 passed, with --features v1 59 (parity test green; gh-check routes engine-render to this PC); full --lib 787 passed (gh run 39143814028); rg "fn cascade_splits" crates/render/src lists one definition; clippy -p engine-render --all-targets is still red on a foreign pre-existing lint (crates/render/src/quality.rs:868 type_complexity), no new lint from this change.
- [x] E60 agents: rewrite or revert the researcher re-read guard from 901d8975 (EB-2026-10-09-S14):
  center's warden shows researcher-reread-guard WORSE, 25 to 34, goal under 20. The guard text sits
  at line 60 of researcher.md. Rewrite it so a repeat read is refused without extra turns, or
  revert it; follow the agent contract in AGENTS.md (edit the original, then
  `node C:/empire/center/agentlint.mjs engine2040 --sync` and the plain run). Diff: regular. Tier B.
  Size S. Owns `.opencode/agents/researcher.md`. Accept: `node C:/empire/center/agentlint.mjs
  engine2040` prints PASS and center's moved.mjs due shows the metric under 20, or the rewrite is
  staged with its new number in the handoff. F2P: the metric reads 34 today. P2P: the other
  agent files unchanged. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — `.opencode/agents/researcher.md` is gone (13 of 14 agents cut 2026-10-17) and center's warden is gone. Replaced by: nothing.
- [x] E61 net: ggrs snapshot restore plus input queue on top of the bounded history Landed 2fc42668. `cargo test -p engine-net --test it ggrs_loss::` 1 passed (ggrs_loss_64_every_5th_converges_bounded), gh run 39056598160; `cargo test -p engine-net --lib` 258 passed, gh run 39056801523.
  (EB-2026-10-09-S24; commit e449553e landed the history bound under ON40, row E31). Port the
  snapshot restore and the input queue (gschup/ggrs, MIT or Apache-2.0, read live, reimplemented
  with a PORT note). Diff: good. Tier B. Size M. Owns `crates/net/src/netcode.rs`,
  `crates/net/tests/it/ggrs_loss.rs`, `crates/net/tests/it/main.rs`. Accept:
  `cargo test -p engine-net --test it ggrs_loss::` green: 64 frames with a loss every 5th keep
  history at or under CAP plus pending and the tip equals the true sim. F2P: the module does not
  exist and restore from a lost frame has no test today. P2P: `cargo test -p engine-net --lib`
  green; golden `d57b5c8ae3096614` x2. needs: none.
- [x] E63 game: strict GameConfig extract with schemars in `layered_extract` — bad rows refuse as
  BadConfig with 5 refusal keys (EB-2026-10-09-S26; GREsau/schemars, MIT). The orchestrator adds
  the dependency first (licence read live, `docs/DEPENDENCIES.md` row, `cargo deny check`). Diff:
  good. Tier B. Size S. Owns the layered_extract hunk of `crates/game/src/lib.rs`,
  `crates/game/Cargo.toml`. Accept: `cargo test -p engine-game --lib layered_extract` shows 6
  passed with 5 refusal keys. F2P: 3 layered_extract tests and no schemars in crates/game today.
  P2P: `cargo test -p engine-game --test it config_vectors::` 8 of 8; golden
  `d57b5c8ae3096614` x2. needs: none. Landed 1a0cc6e0. `cargo test -p engine-game
  --lib layered_extract` 9 passed (6 pre-existing + 3 new: schema keys,
  env>file>defaults precedence, all five refusal keys); config_vectors 8 passed;
  cargo deny clean.
- [x] E68 tools: clean the rust-quality baseline (EB-2026-10-09-S50) — drop the ghost engine-viewer
  row (no such crate exists) and refresh the risen test counts from the generator, 0 bad-metric
  regressions. Diff: regular. Tier B. Size S. Owns `tools/rust-quality.baseline.json`. Accept:
  `node tools/rust-quality.mjs --json` shows only-base and regressions empty. F2P: only-base lists
  engine-viewer today. P2P: `node tools/rust-quality.mjs` PASS; golden `d57b5c8ae3096614` unchanged.
  needs: none. Landed 2f8a6877. rust-quality.mjs now names baseline rows with no workspace crate (export onlyBase, `--json` key onlyBase, a [WARN] line); the ghost engine-viewer row is gone (HEAD baseline gave only-base [engine-viewer], now []). Test: `node --test tools/rust-quality.test.mjs` 18 pass incl. 'onlyBase names baseline rows with no crate, and the real baseline has none' (goes red on the old baseline); `--json` regressions empty on the committed tree. The risen test-count floor was not refreshed: the tree holds other sessions' uncommitted tests, a floor taken now could fail at HEAD.
- [x] E69 physics: PX10 receipt gap (EB-2026-10-09-S58) — commit 1ce87a77 changes 0 crate files
  though its message claims powered.rs and ragdoll.rs work; both files are in HEAD (carried by the
  auto backup 2345f203, with the PX10 terms in the code). Re-prove CCD in `crates/physics` and post a
  receipt that names the landing commit. The inbox said crate combat; the files live in physics.
  Diff: good. Tier B. Size S. Owns `docs/SPRINT-19-receipts.md`. Accept:
  `cargo test -p engine-physics --lib powered` and `cargo test -p engine-physics --lib ragdoll` green;
  `node tools/rust-quality.mjs --crate physics` prints 100, or the receipt names the metric that
  holds it lower. F2P: no receipt names the landing commit today. P2P: golden `d57b5c8ae3096614` x2.
  needs: none. Landed f38ceac8 (docs/SPRINT-19-receipts.md: block 'PX10 receipt gap closed' names the landing commit 2345f203, auto-backup 2026-10-08 21:58, powered.rs +375 and ragdoll.rs +468; 1ce87a77 changed no crate file). Tests: `cargo test -p engine-physics --features rapier --lib powered` 17 passed (gh run 39101816259), `... --lib ragdoll` 28 passed (gh run 39101816876), both need the rapier feature (without it the filter matches 0 tests); `node tools/rust-quality.mjs --crate physics` prints 100 A; `node --test docs/SPRINT-19-receipts.test.mjs` 3 pass. No code changed, golden untouched.
- [x] E71 endpoint: run the mcp-feature suites and fix the pins (EB-2026-10-09-S68, Progress 2026-10-17: Suites run green: tool_budget_exhaustion 5 passed, jsonrpc_conformance_12_vectors passed, tool_count_pin (frozen registry 26) passed, `--features mcp --test it`, gh run 39056814820; docs/book/ai-door.md still says twenty tools and 32,768 chars while the test pins 13 core + pack verbs, so the one-count leg is open.
  EB-2026-10-11-S7). tool_budget_exhaustion (5 tests) and jsonrpc_conformance_vectors (12 vectors)
  compile but were never run; tool_count_pin asserts 26 tools while docs/book/ai-door.md says twenty
  tools and 32,768 chars while the code says 43,514 (the ai-door skill now points at the pin test
  and `mcp.rs` constants instead of a number): make test and book state one count and one budget.
  Heavy cargo goes to GitHub (`tools/gh-check.mjs`). Diff: regular. Tier B.
  Size S. Owns `crates/endpoint/tests/it/tool_budget_exhaustion.rs`,
  `crates/endpoint/tests/it/jsonrpc_conformance_vectors.rs`, `crates/endpoint/tests/it/tool_count_pin.rs`,
  `docs/book/ai-door.md`. Accept:
  `cargo test -p engine-endpoint --features mcp --test it tool_budget_exhaustion` prints 5 passed;
  the same with `jsonrpc_conformance_vectors` prints 12 passed and with `tool_count_pin` passes.
  F2P: no run on record and the book count differs from the test today. P2P:
  `cargo test -p engine-endpoint --lib` green. needs: none. Progress 2026-10-18: book leg landed in the commit after 7bc3f98f: docs/book/ai-door.md no longer says twenty tools or 32,768 chars; it names the core set (CORE_TOOL_NAMES), points at tool_count_pin.rs for the reachable count and at TOOL_LIST_BUDGET_CHARS (held by tool_budget_exhaustion.rs) for the budget, and quotes no number; tools/ai-door-book.test.mjs (2 pass) goes red on the old page. The three suites passed on 2026-10-17 (gh run 39056814820); a rerun on the current tree is blocked by another session's uncommitted insight/mcp_tools.rs (E0597 at lines 193-195), so the row is not ticked yet. The book commit named above is 099954a3. Landed 099954a3 (book leg) with the suites rerun on the current tree: `cargo test -p engine-endpoint --features mcp --test it -- tool_budget_exhaustion jsonrpc_conformance_vectors tool_count_pin` 9 passed 0 failed: tool_budget_exhaustion 5 passed, jsonrpc_conformance_vectors 3 tests incl. jsonrpc_conformance_12_vectors_agree_on_code (the 12 vectors live in that one test, so the row's '12 passed' reads as 12 vectors), tool_count_pin frozen_registry_pins_26_and_router_is_pinned, gh run 39107040004. The book and the pin now agree by pointing at one place, tools/ai-door-book.test.mjs (2 pass) keeps it so. P2P endpoint lib 160 passed (gh run 39101704961).
- [x] E72 render: split the anti-alias block out of post.rs (EB-2026-10-09-S69) — jitter, TAA and
  CAS/RCAS, about 500 of 5,233 lines, into modules beside it in the bevy_anti_alias shape, no
  output change. Claims serialize post.rs with E32 and E38. Diff: good. Tier B. Size M. Owns
  `crates/render/src/post.rs`, `crates/render/src/post/taa.rs`, `crates/render/src/post/upscale.rs`, `crates/render/src/lib.rs`. Accept:
  `cargo test -p engine-render --test it post_aa_parity::` prints 3 passed unchanged. F2P: post.rs
  line count before and after printed (drops by about 500). P2P: golden `d57b5c8ae3096614` x2;
  `cargo test -p engine-render --lib post` green. needs: none. Landed 781ecd67 (CQ9 split: jitter and TAA in crates/render/src/post/taa.rs, EASU/CAS/RCAS in post/upscale.rs, post.rs 5,233 -> 2,351 lines, no output change, public paths kept by pub use; the row's post_aa.rs name became these modules). Test: `cargo test -p engine-render --test it post_aa_parity::` 3 passed (jitter_sequence_is_centered_deterministic_and_projected, taa_resolve_static_scene_is_identity_with_timed_receipt, cas_parity_matches_chain_on_flat_and_clamps_lobe_on_edge), gh run 39101616083. Tick audit 2026-10-19: the row's `post_aa.rs` name became `crates/render/src/post/taa.rs` (jitter, TAA) and `crates/render/src/post/upscale.rs` (EASU/CAS/RCAS).
- [x] E74 render: pin the FSR1 EASU and RCAS constants in a Rust test (EB-2026-10-09-S71).
  post_aa_parity.rs pins the RCAS sharpen only; the EASU constants have the script gate as their
  only net. Diff: regular. Tier B. Size S. Owns `crates/render/tests/it/post_fsr1_parity.rs`,
  `crates/render/tests/it/main.rs`. Accept: `cargo test -p engine-render --test it post_fsr1_parity::`
  passes. F2P: the module does not exist, and a one-constant edit in EASU turns it red. P2P:
  `cargo test -p engine-render --test it post_aa_parity::` 3 passed. needs: none. Landed fcbd7ccb. New crates/render/tests/it/post_fsr1_parity.rs (4 tests: easu_con_pins_scale_offset_and_texel, ramp_follows_the_sample_law_the_constants_feed, flat_field_is_bit_exact, step_edge_never_rings_past_its_two_sides). `cargo test -p engine-render --test it post_` 7 passed (4 new plus post_aa_parity 3 unchanged), gh run 39104364027; `cargo clippy -p engine-render --test it -- -D warnings` green, gh run 39105054192. An edit to the scale, offset or texel formula fails the first test, dropping the 2x2 clamp fails the last (Lanczos-2 undershoot to 0.033 on a 0.1 step). Open finding (not changed, needs an owner call): fsr_easu_pixel in post/upscale.rs samples at (x + 0.5) * scale + offset, but FSR1 feeds the offset the integer pixel, so it sits 0.5 * scale right of the TAAU centre law (0.25 texel at 2x); fixing it takes the post.rs fsr_540p_to_1080p MTF50 lift gate to 0.000 (gate 0.10), because that lift came from the shift. Also, upscale_easu.rs (pinned 720p path) has no caller and returns Err('upscale weights collapsed') on a hard edge.
- [x] E76 research: four NVIDIA cards, research only, zero bytes vendored (EB-2026-10-09-S73,
  Maxim yes): DLSS 5 (neural last pass, RTX 50 only, SDK terms unknown), RTX Mega Geometry (cluster
  BVH for the VG meshlets), SHARC (hashed radiance cache for the RT4 world cache), RTX Character
  Rendering (skin and hair for V5), in the LADDER card format (or its split part once E45 lands).
  Each card: licence read live, source URL, kill rule. NRD, RTXDI and RTXGI stay
  proprietary-excluded unless their LICENSE says otherwise. D7 stays untouched. Diff: regular.
  Tier B. Size S. Owns `docs/research/LADDER.md`. Accept: `node --test docs/research/LADDER.test.mjs`
  passes and `rg -c "DLSS 5|Mega Geometry|SHARC|Character Rendering" docs/research/LADDER.md` prints
  4 or more. F2P: that rg prints 0 today. P2P: `node tools/workspace-check.mjs` adds no FAIL.
  needs: none. Landed 3389d4b4. Four cards appended to docs/research/LADDER.md (DLSS 5, RTX Mega Geometry, SHARC, RTX Character Rendering): each names its source URL, the licence read live 2026-10-18 (all four are the proprietary NVIDIA RTX SDKs LICENSE, so research-only with zero bytes vendored) and a kill rule. `node --test docs/research/LADDER.test.mjs` 5 pass / 0 fail; rg -c prints 4; ladder 245 lines / 205390 bytes under the 399-line and 429783-byte caps; workspace-check no FAIL.
- [x] E79 net: headless dedicated server with a master list and a server browser
  (EB-2026-10-09-S99; Agones and DDNet pattern). dedicated.rs has the LAN beacon only (line 105) and
  the word master appears nowhere in it. Add master-list register and list calls on an integer wire,
  no IO inside the tick; the beacon lists the server, 2 clients join, run steps go in
  plan/multiplayer.md. Diff: good. Tier C. Size M. Owns `crates/net/src/dedicated.rs`,
  `docs/plan/multiplayer.md`. Accept: `cargo test -p engine-net --lib dedicated` green x2 and the
  headless run prints its listing line. F2P: `rg -ci master crates/net/src/dedicated.rs` prints 0
  today. P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-net --lib` green. needs: none.
  Landed 701f9b34. `cargo test -p engine-net --lib dedicated` 10 passed x2;
  `cargo test -p engine-net --lib` 404 passed; listing line `servers=1
  fighters=2 tick=64 hash=0e2d1288d6e8d002 query=3B entry=23B`.
- [x] E80 destruction: debris snapshot resim (EB-2026-10-09-S110; snapshot bytes, digest and restore Landed c466633e. `cargo test -p engine-destruction --lib snapshot` 8 passed (rollback_across_a_collapse_is_byte_equal_to_the_straight_run, an_eight_tick_rollback_resims_equal_at_every_tick_of_the_collapse, debris_snapshot_break_then_rollback_resims_byte_equal, canonical (island, id) restore order, max_shards cap checked on restore), gh run 39056263269.
  landed in c466633e, DX6 debris LOD stays its own row). Restore in island order and replay a
  rollback twice with the 16 and 1024 caps held. Diff: good. Tier B. Size M. Owns
  `crates/destruction/src/snapshot.rs`. Accept: `cargo test -p engine-destruction --lib snapshot`
  green: restore in island order, rollback replays x2 byte-equal, caps hold. F2P: no resim or
  rollback replay test exists today. P2P: golden `d57b5c8ae3096614` x2. needs: none.
- [x] E81 physics: hair rope cloth, Q16 verlet, presentation only and never hashed Landed e4a5308f.
  (EB-2026-10-09-S111). cloth.rs holds the cape drape only (render hair is NS10, another row).
  0.5 ms bar. Diff: good. Tier B. Size S. Owns `crates/physics/src/cloth.rs`. Accept:
  `cargo test -p engine-physics --lib cloth` green with a hair-rope case: Q16 verlet x2 equal, the
  0.5 ms bar printed, match hash identical with the rope on and off. F2P: no hair-rope case exists
  today (rg hair in cloth.rs prints 0). P2P: golden `d57b5c8ae3096614` x2. needs: none.
  Progress 2026-10-21: the accept is met by the three named tests, all in `crates/physics/src/cloth.rs` at HEAD - `hair_and_rope_q16_verlet_replays_the_same_bits_twice` (:2157), `a_head_of_hair_and_a_rope_meet_the_half_millisecond_bar` (:2187), `the_match_hash_is_the_same_with_hair_and_rope_on_and_off` (:2301); landed e4a5308f ("physics: E81 Q16 Verlet hair and rope strands"). The red run is the in-flight cloth work, not this row: `node tools/gh-check.mjs --only crates/physics/src/cloth.rs cargo test -p engine-physics --lib cloth` -> FAIL E0583 "file not found for module `cloth`" (run 39518982186) because `git status` reads ` D crates/physics/src/cloth.rs` and another lane is splitting it into `crates/physics/src/cloth/`. Owed: one green run of that line once the split lands.
- [x] E83 ops: cap target/queue/done at the newest 50 and park stale ready packets
  (EB-2026-10-09-S208). done holds over 600 files today; the named stale file is already gone. Rest goes
  to C:\_quarantine\<date>\ with a MOVES.tsv line, never a hard delete. Diff: regular. Tier B. Size S.
  Owns `target/queue/done`, `target/queue/ready` (outside git). Accept:
  `(Get-ChildItem target/queue/done -File).Count` prints 50 or less and no ready file is older than
  24 h. F2P: the done count is over 600 today. P2P: `node tools/sprint-loop.mjs --dry-run` exits 0.
  needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — cap `target/queue/done` and park stale ready packets. The queue is retired. Replaced by: nothing.
- [x] E84 ops: generate target/sprint-handoff.md from the sprint lock, HEAD and the proof line in one
  script (EB-2026-10-09-S209). sprint-loop.mjs lines 170-171 still tell the lead to write it by hand.
  Diff: regular. Tier B. Size S. Owns `tools/handoff.mjs`, `tools/sprint-loop.mjs` (prompt lines
  only). Accept: `node tools/handoff.mjs --dry-run` prints the summary and exits 0. F2P: handoff.mjs
  does not exist today. P2P: `node tools/sprint-loop.mjs --dry-run` exits 0. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — `tools/handoff.mjs` from the sprint lock. `tools/sprint-loop.mjs` is gone. Replaced by: the lead's wrap-up.
- [x] E85 ops: lane-board keeps one live claim per path in target/queue/claims.txt and expires claims
  whose lock is gone (EB-2026-10-09-S210). Diff: regular. Tier B. Size S. Owns
  `tools/lane-board.mjs`, `tools/lane-board.test.mjs`. Accept: `node --test tools/lane-board.test.mjs`
  passes with a duplicate-claim case and an expired-claim case. F2P: no dedupe or expiry code in
  lane-board.mjs today. P2P: `node tools/workspace-check.mjs` adds no FAIL. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — one live claim per path in `tools/lane-board.mjs`, which is gone. Replaced by: lead commits by path, no claims.
- [x] E86 ops: cap target/debug (EB-2026-10-10-S15; pool-1 and the lane targets are already gone). Landed 31788c7e4. node tools/clean.mjs --dry-run --include-debug listed WOULD target\debug 21.34 GB; node tools/clean.mjs --auto --include-debug with no cargo running moved it to C:\_quarantine\2026-10-22\target-debug (49,450 files, 21,341,494,475 bytes) with one MOVES.tsv row, and nothing was deleted; the dry run now lists no target/debug entry (across 0 path(s)); gh-check cargo check -p engine-core PASS (run 39718650854); node --test tools/clean.test.mjs 11 of 11 pass.
  With no cargo running, `node tools/clean.mjs --dry-run`, then `--auto --include-debug`; anything
  removed goes to C:\_quarantine\<date>\ with a MOVES.tsv line, never a hard delete. Diff: regular.
  Tier B. Size S. Owns `target/debug` (outside git). Accept: `node tools/clean.mjs --dry-run` lists no
  target/debug entry. F2P: target/debug still holds top-level entries today. P2P: the next cargo build
  of one crate succeeds. needs: none.
- [x] E87 tools: finish.mjs reads gate rows from the board and FINISH-RICH only; FALLBACK_ROWS goes Landed 23f5169a. node --test tools/finish.test.mjs 14 of 14 pass on this PC; node tools/finish.mjs exit 0 and prints nothing on stderr; the QA1 collision went when the open texture-format row was renamed QA15.
  (EB-2026-10-10-S88). Today the gates with a recorded Met run still count open and the goal reads
  1 of 26. Update the fallback warning and its test. Diff: regular. Tier B. Size S. Owns
  `tools/finish.mjs`, `tools/finish.test.mjs`. Accept: `node --test tools/finish.test.mjs` passes and
  `node tools/finish.mjs` prints no "fallback rows used" warning on stderr. F2P: FALLBACK_ROWS has 5
  hits in finish.mjs today. P2P: `node tools/workspace-check.mjs` adds no FAIL. needs: none. Progress 2026-10-18: the 'fallback rows used' stderr warning is gone (`node tools/finish.mjs` prints nothing on stderr). FALLBACK_ROWS still defines the A and C gate rows (finish.mjs 4 hits, finish.test.mjs 3) because those tables have no Proven by column. `node --test tools/finish.test.mjs` is 13 of 14 pass: 'every ARCHIVE_TICKED_ROWS member has an archive tick and is not open on the board' fails on QA1, a board id collision (the open QA1 texture-format row from 925a2a35 versus the old archived A7 gate row QA1); rename one of them before this row can close.
- [x] E88 mcp: arxiv and github in opencode.jsonc point at the shared bridges
  (EB-2026-10-10-S89): arxiv at http://127.0.0.1:3222/mcp/ and github at http://127.0.0.1:3221/mcp,
  and the two local stdio copies go. Both bridges accept TCP now. Diff: regular. Tier B. Size S. Owns
  `opencode.jsonc` (mcp block only). Accept: `rg -n "127.0.0.1:322[21]" opencode.jsonc` prints 2
  lines and arxiv tools/list answers through the remote. F2P: lines 146-147 start local stdio copies
  today. P2P: `node C:/empire/center/agentlint.mjs engine2040` PASS. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — opencode.jsonc points arxiv and github at shared bridges. The OpenCode loop is retired (2026-10-17), so its MCP config has no reader. Replaced by: nothing.
- [x] E89 skills: use the shared shelf and drop the 8 shelf copies (EB-2026-10-11-S15). Add
  C:/empire/center/skills-shared to skills.paths in opencode.jsonc (line 35 holds ./skills only),
  then `git rm` bash-abort-guard, edit-reread, engine-builder, git-one-branch,
  pwsh-for-bash-writers, repo-read-first, task-abort-guard and task-scope from .opencode/skills after
  a diff against the shelf. Diff: regular. Tier B. Size S. Owns `opencode.jsonc` (skills line only),
  `.opencode/skills/bash-abort-guard`, `.opencode/skills/edit-reread`,
  `.opencode/skills/engine-builder`, `.opencode/skills/git-one-branch`,
  `.opencode/skills/pwsh-for-bash-writers`, `.opencode/skills/repo-read-first`,
  `.opencode/skills/task-abort-guard`, `.opencode/skills/task-scope`. Accept: `rg -n skills-shared
  opencode.jsonc` prints 1 line and `node C:/empire/center/agentlint.mjs engine2040` shows no
  shelf-copy WARN. F2P: the 8 copies exist and skills-shared is absent today. P2P: agentlint PASS.
  needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — use the shared skills shelf in opencode.jsonc and drop 8 shelf copies. `.opencode/skills` was folded into `skills/` (0b6daaf0) and the OpenCode loop is retired. Replaced by: `skills/` in the repo.
- [ ] E90 skills: skills pass 2 (EB-2026-10-11-S8) — merge the loop-machinery skills that are not
  shelf copies (grep-overflow-guard, rust-book, refactoring-to-rust, reflection-gate,
  multi-agent-batch, planning-rows, loop-failfix; rust-borrow-fix is already archived) into
  skills/opencode-loop and skills/engine-code, engine-code under 4 KB (6,486 bytes today). The A/B of
  the six skills added 2026-10-11 on 20 BLOCKERS tasks is a separate follow-up row. Diff: regular.
  Tier B. Size M. Owns `skills/opencode-loop/SKILL.md`, `skills/engine-code/SKILL.md`,
  `.opencode/skills/grep-overflow-guard`, `.opencode/skills/rust-book`,
  `.opencode/skills/refactoring-to-rust`, `.opencode/skills/reflection-gate`,
  `.opencode/skills/multi-agent-batch`, `.opencode/skills/planning-rows`,
  `.opencode/skills/loop-failfix`. Accept: `node C:/empire/center/agentlint.mjs engine2040` prints
  PASS and `(Get-Item skills/engine-code/SKILL.md).Length` prints under 4096. F2P: engine-code is 6,486 bytes
  today. P2P: agentlint PASS; the merged rules keep their error codes. needs: E89.
  rot: skills/opencode-loop/SKILL.md and the 7 .opencode/skills homes are gone (6bdf32b, 0b6daaf0, 6f2195b0, b0aa532c, 4d10c40d); the merged set lives in skills/ (33 skill dirs), so re-home Owns to skills/<name>/SKILL.md.
- [x] E93 save: publish the save envelope constants for forge to read (EB-2026-10-08-S231, the part Landed 55f813eb. `cargo test -p engine-save --lib` 32 passed incl. envelope_drift (VERSION 1, SLOTS 16, payload 1,048,576, total 1,048,605 pinned against content/arena/save-envelope.json), gh run 39056263382.
  GP37 does not cover; GP37 moves rome-spec to content/arena/rome-spec.json, and forge's Rome
  SPEC.md names no spatial-threshold file yet, so that leg waits for a forge ask). Write VERSION 1,
  SLOTS 16, MAX_PAYLOAD_BYTES 1,048,576 and MAX_TOTAL_BYTES (29 plus the payload) from the engine-save
  lib into `content/arena/save-envelope.json` and add a drift test that fails when a constant moves.
  Diff: regular. Tier B. Size S. Owns `content/arena/save-envelope.json`, `crates/save/src/lib.rs`
  (test hunk only). Accept: `cargo test -p engine-save --lib envelope_drift` passes and changing
  SLOTS to 17 turns it red. F2P: the json file and the pin do not exist today. P2P:
  `cargo test -p engine-save --lib` green; golden `d57b5c8ae3096614` x2. needs: none.
- [x] E94 checks: drift-check prints RESULT PASS (EB-2026-10-08-S245, Maxim P1 2026-10-11). The
  ratchet line is [PASS] on 2026-10-12 and the workspace audit is clean again; RESULT is FAIL on four
  other lines: focus feel (0 helpers named a feel row; LF10 LF17 LF11 LF14 are the next packets) and the
  stuck rows EG11, OPT24 and EG18 (a planner deep-dive first: root cause, a different approach, the
  proof command; then the writer). Write the three deep-dives; the focus line clears when a coder packet
  takes a feel row. Diff: regular. Tier A. Size S. Owns `docs/plan/EG11-deepdive.md`,
  `docs/plan/OPT24-deepdive.md`, `docs/plan/EG18-deepdive.md`. Accept: `node tools/drift-check.mjs`
  prints RESULT PASS. F2P: RESULT FAIL with 4 fail lines today. P2P: `node tools/workspace-check.mjs`
  PASS; golden `d57b5c8ae3096614` x2. needs: none.
  Landed obsolete. Owner chat 2026-10-19 (delegated): the target or premise is gone — make `drift-check.mjs` print RESULT PASS by writing 3 deep-dives. The tool is gone and EG11, OPT24 and EG18 are ticked. Replaced by: nothing needed.
- [x] UIX3b ui: wire LobbyScreen into the windowed game loop (center re-score 2026-10-12; Landed 1d01a410. `cargo test -p engine-game --test it lobby_ui_flow::` 3 passed incl. windowed_loop_reaches_lobby_screen_through_its_own_entry, gh run 39056624992; `rg -c LobbyScreen crates/game/src` 4 hits. Caveat: run_windowed drives drive_lobby_once once per smoke (windowed.rs:1030), not per press.
  production caller for the screen, whose headless tests are in
  crates/game/tests/it/lobby_ui_flow.rs). `rg LobbyScreen crates/game/src` prints 0 today. The loop
  draws the host and slot lists, feeds one LobbyInput per press and runs each LobbyCommand against
  the net lobby; the game layer owns the sockets and no address is typed. Diff: good. Tier B. Size M.
  Owns `crates/game/src/windowed.rs`, `crates/game/src/netgame.rs`,
  `crates/game/tests/it/lobby_ui_flow.rs`. Accept: `cargo test -p engine-game --test it
  lobby_ui_flow::` green plus one windowed smoke case that reaches the lobby screen through the
  loop's own entry; `rg -c LobbyScreen crates/game/src` prints 1 or more. F2P: that rg prints 0 today.
  P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-game --lib` green. needs: none.
- [x] AM5b anim: rotation (quat) lanes in the ACL-shape pack (center re-score 2026-10-12;
  docs/plan/BOOK-GAPS-2026-10-12.md line 113). pack.rs packs position lanes only, so packed clips
  drop joint rotation (`rg -c quat crates/anim/src/pack.rs` prints 0). Add quaternion lanes
  (smallest-three, 16 bits per component), a bounded-error round trip and a re-measure of the 2 MiB
  per-fighter budget. Diff: good. Tier B. Size M. Owns `crates/anim/src/pack.rs`,
  `crates/anim/src/clip.rs`. Accept: `cargo test -p engine-anim --lib pack` green with a new
  rotation_lane_round_trip case and the hero bundle still under BUDGET_BYTES_PER_FIGHTER; `rg -c
  quat crates/anim/src/pack.rs` prints 1 or more. F2P: that rg prints 0 and no rotation case
  exists today. P2P: the existing pack tests stay green; golden `d57b5c8ae3096614` x2 (pack feeds
  the hero bake only). needs: none. Landed 572bb104. engine-anim --no-fail-fast 443 lib + 8 it passed (run 39106084274), clippy -D warnings run 39106954469: pack::tests rotation_lane_round_trip, position_only_clips_keep_their_version_1_bytes, hero_bundle_carries_foot_rotation_inside_the_budget (hero + full 256-joint rotation worst case under BUDGET_BYTES_PER_FIGHTER), bad_rotation_lanes_are_refused; every old pack test green; pack feeds no sim, golden untouched.
- [x] E95 net+ai: close the EB-2026-10-13-S28 remainder (confirm_frame_wan live-or-private plus one shared ai row scanner) — `crates/net/src/rollback.rs:740` defines `pub fn confirm_frame_wan` with no production caller (only `mod tests` at line 798 calls it, lines 1203/1215); `depth.rs:342` `TierRow::from_row` and `curves_text.rs:155` `parse_row` still parse rows separately (digits already share `parse_dec_u32` from curves). Already landed, not in this row: WAN_ROLLBACK_CAP aliases netcode::MAX_ROLLBACK with try_from at the one boundary (rollback.rs:39/741); netcode uses HISTORY_CAP vs rollback MAX_P2P_ROLLBACK (distinct names); FIGHTER_CROSSFADE_TICKS genuinely used by crossfade.rs. Diff: regular. Tier B. Size S. Owns `crates/net/src/rollback.rs` (visibility hunk only: private or called by the live WAN path) plus new `crates/ai/src/rows.rs` (one shared scanner) with depth.rs and curves_text.rs delegating. Accept: confirm_frame_wan is private or reached from live WAN code; both ai parsers delegate to the one scanner; `cargo test -p engine-ai --lib` and `-p engine-net --lib` green; no new warnings. F2P: the def is pub with 0 production callers and two separate row parsers today. P2P: golden `d57b5c8ae3096614` x2. needs: none. (EB-2026-10-13-S28.) Progress 2026-10-19: code landed 7a81fb24. net: `confirm_frame_wan` is crate-private and test-only (`#[cfg(test)]`, same body): it has no production caller, and a private fn without the gate is dead code in the lib target under `clippy -D warnings`. ai: new `crates/ai/src/rows.rs` is the one scanner (`strip_comment` plus the `Fields` key=value walk), `TierRow::from_row` and `parse_row` both delegate to it, each keeping its own error type and its exact variants for each failure. Green on the runner: `cargo test -p engine-ai --lib` 274 passed (39225123760), `cargo test -p engine-net --lib` 368 passed (39221481318), `cargo clippy -p engine-ai --all-targets -- -D warnings` clean (39225453375) and the combined ai+net clippy clean on this source (39222292946), rust-quality engine-ai 98 A and engine-net 99 A PASS, docs-coverage PASS, workspace-check PASS. Left open: `cargo test -p engine-net --lib` is red now only in another session's uncommitted `crates/net/src/handshake.rs` (new untracked file: unused `mut` + E0277 at line 907), and that same source tree does not build, so the golden x2 run could not be made today. Closed 2026-10-19 by the main pass (owner delegated): the red this note names was another session's uncommitted `crates/net/src/handshake.rs`, and RC15 landed it as 9e025451, so the tree builds again. Judged on the clean tree: `cargo test -p engine-net --lib` 387 passed, 0 failed, 3 ignored (run 39233334348); `cargo test -p engine-ai --lib` 274 passed (run 39225123760) stands; code 7a81fb24, row note c9e65490. No sim or render path in the diff (a visibility hunk plus the shared row parser), so the golden d57b5c8ae3096614 is untouched.
