# Hard systems

Pre-merge text: `git show a0e13976:docs/plan/hard-systems.md`. Merged in below: `docs/archive/2026-10-10/plans/ai-door-eval.md` (section 8, verbatim) and `docs/archive/2026-10-10/plans/deepdive-HS15-2026-10-07.md` (one line in section 8). Both move to `docs/archive/2026-10-10/plans/`. The body-and-break HS2 row is a pointer to `body-and-break.md`.

Goal: published, reproducible AI-door scores beside rival scores, and the proofs that back them.

Acceptance:
1. `cargo test -p engine-playtest --lib mirror`, run twice: HS29, HS30 and HS31 are 10 tasks each, the same on mock, local and remote; HS32 aggregates the 30.
2. HS20 prints our 30-task score beside published rival numbers, or UNKNOWN with the reason.
3. HS21 refuses unscoped calls; the 20 adversarial briefs give the same digest twice.

Order: HS29, HS30, HS31, HS32 and HS20 (FOCUS Now, in that order), then HS21 and HS23 (Tier A text).

Owner crates: `playtest`, `endpoint`, `proof`.

Open board IDs: HS20 HS21 HS23 HS29 HS30 HS31 HS32 (HS2 belongs to `body-and-break.md`; the never-boarded rows of this plan are dropped and stay in git).

## 1. Where it stands

Newcomer first: the engine already has a lot of the ground; the map below says
what is real, what is a stub and what is empty.

| System | Status in code |
| --- | --- |
| Rigid physics, ragdoll, cloth (presentation) | unit-tested, integrated, ticked |
| Learned locomotion pipeline | **not started, and the two halves do not meet**: `crates/nn` is a working i8 `.e2nn` runtime and `crates/distill` trains students, but it trains *one-hot action classes* (`train.rs`: "targets are one-hot class vectors"), on the burn NdArray CPU backend with no GPU runtime. There is no locomotion teacher (no RL policy, no gait), no `crates/physics/src/learned_ctrl.rs`, and no continuous-target path in the trainer. PX5 is open and unblocked; nothing has been trained for it. |
| Behavior and planning | unit-tested, integrated |
| Crowd and nav | unit-tested |
| Netcode | unit-tested, loopback and seeded WAN soaks |
| Audio | unit-tested |
| Learned rendering (radiance cache, neural textures, splats) | missing or OPEN |
| Virtual geometry | missing (rowed) |
| Motion generation (diffusion) | missing |
| Formal verification, property tests | missing |
| Agent gym (vectorized headless envs for RL) | missing |
| Public agent benchmarks run against our door | missing |
| Data export for training any model | missing |
| Rollback-resimulate checksum gate | missing |
| Player-legal agent harness | partial: bespoke driver, no legality boundary |
| Seeded fault-injection DST | missing |
| Frozen-time playtest plus input injection | missing |
| Seeded RNG behind the door | missing |

Status 2026-10-17 (rows 1 and 4 of this table are out of date): cloth was removed (`cloth.rs`, commit `3d6ee4a1`), and so were the nav crowd, ORCA, funnel, A* and HPA modules (`3ed325c8`, 2,900 lines; `engine-nav` keeps bake, cursor, flow, links, mesh, rebake and tiles). Both were deliberate dead-code cuts (rot packets `0-rot-physics-dead-r1` and `0-rot-nav-dead-r1`). NV2 stays ticked; NV6 and NV10 are open and name `funnel.rs` and `crowd.rs`. The playtest crate also lost its look modules, `agreement` and `live_probe` (`f1a4f9d7`); none of them belongs to the mirror.

## 2. Finish line

Tier A (proof that the moat is real, no new tech): HS20 (public benchmark
mirror), HS21 (auth and scopes on the door), HS23 (Kani on the integer kernels),
HS2 (cross-architecture golden matrix). Bar: 30 mirrored tasks from public
game-agent suites score on mock, local and remote providers with the same
result and a published number beside the rivals' published numbers; the door
refuses unscoped calls; 20 proof harnesses pass; the golden matches on two
architectures or the mismatch is named.

Tier B (land what the spikes prove): HS1, HS4, HS6, HS7, HS9, HS15-HS19, HS22,
HS25-HS27. Each has one spike with a baseline and a kill rule; a spike that
fails is a dated rejection, not a retry. Order (owner 2026-10-04, game data for
the autonomous factory): HS15 comes first of all hard rows and before more
graphics; DT5 (dataset provenance, board row) is its one open prerequisite and
goes right before it. The `TOP:` line of `docs/FOCUS.md` carries this order.

Tier C (only on a measured win on the RTX 3050, O5): HS3, HS5, HS8, HS10-HS14,
HS24, HS28. Rendering and generation items stay behind the look and ship gates.

CUT stays CUT (`FOCUS.md`): XR, mobile, seamless open world, world-model
rendering as the game (a learned video model never draws or decides a frame we
ship; it may only teach, test or predict outside the tick).

## 3. Target design

Newcomer first: everything that learns or looks rich sits outside the tick and
talks to it through recorded, typed events; everything inside the tick is
integers we can prove.

- **Outside-the-tick rule for AI.** Models, world models, diffusion motion and
  neural render caches run asynchronously; what they change enters as a
  tick-stamped recorded action or a baked asset. Replay never calls a model.
- **Engine as teacher.** The deterministic engine is ground truth. It exports
  action-conditioned tapes (state, action, observation, hash) so any provider,
  local or remote, can train or evaluate against it (HS15, HS27). This is what
  world-model teams want (Unreal-based pretraining pipelines already exist,
  arXiv 2609.03557) and nobody hands out with replay-exact labels.
- **Engine as gym.** Vectorized headless environments with seeds, a typed
  observation from the spatial brief, and a typed action set; no Python
  dependency in the engine, a JSON-RPC and C-ABI door for any trainer (HS27).
- **One door, one auth.** Every agent path is an MCP verb with capability
  scopes, budgets, undo and typed refusals; rivals ship an MCP door too (see
  section 4), so the door is table stakes and the moat is what stands behind it:
  verify before apply, fork play-test, replay proof, provider parity.
- **Presentation may use anything.** GPU solvers, neural caches, splats and
  diffusion motion are allowed only where they can never change a hash.
- **Bounded cost.** Each row states its budget on the reference laptop (O5).

## 4. Hard problems

Competitor cells: public facts read 2026-09-29 through web search results;
"est." marks my inference. Papers are ideas only until a spike passes; code is
ported only under the `deny.toml` allow-list with a `PORT(...)` marker.

**Competitor reality check (the vision must not overclaim).** Unreal Engine 5.8
ships an experimental MCP server in the editor (local only, no authentication
layer, per the sources in the card file). Unity ships Unity AI (open beta, May
2026) with an in-editor agent, an AI Gateway and an MCP server. Godot 4.6 made
Jolt the default 3D physics engine for new projects. Public benchmarks of agents
doing game development now exist: GameDevBench (333 Godot tasks, ICML 2026;
visual feedback lifted one frontier model from 41.1% to 52.0%), CraftBench-UE
(70 Unreal tasks with deterministic build, asset and runtime checks; Blueprint
submissions that passed asset checks still failed runtime tests 42-50% of the
time), GameCraft-Bench, GameLogicBench (tick-level state assertions) and GBQA
(bug finding by LLMs, still weak on long-horizon state errors). So "an MCP
door" is no longer a moat and "no benchmark" is no longer true. Our edge that
survives this check: determinism (replay-exact tapes), spatial checks and
counterfactual forks before apply, provider parity as a published number, a
door that refuses by default, and the same tasks scored on our engine.

| Problem | Technique to test | Why | Reference (source ID) |
|---|---|---|---|
| Proving our math | bounded model checking of pure kernels | Kani found 11 bugs missed by tests and fuzzing in industrial Rust | Kani arXiv 2607.01504 |
| Player-legality boundary for agents | PlanCommands with compile-time contexts plus replay-from-input-log fidelity | bespoke driver can reach anywhere, so a pass may not be player-legal | `LeadFreeCandy/bevy_playtest` harness shape |
| Numbers the model cannot invent | seeded RNG plus all state behind door tools, second harness on the same contract | a second harness and a dependency-audit test are missing | `rc-ventura/fantasy-gamebook-engine` rule shape |
| Guards the model cannot mislabel | explicit title plus destructiveHint plus exact-set guard test on all 20 tools | omitted hints default to destructive and over-claimed readOnly slips through | `CoplayDev/unity-mcp` annotation shape (`Server/src/services/tools/manage_script.py` plus `Server/tests/test_tool_annotations.py`) |
| Scopes the door cannot bypass | readOnly plus allow-list filter with always/never approval split before dispatch | capability OpMask alone has no readOnly filter and no named-tool list | `64bit/async-openai` filter shape (`async-openai/src/types/mcp/mcp_.rs`) |

## 6. Slim

- No new agent roles; a specialty is a skill the role loads.
- HS20 reuses the EV1 and EA1 harness; HS21 extends MCP11; HS23 extends the proof
  crate; HS27 reuses the spatial brief as the observation. No mirrored types.
- Not built: a second physics backend, a learned renderer of the game, a world
  model in the tick, a Python dependency, a model account as a scene dependency.

## 7. Board tasks

Ticked rows are dropped; rows never boarded are not listed. Now rows are a checklist with the Accept line verbatim; the rest are a table.

Status 2026-10-17: disk shows this rule broken by the two rows below, HS33 at `:148` and HS34 at `:149` (the rows moved down by this line) - neither holds a board row: `node tools/board-slice.mjs --find HS34` prints `0 row(s)`, `--find HS33` returns only DX17 (whose text cites HS33), and `HS34` appears in `docs/*.md` plus `docs/plan/*.md` only at `docs/plan/ATLAS.md:36` (the health cell) and `docs/plan/hard-systems.md:149` (the row itself), with one history hit for HS33 at `docs/SPRINT-08-hard-stuff.md:290` rather than a row. The generated health cell spells it out: `docs/plan/ATLAS.md:36` reads `2 section-7 row(s) off the board (HS33, HS34)`. The rest of the section still matches the disk: the seven ids at `:16` are open on the board today (HS20, HS21, HS23, HS29, HS30, HS31, HS32), and every Accept filter resolves - `crates/playtest/src/lib.rs:47-51` declares `mirror`, `mirror_craft`, `mirror_gdb` and `mirror_logic` with 6, 8, 6 and 15 `#[test]` fns - while HS34's own path `crates/playtest/src/mirror_partial.rs` is absent (Test-Path False), so its F2P line holds. The rows stay; this line stays wrong on disk until the board takes HS33 and HS34 or the plan drops them.

Status 2026-10-17 (board vs the checkboxes below): the board has HS20, HS21, HS29, HS30 and HS31 ticked (landed `302e61b7`, `6553e755`, `e180771f`, `1ab69225`, `7727011e`); HS23 and HS32 are open. The `[ ]` boxes below are older than the board. Checked on the GitHub runner today: `cargo test -p engine-playtest --lib mirror` 49 passed (run 39025499138); `mirror_gdb` 6, `mirror_craft` 8 and `mirror_logic` 15 passed in their own runs. So HS32's Accept command is green while its row is still open. HS21 was not re-run today. HS23 has no harness yet: there is no `kani::proof` anywhere in `crates/`.

- [ ] HS20 public benchmark mirror. Accept: same results on mock, local, remote; UNKNOWN stays UNKNOWN for rivals we did not run; fills gap G2 and G6. needs: EV1.
- [ ] HS21 capability scopes and auth on the door. Accept: an unscoped call is refused with a typed reason; 20 adversarial briefs, 0 legitimate false refusals. needs: MCP11.
- [ ] HS23 Kani harnesses on the integer kernels. Accept: 20 harnesses pass in CI; each names its kernel and bound; a found bug becomes a test row. needs: BX1.
- [ ] HS29 mirror GameDevBench slice. Accept: `cargo test -p engine-playtest --lib mirror_gdb` green with the 10-task report reproducing x2; same results on mock, local, remote; golden `d57b5c8ae3096614` unchanged. needs: EV1.
- [ ] HS30 mirror CraftBench-UE slice. Accept: `cargo test -p engine-playtest --lib mirror_craft` green with the 10-task report reproducing x2; same results on mock, local, remote; golden `d57b5c8ae3096614` unchanged. needs: EV1.
- [ ] HS31 mirror GameLogicBench slice. Accept: `cargo test -p engine-playtest --lib mirror_logic` green with the 10-task report reproducing x2; same results on mock, local, remote; golden `d57b5c8ae3096614` unchanged. needs: EV1.
- [ ] HS32 mirror aggregate report. Accept: `cargo test -p engine-playtest --lib mirror` green with the 30-task report reproducing x2; fills VISION gaps G2 and G6; golden `d57b5c8ae3096614` unchanged. needs: HS29, HS30, HS31.
- [ ] HS33 proof: Windows golden plus BX28 corpus x2 receipt, first HS2 leg on Windows x86-64 only. Tier A. Size S. Owns evidence under `target/hs2-windows/` plus runner hunks beside `crates/proof` (no sim change; HS2 keeps the CI files). Accept: golden `d57b5c8ae3096614` x2 plus the BX28 corpus digest x2 posted on Windows x86-64, or the first divergent tick named with its cause; `node tools/proof.mjs` PASS. F2P: no x2 receipt under `target/hs2-windows/` today. P2P: golden `d57b5c8ae3096614` unchanged. needs: PX17.
- [ ] HS34 playtest: HS29-first partial aggregate dry-run, reads whatever of the HS29 HS30 HS31 reports exist (at least HS29) and posts a partial mirror report x2. Tier A. Size S. Owns `crates/playtest/src/mirror_partial.rs` (new, aggregator only; slices stay in their files and HS32 keeps `mirror.rs`). Accept: `cargo test -p engine-playtest --lib mirror_partial` green with the partial report reproducing x2; golden `d57b5c8ae3096614` unchanged. F2P: no `mirror_partial.rs` today. P2P: golden `d57b5c8ae3096614` unchanged. needs: HS29.

### Board rows

HS2 (cross-architecture golden matrix) is a pointer: its row and plan text live in `body-and-break.md` (Bodies).

## 8. Quality gates

- Golden hash `d57b5c8ae3096614` unchanged; every HS row that touches the tick
  adds its own tape, hash x2 and snapshot-restore equality.
- Presentation code never enters checkpoint types (grep gate).
- A spike ships with baseline, number, kill rule and a dated verdict.
- Provider parity (mock, local, remote, external MCP client) on HS20, HS21, HS27.
- Licenses read live before any weight, dataset or code is used; UNVERIFIED
  blocks adoption.
- HS33-HS40 merge gates: test-only seams, no sim change; HS37 takes no donor
  code (UNVERIFIED ideas-only); HS36 plus HS38 hold the tool-list budget headroom;
  HS33-HS35 plus HS39-HS40 kill on any same-seed divergence with numbers.
- No claim about a rival without a source and a date; UNKNOWN stays UNKNOWN.

### 20 adversarial briefs (docs/archive/2026-10-10/plans/ai-door-eval.md, verbatim)

#### Bar

Same brief run twice gives the same digest and the same approval hash.
Different brief never reuses another brief's idempotency key.
Fail = any digest or hash mismatch, or any cross-brief key reuse.

1. place box next_to wall:north twice, same key gives same hash
2. same as 1 with a different key, expect conflict not silent pass
3. move crate then undo, expect digest back to start
4. delete parent, children relink, repeat gives same digest
5. reparent fighter under two parents, same end pose both runs
6. stale expected_revision rejected, retry on fresh revision passes
7. duplicate tx.apply with same key writes nothing the second time
8. expired approval rejected, fresh preview passes
9. overlapping placements conflict instead of stacking
10. out-of-bounds placement rejected with reason
11. blocked path placement rejected with reason
12. occluded watcher placement flagged, digest still stable
13. rapid identical previews converge to one approval hash
14. interleaved briefs keep per-brief digests stable
15. undo with wrong token rejected, newest token passes
16. remove missing node rejected, digest unchanged
17. set_local twice to same pose is idempotent
18. insert with used id rejected, fresh id passes
19. mixed insert/remove batch replays to same digest
20. full 20-brief loop twice, every digest matches run one

#### Done-when

- Test-Path docs/archive/2026-10-10/plans/ai-door-eval.md is True.
- line count under 60 via Measure-Object -Line.
- node tools/board-slice.mjs --find ai-door-eval names the follow-up row.
- skipped: none. Every brief runs headless with fixed seed.

- HS15 is ticked. Its deep-dive (2026-10-07) is archived under `docs/archive/2026-10-10/plans/`; the proof is `cargo test --locked -p engine-proof --lib dataset`, green x2 on one tree with golden `d57b5c8ae3096614` unchanged.

