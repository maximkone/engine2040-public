# Fix-docs-rot — the false claims in our docs, corrected 2026-10-21

Agent: the rot-fix session. Work list: `docs/research/powerups-2026-10-20/00-CORRECTIONS.md`
(lead's file, read-only here). Nothing was committed, staged or pushed; the lead commits by path.

Every truth in the table below was re-checked in the tree on 2026-10-21 with `git grep`,
`Test-Path`, `Get-Content | Measure-Object -Line`, a `node` script over `Cargo.lock`, or
`node tools/board-slice.mjs <ID>`. The proof column names that path and line.

Style rule followed everywhere: **one short dated line per fix, in the file's own style**. No
audit, section or frozen denominator was rewritten. Where a row's original text was false, the
original text stays and a dated correction sits beside it.

## Task 1 — the false claims named in 00-CORRECTIONS.md

| # | file | the false claim | the truth, with proof | the fix |
|---|---|---|---|---|
| 1 | `docs/plan/forge-adoption.md` | A.3 #5 says there is no outside caller for `nav::crowd` (ORCA), `assets/import_repair`, and (via A.3 #3) that `engine-playtest` has no consumer | There is no `nav::crowd` module: `crates/nav/src/` holds 14 files and no `crowd.rs`; the ORCA code is `crates/nav/src/orca.rs` (971 lines on disk) with zero callers outside its own file (`git grep -rn "orca" -- crates/` hits only `crates/nav/src/lib.rs:12 pub mod orca;` and its own file); the live crowd cue is `crates/audio/src/crowd.rs` (512 lines) with six intra-crate callers plus `crates/game/src/audio_director.rs:17`. `assets/import_repair` IS wired: `crates/render/src/v1.rs:256` and `crates/render/src/present/gpu_present/presenter.rs:2213` call `engine_assets::import_repair::recompute_tangents`. `engine-playtest` HAS a consumer: `crates/endpoint/src/packs/verify.rs:34-37`, plus `crates/endpoint/src/look_host.rs:7` and `crates/endpoint/src/packs/view.rs:49`; `crates/endpoint/Cargo.toml:16,:79` declares it optional behind `verify`/`mcp`. `ai::utility` is live inside `crates/ai` (`crates/ai/src/gladiator.rs:144`, `crates/ai/src/personality.rs:155`) — the class is internal-only, not "no caller" | dated `CORRECTION 2026-10-21` line appended to A.3 #5; the audit text above it is unchanged |
| 2 | `docs/SMOOTH-BUILD.md` | the "16 syn / 12 serde_json / 14 num_traits copies" figures read as dependency counts | They were 2026-10-17 build-artifact counts in `target/debug/deps`. Parsing `Cargo.lock` (node script over the `[[package]]` blocks) gives 1,032 packages, 80 names with more than one version, 101 extra version copies; `serde_json` is one version (1.0.151), `num-traits` is one version (0.2.19) and is not a direct dependency of any crate (`git grep "num-traits" -- crates/*/Cargo.toml Cargo.toml` = 0 hits), `syn` is three (1.0.109, 2.0.119, 3.0.6) | dated bullet added to "Build speed facts", naming `Cargo.lock` at HEAD `c36c060e` as the source and the count method; the 16/12/14 origin is named |
| 3 | `docs/SPRINT-13-engine-edge.md` | the OPT8 progress note quotes the same 16/12/14 numbers and points at `docs/SMOOTH-BUILD.md` as their home | same truth as #2; the note's parenthetical was the only live carrier of the old numbers | dated `Correction 2026-10-21` line under the OPT8 row; the row id, tick state and rest of the row text are unchanged |
| 4 | `docs/research/COMPARE.md` | D-CLOTH-1 (frozen 2026-09-30) says engine2040 collision is UNKNOWN "no collider input in cloth.rs; only anchors; character colliders absent" | `crates/physics/src/cloth.rs:596` is `Cloth::step(&mut self, colliders: &[ClothCollider])` (HEAD `c36c060e`) and the one production collider is the wearer's torso capsule at `crates/game/src/fighter_look.rs:373`. The real gap is narrower: one collider, and at HEAD no self-collision and no CCD (`git show HEAD:crates/physics/src/cloth.rs` has no intercollision set and no swept cast). An in-flight lane adds `set_self_collision_set`, `resolve_self`, `sweep_colliders` and `crates/physics/src/layers.rs`; that work is uncommitted and is not counted | dated `CORRECTION 2026-10-21` bullet added beside the D-CLOTH-1 evidence line. **The frozen items, the totals and the rival cells are untouched** |
| 5 | `docs/SPRINT-18-gaps.md` (row EG24) | "no `.e2nn` is in the tree" | `fixtures/nets/depth_student.e2nn` is tracked (`git ls-files fixtures/nets/`) and in use at `crates/distill/src/duel.rs:709`, `crates/distill/src/fps.rs:179` and `crates/distill/src/bin/distill_duel.rs:54`. What EG24 really misses: the arena bind (`ArenaPolicy` exists at `crates/ai/src/depth.rs:699` and is re-exported at `crates/ai/src/lib.rs:95`, but `crates/arena` names it nowhere) and the gate test names (`infer_into_no_alloc` and `student_gate` exist nowhere; the landed names are `infer_into_allocates_nothing` at `crates/nn/src/lib.rs:1327` and `strength_gate_swaps_sides_and_repeats_exactly` at `crates/distill/src/tournament.rs:1278`) | dated `Correction 2026-10-21` line under the EG24 row; the row id, tick state and rest of the row text are unchanged |
| 6 | `docs/SPRINT-11-north-star.md` (row GN4) | owns center `kaggle.mjs` and `kaggle.test.mjs`; steps run `node kaggle.mjs run mesh ...` | `center/kaggle.mjs` and `center/kaggle.test.mjs` do not exist (`Test-Path` False) and have no git history. The new home is `C:\empire\center\make.mjs` (center repo, commit `9fda6705`): `node make.mjs models --verify` (licence gate), `node make.mjs run <kind> --spec <file>` (prints `started run <id>`), `node make.mjs job <id>` (poll), `node make.mjs selftest` (no network); kinds `mesh`, `parts`, `texture`, `motion`, `sound` (`make.mjs:67`) | dated `Correction 2026-10-21` line in the row; row id, tick state and rest of the row text are unchanged |
| 7 | `docs/SPRINT-11-north-star.md` (row GN5) | same dead center file; the `parts` kind is to be added to its `JOBS` table | same new home; `parts` is already one of the accepted kinds (`make.mjs:67 KINDS`) | dated `Correction 2026-10-21` line in the row; rest of the row text unchanged |
| 8 | `docs/SPRINT-11-north-star.md` (row GN7) | the endpoint runs `node <center>/kaggle.mjs run mesh --repo ... --detach` and `kaggle.mjs job <id>` | the same two verbs now live in `C:\empire\center\make.mjs`: `run <kind> --spec <file>` and `job <id>`; the run id is the hash of the spec (`make.mjs:141 runId`), so the same spec always names the same run | dated `Correction 2026-10-21` line in the row; rest of the row text unchanged |
| 9 | `docs/SPRINT-11-north-star.md` (row GN8) | the new centre command is `node kaggle.mjs models --verify` | that file is gone; the licence gate is `node make.mjs models --verify` in `C:\empire\center\make.mjs` (fails closed on a missing, gated or non-commercial card, exits 1 on a refusal) | dated `Correction 2026-10-21` line in the row; rest of the row text unchanged |

## Task 2 — the rot sweep over the rest of docs/

| # | file | the false or stale statement | the truth, with proof | the fix |
|---|---|---|---|---|
| 10 | `docs/plan/quality.md` | CQ24-CQ30 are listed as open rows; the CQ30 row cites `crates/render/src/post.rs:2650`, `:2806`, `:3133`; CQ24's new file is `post/jitter_motion.rs` | all seven rows are LANDED and ticked on the board (`docs/SPRINT-05-code-quality.md:41-70`; `node tools/board-slice.mjs CQ27` and `CQ30` both print `- [x]`). The files are on disk: `crates/render/src/post/{taa,upscale,chain,auto_exposure,graph,gpu}.rs` plus `crates/render/src/post_jitter_motion.rs` for CQ24 (`git grep -ln "pub fn halton"` names the flat file, not `post/jitter_motion.rs`). `post.rs` is 2,376 lines, so `:2650`/`:2806`/`:3133` do not exist; the moved symbols read `post/gpu.rs:163` (`pub struct Rp1Submitter`), `post/gpu.rs:363` (`pub fn resolve`), `post/chain.rs:923` (`pub fn run_frame`) | two dated `Status 2026-10-21` lines: one on the CQ30 row (the landed files and the new symbol homes) and one under the "Open board IDs" list (the CQ24-CQ30 ids come off it). The row bodies and the order line are otherwise unchanged |
| 11 | `docs/plan/quality.md` | item 10 says "sprint-loop refuses to commit when workspace-check exits non-zero", with the audit gate in `tools/land.mjs` | `tools/sprint-loop.mjs`, `tools/sprint-loop.test.mjs` and `tools/land.mjs` were all removed on 2026-10-17 (commit `6bdf32bf`; none of the three files exists). The refusal shape is now the by-path land rule in the root `AGENTS.md` "Proof" section plus `node tools/workspace-check.mjs`. The second half of the item (board commit `f336c06b`) still holds | dated `Status 2026-10-21` line under the 2026-10-17 status line; the item text is unchanged |
| 12 | `docs/plan/render-look.md` | the S5 status line cites `live.rs:1172`/`:1221`/`:1352`/`:1430` and `aa_scale.rs:2603` inside a `#[cfg(test)] mod tests` opened at `:1833` | the cites drifted: `crates/render/src/present/gpu_present/live.rs` is 3,191 lines and `encode_forward` is at `:1298` (then `encode_blit` at `:1358`, then `:1548`/`:1647`); `crates/render/src/aa_scale.rs` is 2,485 lines, its test mod opens at `:1232` and the only `run_frame` caller outside `post.rs` is `aa_scale.rs:2000`; `run_frame` now lives in `crates/render/src/post/chain.rs:923` (the CQ27 post split moved it). The gap itself still holds: `git grep -n encode_post -- crates/render/` = 0 hits | dated `Status 2026-10-21` line under the 2026-10-17 line; the original text stays |
| 13 | `docs/plan/animation.md` | the LM2 cell says `crates/anim/src/warp_ground.rs` "does not exist yet" | both `crates/anim/src/warp_ground.rs` and `crates/anim/src/warp_ground_feel_test.rs` exist (Test-Path True) and the live consumer is `use engine_anim::warp_ground::GroundWarp;` at `crates/arena/src/lib.rs:109` | the LM2 cell now reads "on disk, live consumer present" with the proof paths and a `corrected 2026-10-21` tag; the file's own 2026-10-12 / 2026-10-17 status lines are unchanged |
| 14 | `docs/plan/seats.md` | the plan tells agents to run `tools/land.mjs`, `tools/land.test.mjs`, `tools/queue-cap.mjs`, `tools/seat-ledger.mjs`, `tools/packet-gate.mjs` | all five were removed on 2026-10-17 with the loop (commit `6bdf32bf`; Test-Path False for each; only `tools/hooks/pre-commit` survives). `docs/README.md:31` already lists this file as a known leftover to "read as history" | dated `Status 2026-10-21` line after the land-audit paragraph; the plan text is unchanged |
| 15 | `docs/plan/EG18-deepdive.md` | `docs/BLOCKERS.md:27` holds the EG18 anim-crossfade row | `docs/BLOCKERS.md` was rewritten on 2026-10-21 (122 pre-standalone rows moved to `docs/archive/BLOCKERS-to-2026-10-17.md`); the row now reads `docs/archive/BLOCKERS-archive.md:213` | the cite carries an inline 2026-10-21 note with the new home; the row text is unchanged |
| 16 | `docs/plan/GS30-deepdive.md` | `docs/BLOCKERS.md:34` (engine-spatial link-red) and `:43` (GS31 abandoned) | the same 2026-10-21 BLOCKERS rewrite; the rows now read `docs/archive/BLOCKERS-archive.md:220` and `:229` | both cites carry an inline 2026-10-21 note with the new homes |
| 17 | `docs/plan/lf10-pose-hold-rethink.md` | `docs/BLOCKERS.md:139`, `:138-139,141`, `:108` | the same 2026-10-21 rewrite; the old line numbers no longer point at the rows (the link wall and the combat-lib row are in `docs/archive/BLOCKERS-to-2026-10-17.md`) | each of the three cites carries an inline 2026-10-21 note |
| 18 | `docs/plan/SP63-deepdive.md` | `docs/BLOCKERS.md:35`, `:46`, `:53`, `:28`, `:51`, `:54` | the same rewrite; the three named rows now read `docs/archive/BLOCKERS-to-2026-10-17.md:32` (EG11 window build), `:43` (provider-json-vs-audit) and `:50` (EG29 slice-1), and the SP63 rows `:25`, `:48`, `:51` | a 2026-10-21 line under the 2026-10-16 note gives the new homes, plus a 2026-10-21 note on the `:46` cite in section 3 |
| 19 | `docs/plan/BOOK-GAPS-2026-10-12.md` | row S3 cites `crates/render/src/shaders/forward.wgsl:830` (file has 820 lines) | the claim still holds at the current lines: `forward.wgsl:223` defines `fn sun_contact_factor(world: vec3<f32>) -> f32` and `:570` calls it, while the GTAO path is a separate sampled `ao_tex` (`:149`, ambient-only note at `:650`) | dated `Status 2026-10-21` line under the S3 statuses; the S3 cell and the frozen numbers are unchanged |
| 20 | `docs/plan/BOOK-GAPS-2026-10-12.md` | the "Window smoke boot per landing" cell cites `tools/land.mjs:89-90`; row S14 cites `tools/land.mjs:89-90` and `:113-118` | `tools/land.mjs` was removed on 2026-10-17 (commit `6bdf32bf`; Test-Path False) | dated 2026-10-21 notes on both cells; the cell text is otherwise unchanged |
| 21 | `docs/plan/2026-10-20-parallel-campaign.md` | GN4, GN5, GN7, GN8 "own or run `kaggle.mjs`" and the plan says "the center pipeline is gone" with nothing to replace it | the generator chain has a new home, `C:\empire\center\make.mjs` (center repo, commit `9fda6705`; the four verbs and kinds are in the Task 1 table) | a dated `Status 2026-10-21` line under "Before counting the 309" and a short inline 2026-10-21 note on the "Leave GN7 out" line; the counts and the rest of the plan are unchanged |
| 22 | `docs/plan/2026-10-17-obsolete-rows.md` | E4 "Replaced by: nothing in this repo" | the replacement now exists in the center repo: `C:\empire\center\make.mjs` (commit `9fda6705`); the engine half is still open as board row PW56 (`docs/SPRINT-28-powerups.md`) | dated `Correction 2026-10-21` line under the E4 bullet; the E4 text and the "closed 2026-10-19" header are unchanged |
| 23 | `docs/SPRINT-10-empire-inbox-drain.md` (row E4) | "Replaced by: nothing in this repo" | same new home; the MIT shape-model swap itself is still open as PW56 | dated `Correction 2026-10-21` line under the row's "Landed obsolete" note; the row id, tick state and rest of the row text are unchanged |
| 24 | `docs/SPRINT-09-empire.md` (row SL44) | owns hunks beside `crates/render/src/present.rs:3084` (the file has 828 lines) | the submit path now lives in `crates/render/src/present/gpu_present/submit.rs` (3,221 lines) and the `steady_120_frames_read_zero_allocs` test the row's own Progress note names is at `submit.rs:2562` | dated `Correction 2026-10-21` line under the row; row id, tick state and rest of the row text are unchanged |
| 25 | `docs/systems/README.md` | three cites point at `docs/SPRINT.md:100-104` (the file has 101 lines) for the parked SC2 / SC3 / RV1 rows | the SC1-SC3 and RV1 names sit at `docs/SPRINT.md:91-95` (line 94 names SC2 and SC3; line 93 names RV1) | the three cites now read `docs/SPRINT.md:91-95`; the rest of the file is unchanged |
| 26 | `docs/book/door-reference.md` | `asset.intake` cooks "center's kaggle mesh job" | that job ran in the retired `center/kaggle.mjs`; the new home is `C:\empire\center\make.mjs` (`run <kind> --spec <file>` / `job <id>` / `models --verify`). The `lane_json` file the runner writes is still named `kaggle-lane.json` | dated `Correction 2026-10-21` note under the `asset.intake` section; the tool table row is unchanged |
| 27 | `docs/research/steals-2026-10-12/arch.md` | item 10 cites `.cargo/config.toml:38-39` and `:54-55` for a "sccache pool wrapper", `Cargo.toml:216-291` for the profiles, and `docs/SMOOTH-BUILD.md:169` | `.cargo/config.toml` is 74 lines: the rust-lld linker is at `:50` and `:57-58`, the 121 s -> 79 s measurement is the comment at `:52-56`, and `:67-71` says plainly "No rustc wrapper. sccache is opt-in per run" - there is no pool wrapper. `Cargo.toml` starts `[profile.release]` at `:237` and ends with `[profile.web]` at `:325` (329 lines). The dev-build numbers live at `docs/SMOOTH-BUILD.md:150` (154 lines) | dated `Correction 2026-10-21` line under item 10; the item text is unchanged |
| 28 | `docs/SPRINT-19-receipts.md` | the template says new blocks append via `node tools/land.mjs <packet> --dry` | `tools/land.mjs` was removed on 2026-10-17 (commit `6bdf32bf`; Test-Path False) | dated 2026-10-21 comment line under the template; the template line itself stays |
| 29 | `docs/compaction-baseline.md` | the "Doc line that must change" section names `docs/SMOOTH-BUILD.md` and `docs/SPRINT-13-engine-edge.md:34` as still carrying the 16/12/14 numbers | both carry the dated correction now (Task 1 items #2 and #3) | dated `Status 2026-10-21` line after the quoted corrected text; the baseline numbers above are untouched |

## Fix count per file

Counted as dated lines added. Some numbered corrections above carry more than one dated line
(three BLOCKERS cites in one file, three in another), so the two numbers differ: **29 numbered
corrections, 33 dated lines, 24 files**.

| file | fixes |
|---|---|
| `docs/SPRINT-11-north-star.md` (GN4, GN5, GN7, GN8) | 4 |
| `docs/plan/quality.md` | 2 |
| `docs/plan/BOOK-GAPS-2026-10-12.md` | 2 |
| `docs/plan/2026-10-20-parallel-campaign.md` | 2 |
| `docs/plan/lf10-pose-hold-rethink.md` | 3 (three BLOCKERS cites) |
| `docs/plan/SP63-deepdive.md` | 2 (four BLOCKERS cites) |
| `docs/plan/GS30-deepdive.md` | 1 (two BLOCKERS cites) |
| `docs/systems/README.md` | 1 (three cites) |
| `docs/plan/EG18-deepdive.md` | 1 |
| `docs/plan/render-look.md` | 1 |
| `docs/plan/animation.md` | 1 |
| `docs/plan/seats.md` | 1 |
| `docs/plan/2026-10-17-obsolete-rows.md` | 1 |
| `docs/plan/forge-adoption.md` | 1 |
| `docs/SPRINT-13-engine-edge.md` | 1 |
| `docs/SPRINT-18-gaps.md` (EG24) | 1 |
| `docs/SPRINT-10-empire-inbox-drain.md` (E4) | 1 |
| `docs/SPRINT-09-empire.md` (SL44) | 1 |
| `docs/SPRINT-19-receipts.md` | 1 |
| `docs/SMOOTH-BUILD.md` | 1 |
| `docs/book/door-reference.md` | 1 |
| `docs/compaction-baseline.md` | 1 |
| `docs/research/COMPARE.md` | 1 |
| `docs/research/steals-2026-10-12/arch.md` | 1 |
| **total** | **29 numbered corrections, 33 dated lines, 24 files** |

## Found but NOT verified, so left alone

Each of these was checked and could not be proven from the tree on 2026-10-21. Per the rule
("never guess a number - if you cannot verify it, leave the sentence alone and note it"), each was
left exactly as it was.

| file | claim left alone | why it could not be verified |
|---|---|---|
| `docs/research/COMPARE.md` | the whole D-CLOTH-1 denominator: the nine frozen items, their weights, the fixture, the totals line and every rival cell (Unreal 5.8, Unity 6000.5, Godot, Flax, Bevy, O3DE) | frozen by the task; also the rival cells are source reads from 2026-09-30 that cannot be re-read from the tree. The engine2040 sentence was corrected with a dated line; the denominator and the totals are untouched |
| `docs/research/COMPARE.md` | the D-CLOTH-1 totals "engine2040 confirmed 0/9; unknown 4/9" | the frozen denominator; the correction note says the items and the totals are unchanged, and no test was run here to change a cell |
| `docs/plan/final-picture.md` | "42 library crates, about 772k lines in `crates/*/src`" and "About 700 open rows, about 300 ready" | the line count is right in kind (42 crates; 834,489 physical / 787,569 non-blank on disk today) but the file is dated "Where it stands (2026-10-20)" and the tree moved; the row counts are 782 open / 361 ready today, not 700/300. `docs/plan/final-picture.md` is being edited by another session right now (`git status` shows it dirty in their wave), so editing it would collide. Left for the owner of that file |
| `docs/plan/2026-10-20-parallel-campaign.md` | "The target is 309, not 695", "about 700 open rows, 309 startable now", "217 wait on another open row", "154 carry `LATER:`", "13 carry `BLOCKED:`" | the board moved since 2026-10-20 (782 open, 361 ready today), and the sub-counts (217 / 154 / 13) need the strict `rowReady` rule, which the tool does not print as those three numbers. The counts are a dated snapshot of a dated plan; I corrected the kaggle claim beside them, not the counts |
| `docs/compaction-baseline.md` | the whole measured baseline: 42 crates, 816,611 src lines, 771,870 non-blank, 1,032 lock packages, 80 multi-version names, 101 extra copies, 7 dead feature flags, the per-crate table, the top-20 files and the D1-D6 deletion manifest | it is a dated snapshot taken at HEAD `77789c2a` on 2026-10-20 with a drift gate that owns it (board row PW51, `docs/SPRINT-28-powerups.md`). It also holds a `Cargo.lock` snapshot; re-measuring it here would break the gate's pairing. Its line counts are from the same wave's pass, and the file says so at the top |
| `docs/compaction-baseline.md` | the D1-D6 line counts (orca.rs 971, heal.rs 1,090, audio heal.rs 749, kcc.rs 928, rerun_sink.rs 321, behavior.rs 275) | these are the wave's own pass figures on the dirty tree; on HEAD they read 910 / 1,014 / 686 / 925 / 297 / 260, and on disk right now 971 / 1,090 / 795 / 928 / 437 / 275. The file is a dated snapshot and the deletion manifest is the lead's to re-measure, not a rot fix |
| `docs/plan/quality.md` | the "Code as-is 2026-10-15" paragraph: "12 files over the 1500 bar", the `post.rs` monolith line numbers, "`present.rs` leaves still in the facade", "`camera.rs` unsplit", "`assistant/` missing `close.rs`" | it is explicitly dated 2026-10-15 and several halves have since changed (the `post/` dir now exists with six leaves; `node tools/rust-quality.mjs --list big_files` reports 13 hits today, not 12). I corrected the CQ rows above it, not this dated paragraph; a fresh "as-is" pass is the plan owner's job |
| `docs/plan/quality.md` | the per-crate OPT/CQ row list in section 7 for every row other than CQ24-CQ30 | the other rows' landed state was not checked one by one; only the CQ24-CQ30 block was verified against the board and the tree |
| `docs/plan/seats.md` | the AW1-AW29 board-task list and the "6 section-7 row(s) off the board" note in `docs/plan/ATLAS.md` | `docs/plan/ATLAS.md` is a generated file (`node tools/plan-atlas.mjs --write`, per `docs/README.md:25`) and must not be hand-edited; the AW rows themselves live on the board in `docs/SPRINT-14-agent-workspace.md`, not in this plan |
| `docs/plan/forge-adoption.md` | the rest of A.3 (items 1-4) and all twenty AD rows | the task named A.3 item 5; items 1-4 and the AD rows are the architect pass's own audit, re-opening them is a new audit, not a rot fix |
| `docs/research/COMPARE.md` | the D-CROWDS-1, D-RAGDOLL-1, D-DESTRUCT-1, D-FLUIDS-1, D-VEHICLES-1, D-RIGID-1, D-SOFT-1, D-SKEL-1 denominators and the AI-door surveys | frozen denominators; only D-CLOTH-1 was named in the work list |
| `docs/SPRINT-28-powerups.md` (row PW56) and `docs/SPRINT-28-rome.md` | the rows that name files the tree does not have yet (`crates/rome/src/*`, `tools/codex/*`, `content/rome/*`) | these are open rows for work that has not landed yet. A missing file named by an open row is the row's purpose, not rot |
| `docs/plan/2026-10-18-*.md`, `docs/plan/2026-10-19-*.md`, `docs/plan/wiring-triage-2026-10-17.md` | every claim in them | dated audit snapshots of 2026-10-17 to 2026-10-19; their status lines are already dated corrections in their own right, and re-auditing them is new work, not rot |
| `docs/research/steals-2026-10-12/*` (all other files) | the donor tables and the "WE HAVE" lists | dated steal catalogues read live on 2026-10-12 against a donor tree that is not ours; they are evidence records, not claims about our tree. Only `arch.md` item 10 made a claim about our own tree, and that one is corrected |
| `docs/research/web-2026-10-17/*`, `docs/research/camera-mods-combat-2026-10-19/*`, `docs/research/finish-line-2026-10-17/*` | every claim | dated external research reads; nothing in them claims a fact about our tree that I could check without re-reading the donors |
| `docs/archive/**` (851 files) | everything | history, no authority (`docs/README.md:19`) |
| `docs/research/powerups-2026-10-20/**` | everything | the lead's own wave folder; read-only for this session by the task's rules |

## Gates run

- `node tools/workspace-check.mjs` - the four FAILs are all code-side and belong to other
  sessions' uncommitted work (`engine-arena -> engine-nav` edge, `engine-assets/proxy.rs` and
  `engine-physfx/body.rs` undeclared modules, the GE2 genre-name ledger). No doc check failed.
- `node --test docs/INDEX.test.mjs tools/index-check.test.mjs tools/plan-index.test.mjs` - 13 pass,
  1 fail. The failure is `build-cpu.md: proof 'tooling' must be from Look, Bodies, AI door,
  Multiplayer, all four` - `docs/plan/build-cpu.md` was added by another session in commit
  `0cf84494` and is not a file this session touched.
- `node tools/board-slice.mjs CQ27` and `CQ30` - both print `- [x]`, used as the proof for the
  quality.md correction.
- `node` script over `Cargo.lock` (package blocks, name->versions) - 1,032 packages, 80
  multi-version names, 101 extra copies, `serde_json` 1.0.151, `num-traits` 0.2.19, `syn`
  1.0.109 / 2.0.119 / 3.0.6. Used for the SMOOTH-BUILD.md and SPRINT-13 corrections.
- `node` script over `crates/*/src/**/*.rs` - 1,174 files, 834,489 physical lines, 787,569
  non-blank on disk; used to judge the final-picture.md number (left alone, see above).
