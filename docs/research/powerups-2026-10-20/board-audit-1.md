# Board audit 1 — open rows in docs/SPRINT.md and SPRINT-01..08

Read-only audit by a board auditor subagent, 2026-10-20T23:38Z, HEAD `2213a825` + working tree.
Scope: every open row of `docs/SPRINT.md` and `docs/SPRINT-01-*` .. `docs/SPRINT-08-*` — **168 rows**.
No board, doc or code file was edited. Nothing was ticked. No sim program ran, so the golden
`d57b5c8ae3096614` was not measured and did not move.

Verdicts: **DONE** (its Accept command passed today), **DUP** (another row asks the same),
**WP** (the row's text is false), **REAL** (needs building).

Proof commands used, all from `C:\engine2040`:
`node tools/gh-check.mjs cargo test -p engine-spatial --lib rooms` (PASS, run 39513546216),
`node tools/gh-check.mjs cargo test -p engine-spatial --lib sees_pair` (PASS, run 39515251019),
`Test-Path` / `git grep -n` for every path and symbol cited below.

Tree state while auditing: `cargo test -p engine-endpoint --features mcp --lib query_compact`
did **not compile** — `crates/ecs/src/pool.rs:327` (`pub(crate) fn shared`, E0603 from
`crates/arena`) — another lane's in-flight edit, not this row's. The render Accept could not be
run at all: `crates/render/src/accel.rs` holds GPU/window code, so it runs on this PC and the
build directory was locked by another lane's cargo.

---

## A. ALREADY DONE — 2 rows (tick these first)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| SPR7 | **DONE** | `cargo test -p engine-spatial --lib sees_pair` → PASS on the runner: `2 passed; 0 failed; 1 ignored` (the ignored one is the release timing test), run 39515251019. Row text already says "the Accept is met, the tick waits for AD7". | Tick with the landing sha (`a2dba030` sees.rs, `755cd907` visible_from, `5cc241ed` door kind=sees). AD7 only gates readiness, not the tick. |
| SPR9 | **DONE** | `cargo test -p engine-spatial --lib rooms` → PASS on the runner: `7 passed; 0 failed`, run 39513546216 (`a_two_room_house_has_two_rooms_and_one_exit`, `fifty_seeded_scenes_match_their_hand_labels`, membership, isovist parity). | Tick with `755cd907` / `39b0e28a`. The row's own "left" (wants `inside room:<n>`, brief grouping by room) is a separate piece — an inbox line, not this Accept. |

---

## B. DUPLICATE — 4 rows (one of each pair can go)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| WIN6b (07) | **DUP of WIN6** | WIN6 is `- [x]` on the board; WIN6b's own park line: "duplicate of WIN6 (same title and accept; WIN6 holds the judge-PASS lane win6-top)". Both own `crates/game/src/windowed.rs`. | Delete the LATER line and retire WIN6b to the archive. Nothing to build. |
| SN10 (01) | **DUP of AU3** (and PB61) | All three own `crates/audio/src/coherence.rs`. SN10: "10 scenarios synced with loudness/clipping/delta per hit"; AU3: "10 scenarios synced, 0 audio-thread allocs"; PB61 (open): "no hit-synced numbers". `coherence.rs:368-478` already holds `scenario_hit/blocked/parry/dodge/knockdown/stamina/…_synced` tests. | Keep one row (AU3 is the narrower, unblocked one) and delete the other two. |
| AU3 (02) | **DUP of SN10/PB61** | Same file, same 10 scenarios; AU3 only adds the 0-alloc proof on top of SN10's numbers. | Keep AU3 as the surviving row; retire SN10 and PB61 into it. |
| PX9 (08) | **DUP of PW16** | PW16 (SPRINT-28, open) "physics: the ragdoll 1 cm bar (closes PX9)" asks exactly PX9's failing bar (50-seeded falls; seed 44 overlaps `lower_arm_r`/`shin_r` by 8 cm). AD16 "amends PX9" also owns `crates/physics/src/joint_limits.rs`. | Let PW16 land the bar and close PX9 with it; do not work both. |

---

## C. WRONG PREMISE — 27 rows

### C1. The Bevy viewer was dropped: 14 rows name `crates/viewer/**`, which does not exist

Proof for all 14: `Test-Path crates/viewer` → **False**; `Get-ChildItem crates` lists no `viewer`
(ai anim app arena assets audio clock combat core destruction distill ecs editor endpoint
engine-chat chaos game geo input kam2-* material nav net nn observe ossim perf physfx physics
playtest proof provider rules save scene spatial ui web world workspace-hack). Every one of the 14
owns a file under `crates/viewer/src/` and every one already carries the `LATER:` line
"viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK".

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| BV3 | **WP** | `crates/viewer/src/manifest.rs` and `main.rs` both absent; row asks for per-pass GPU timers in the capture manifest of a crate that is gone. | Delete the LATER line and retire the row. |
| BV5 | **WP** | `target/bv1/frame_000060.png` absent, crate gone; the release pilot cannot be re-shot. | Retire with the rest of the BV block. |
| BV6 | **WP** | `crates/viewer/src/rig.rs`, `main.rs` absent (`rig::tests::every_body_gets_clips_at_its_own_size` has no crate). | Retire. |
| BV7 | **WP** | `crates/viewer/src/camera.rs`, `lib.rs` absent (`camera::tests::both_fighters_inside_the_frame`). | Retire. |
| BV8 | **WP** | `crates/viewer/src/manifest.rs` absent (`manifest::tests::the_frame_gpu_time_is_in_the_json`). | Retire. |
| BV9 | **WP** | `crates/viewer/src/kit.rs` and `target/camtest_270/frame_000090.png` absent. | Retire. |
| BV10 | **WP** | `crates/viewer/src/look.rs` absent; the "one look switchboard" switchboard has no host. | Retire. |
| BV11 | **WP** | `crates/viewer/src/main.rs` absent; the Bevy `bevy_light`/`bevy_pbr` donor paths are not in the tree either. | Retire. |
| BV12 | **WP** | same (donor `bevy_pbr-0.20.0-rc.2/src/ssao/mod.rs`, `contact_shadows.rs` absent). | Retire. |
| BV13 | **WP** | `crates/viewer/Cargo.toml` absent. | Retire. |
| BV14 | **WP** | `crates/viewer/Cargo.toml` absent. | Retire. |
| BV15 | **WP** | `crates/viewer/Cargo.toml` absent. | Retire. |
| BV16 | **WP** | `crates/viewer/src/main.rs` absent; the Bevy fog/volumetric/SSR donors are absent. | Retire. |
| BV17 | **WP** | `crates/viewer/src/rt.rs` and the `bevy_solari`/meshlet donors absent; the RTX ray-trace spike only lived in the viewer. | Retire; if a Bevy spike is still wanted, it is a new row, not this one. |

### C2. The Accept's "before" capture is not on disk: 10 rows can never be proven

Proof for all 10: `Test-Path target/pilot-owner-view` → **False** (the whole folder is gone);
`Test-Path target/pilot-gx13/owner-arena/frame_000015.png` → **False** (`target/pilot-gx13` exists
but holds only `capture/`, `r2/` and chart files). These rows each require
"capture pair before <that png> vs after".

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| CT29 | **WP** | Accept's before frame `target/pilot-gx13/owner-arena/frame_000015.png` → False. The 2026-10-19 note already re-scoped the row to "the capture pair only", i.e. to the one artefact that is missing. | Amend the Accept to a pair this seat renders itself (twin frame vs kit frame), or close it. |
| RL16 | **WP** | same before frame → False (`crates/render/src/twin_look.rs` is also absent). | Same: amend or close. |
| RL21 | **WP** | `target/pilot-owner-view/r2-20260929/arena/frame_000000.png` → False; the row's own 2026-10-19 note says the bind is landed and only the before PNG is missing. | Owner supplies the PNG or amends the Accept; the code half is done. |
| RL22 | **WP** | `target/pilot-owner-view/r2-20260929/editor-a3/viewport.png` → False; `crates/render/src/editor_day_preview.rs` absent. | Retire or rewrite the Accept; parked LATER since D7. |
| RL23 | **WP** | `target/pilot-owner-view/r3-20260929/arena/frame_000014.png` → False; `twin_grade.rs`, `twin_look.rs` absent. | Retire or rewrite. |
| RL24 | **WP** | `target/pilot-owner-view/r4-20260929/editor-a3/viewport.png` → False (`crates/editor/src/viewport_image.rs` does exist). | Rewrite the Accept as an on-disk pair. |
| RL25 | **WP** | `target/pilot-owner-view/r4-20260929/arena/frame_000014.png` → False (`crates/game/src/windowed.rs` exists). | Rewrite or retire. |
| RP11 | **WP** | `target/pilot-owner-view/r4-20260929/arena/frame_000014.png` → False (`crates/render/src/post.rs` exists). | Rewrite or retire. |
| RL26 | **WP** | `target/pilot-owner-view/r5-20260929/arena/frame_000014.png` → False; `target/rl26-20261019/after8/manifest.json` → True, so only the before half is missing (`743e3dcc`). | Owner amends Accept to the before/after pair the seat can render. |
| RP10 | **WP** | `target/pilot-owner-view/r2-20260929/arena/manifest.json` → False; `crates/render/src/clip_advance.rs` absent. | Rewrite or retire. |

### C3. False facts inside otherwise real rows

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| GP14 (07) | **WP** (path only) | Row owns `crates/anim/src/camera_framing.rs`; `Test-Path` → **False**. The module is `crates/combat/src/camera_framing.rs:1` ("camera_framing: third-person melee framing as pure integer functions (GP14)"), and `crates/combat/src/camera_framing.rs` → True. | Fix the owned path to `crates/combat`; then the only work left is the live-camera hook (row note: the arena camera still comes from `fighter_look::fight_camera`). |
| PF19 (07) | **WP** | Its Accept is `node tools/perf-baseline.mjs --check`; `Test-Path tools/perf-baseline.mjs` → **False** (the row's own Progress says "does not exist"). | Give the row the real tool path (`tools/perf/*` baseline) or close it as an unprovable Accept. |
| ARC13 (07) | **WP** | The Accept asks that a seeded divergence "fails gate + **reverts**", while the row's own `Owner chat 2026-10-19 (delegated)` decided "report-only … and no auto-revert is wired". The report half exists: `tools/tape-checksum.mjs:56-59` prints `first-divergence: <tick>`. | Rewrite the Accept to the owner's report-only answer and tick on that tool's test (`tools/tape-checksum.test.mjs:22`). |

---

## D. REAL — 135 rows

### D1. SPRINT-01 (13)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| VS3 | REAL | Kit base-colour textures do bind (`crates/game/src/arena_kit.rs:192-193, 605-606`); the Accept is a pixel F2P ("the sand region of the run"), no command, so it cannot be proven from the tree. | Run the F2P capture pair for the sand region, or reword the Accept to a test. |
| VS4 | REAL | `App::fighter_views` exists and is used (`crates/game/src/capture.rs:1532`, `windowed.rs:2758`, `staged_fight_frames.rs:388`); the row's own left: no test that ≥2 fighter meshes over 5,000 triangles are drawn. `crates/game/src/fighter_draw.rs` → False. | Add the drawn-mesh test (that is the whole row). |
| VS5a | REAL | No pose→joint-matrix builder in the crate: `git grep -n "joint_matrices\|to_matrices\|pose_matrices\|palette" -- crates/anim` → **no matches**. | Build `LocalPose` → 45 joint matrices in `crates/anim`, with the tick-t matrix test. |
| VS5b | REAL | `crates/game/src/fighter_draw.rs` → False; no `PoseSource::ImportedFighter` anywhere in `crates/`. | Build it after VS5a; the file the row names does not exist yet. |
| VS7 | REAL | Torches are not lights in the frame light list (RL42, the consumer, is open and needs VS7). | Feed torch actors into the frame light list; RL42 builds on it. |
| RA14 | REAL | `crates/render/src/present.rs` exists but the row is the integration gate over VS3/VS4/RA1/RL2/RL4/RL7, several of which are gone or open. | Re-scope to the presenter path that exists today (PW01/PW02 own `present.rs` in SPRINT-28) before anyone starts. |
| UI4 | REAL | Row note 2026-10-18: "skipped, needs crates/render edits and real GPU captures"; nothing landed. | Needs a GPU seat; hand it to the lane that owns render captures. |
| UI5 | REAL | UI half landed (`crates/ui/src/play_hud.rs:1`, raster test); left: paint it into the `play.step` PNGs (an endpoint→ui edge). | Build the endpoint paint call; the UI data is done. |
| Q1 | REAL | `content/reference` → False (no reference capture set) though `content/incoming/studio_small_09_1k.hdr` → True and `target/q1` → True. | Needs the reference GPU seat: the HDRI is vendored, the V1–V3 set is not. |
| Q2 | REAL | Accept `node tools/proof.mjs --skip-build` (`tools/proof.mjs` exists) but it needs a clean tree; today `engine-arena` does not compile (see the header). | Re-run the proof once the ecs/arena lane lands; do not open new work. |
| CT12 | REAL | `crates/assets/src/manifest.rs` → False; the cook manifest work is parked LATER and `crates/assets/src/bundle.rs:384` already has a ship manifest, a different artefact. | Keep parked; if wanted, split "bake manifest" out of the ship manifest. |
| WIN10 | REAL | `git grep -n -- "--version\|--help" -- crates/game/src/bin/arena.rs` → **no matches**; the CLI surface is not there. | Build argv → exit code with the loop never entered (`crates/game/src/bin/arena.rs`). |
| WIN11 | REAL | `crates/game/src/windowed.rs` exists; no suspend/restore drill (`Pause`/resume receipt) in the tree. | Parked shipping row; build the scripted minimize/restore drill if shipping resumes. |

### D2. SPRINT-02 (1)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| CB3 | REAL | `crates/combat/src/design.rs` exists ("hit-stop timing, cancel windows"), so the code half is there; the Accept is "PQ2 tournament balance divergence", which no command proves. | Run the PQ2 tournament and record the divergence numbers, or fold CB3 into PQ2. |

### D3. SPRINT-03 (9)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| GX14 | REAL (blocked) | Its own receipt `docs/receipts/2026-10-19-gx14-gtao-pilot.md` → True and `target/evidence/gx5/ra8.json` → True; the p95-at-1080p bar is unproven and needs `--size` in render/game. BLOCKED on a clean tree. | Leave parked; the owner decision (which row lands `--size`) is already logged in `docs/plan/2026-10-19-owner-queue.md`. |
| RL5 | REAL (blocked) | `crates/render/src/baked_gi.rs` → True with 25 tests and 2 ignored; the close needs the reference-GPU 60 fps 8-fighter receipt (operator). | Wait for the operator capture; nothing to code. |
| RL14 | REAL | `target/evidence/rl5-pacing` → False; the fixed-step pacing fix is not in. | Fix capture pacing before RL5 can close. |
| RL11 | REAL | Needs RL5 (blocked); `crates/render/src` and `tools/` present, no capture bundle. | Wait on RL5. |
| RL13 | REAL | Needs RL5/RL6; no material/exposure chart on disk. | Wait on RL5. |
| RL18 | REAL (blocked) | Pilot half green (`post.rs`, `view_state` present; row cites 12 passed / 1 ignored); BLOCKED on live-adapter per-pass ms. | Operator run for the adapter line. |
| RL29 | REAL | `target/rl29` → False; the boost-release capture pilot has no outputs. | Needs the release GPU run; do after RL5. |
| RL31 | REAL | `crates/render/src/sky.rs` → True; the Bruneton LUT bake is not there; parked LATER since D7. | Keep parked (D7 gate is dead but nobody asked for it). |
| GM3 | REAL (blocked) | `C:/e2040-clean` → False, `target/gm3-dist` → False; the 5-minute clean-folder run has no receipt. | Blocked on a clean-folder ship run; blocks PQ3/A8. |

### D4. SPRINT-04 (8)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| PQ3 | REAL | `crates/proof/src/soak.rs` exists; needs GM3 (blocked) so the packaged 1-hour soak cannot run. | Wait on GM3; no code work. |
| PQ11 | REAL | `crates/proof/src/online_soak.rs` exists and wave 54 adopted the spike, but the row waits on PQ3 (open). | Land the onset hunks once PQ3 closes; it is not a duplicate of PQ3 (different file, different bar). |
| PQ19 | REAL | `crates/proof/src/gpu_live.rs` exists; `target/evidence/c1` → False. Parked LATER. | Unpark only when a gate needs the receipt. |
| PQ20 | REAL | Status-table row only; wait on PQ19. | Keep parked behind PQ19. |
| PQ4 | REAL | `crates/playtest/src/lookcheck.rs` → False and RT7 (path-traced reference) does not exist (`PW11` is open saying "the engine has no path tracer"). | Real dependency: RT7 first. |
| API3 | REAL | `crates/endpoint/src/insight/memo.rs` → False. Parked LATER. | Keep parked (tooling outside focus). |
| MCP12 | REAL | `crates/endpoint/src/sandbox/` exists but `wasm_tool.rs` → False. Parked LATER. | Keep parked. |
| API5 | REAL | `crates/endpoint/src/insight/delta.rs` and `fixtures/insight-delta` → False. Parked LATER. | Keep parked. |

### D5. SPRINT-05 (4)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| CQ6 | REAL | The premise holds: `tools/rust-quality.mjs:396-400` (`boardIds`) reads only `docs/SPRINT.md`, and that file now holds **0** rows, so its id set is empty. | Point `boardIds()` at the `SPRINT-*.md` shards (one-line change), then burn down the ~1,049 comment ids. |
| CQ7 | REAL (blocked) | Row note: `hot_alloc` 120 → 0 (`cd85016d`), 2 CPU slices green, but the 120-frame window loop reads wgpu's own 23,318 command calls. | Owner decision on the probe floor; not more code. |
| CQ9 | REAL | 9 of 12 files split (`big_files` 19 → 10, run 39063249080); render fog/gtao/ibl and the rest remain. | Continue the split list in the row's note. |
| CQ10 | REAL | Casts 4,236 → 2,965; combat 64 → 22 (`013124a`); nav 226, anim 109, destruction 109 remain. | One crate per packet, as the row says. |

### D6. SPRINT-07 (71)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| RC26 | REAL | `crates/core/src/tick_record.rs` → False; no tick recording/replay module. | Build it (1,000 ticks, ≤0.2 ms/tick). |
| RC27 | REAL | `crates/ecs/src/sched_verify.rs` → False. | Build the schedule verifier. |
| RC34 | REAL | Waits on UW0 (urban war, later game); no second-game boot receipt. | Leave parked behind UW0. |
| ARC7 | REAL | `git grep -in spine -- crates/` returns bone code only (`crates/anim/src/additive.rs:94`), no legacy `spine` flag to flip. | The legacy flag is already gone: re-scope the row or close it as moot. |
| MP16b | REAL | Parent MP16 accept at 32 bots has no scale receipt; `crates/net/src/scale.rs` exists. | Needs a 32-bot LAN soak seat. |
| MP18 | REAL | No channel/compression flood chart; waits MP8/MP9/MP16. | After MP16. |
| MP21 | REAL | 64-player interest handoff has no spike output; waits MP16, ON3. | After MP16. |
| GP22 | REAL | `crates/combat/src/feel.rs` exists; one event fan-out at 0 allocs is not wired. | Build the fan-out; measure allocs. |
| GP6 | REAL | `fixtures/feel-gold` → **True** and the scorer landed (`perf --feel-gold`, 11 passed), so the row's first "fixtures missing" sentence is stale; left: no live PQ1 run feeds it. | Fix the stale note; the row now waits on PQ1. |
| ED18 | REAL | Needs a real newcomer on a clock (row note 2026-10-18); `crates/editor/src/onboard.rs` exists. | Cannot be proven by an agent; needs a person. |
| AS6 | REAL | `crates/scene/src/usd_codec.rs` → False (and `crates/assets/src/usd.rs` → False). | Build the USD reader. |
| SN4 | REAL | `crates/audio/src/hrtf.rs` exists but is a std-only HRIR sphere, not Steam Audio behind the `device` wall; parked with the audio block. | Keep parked. |
| SN9 | REAL | Waits on ON8; `crates/audio/src/voice.rs` → False (the file moved into `audio`), so only the boundary tests are missing. | Write the boundary tests once ON8 lands. |
| HR6 | REAL | `crates/combat/src/move_intake.rs` → False. | Build the generated-move intake. |
| SP20 | REAL | Engine half re-proven (`relation::sp20_qa50`, run 39486878406, 50/50) and the door verb 27 passed; left: the model-vs-raw-brief arm needs a live model. | Needs a model seat; the code half is done. |
| BET-SP27 | REAL (held) | Moonshot, held; no probe-field bake and no +10% measurement. | Keep held. |
| AX12 | REAL | Fork-ID plumbing in `crates/arena` is not there (row note 2026-10-18 skipped for this reason). | Needs the fork work to settle first. |
| BET-AX13 | REAL (held) | Moonshot, held; advisory report only. | Keep held. |
| BET-AX14 | REAL (held) | Moonshot, held; no tuner in the repo. | Keep held. |
| API6 | REAL | `crates/endpoint/src/collapse.rs` → False. | Build the collapse guardrail experiment. |
| RT14 | REAL | Parked LATER; no many-light spike under `examples/`. | Keep parked. |
| RT15 | REAL | Parked LATER; `crates/render/src/shadow_cluster.rs` exists, `graph.rs` → False. | Keep parked; overlaps RT20's shadow work. |
| VG12 | REAL | Parked LATER; meshlet spike rejected on LADDER, no code. | Keep parked. |
| CT23 | REAL | `content/fighters/incoming` exists; no 80-clip receipt (`content/fighters/CLIP-80-RECEIPT.md` → False). | Content batch work; needs real clips. |
| CT24 | REAL | `content/fighters/sidecars` exists; no audit doc. | Content audit. |
| CT25 | REAL | `content/crowd` and the deterministic cook are in (cb822736, `crowd_cook` PASS run 39105798540); left: soak captures with 0 findings on the reference GPU and a stale `content/crowd/proof/Cargo.lock`. | Refresh that lock, then the GPU soak. |
| CT27 | REAL | `content/crowd/scale` → False; needs CT5/CT23/CT25. | After CT25. |
| CT31 | REAL | `content/locomotion` exists; no 100STYLE/CMU bake receipt. | Content work. |
| CT34 | REAL | `content/crowd/scale` → False. | Needs the scale card. |
| CT36 | REAL | Sidecar consumer wiring not proved (needs CT24 audit). | Content work. |
| CT37 | REAL | `content/locomotion/real` exists; no 16-clip intake receipt. | Content work. |
| CT38 | REAL | `content/crowd/variants` → False. | Content work. |
| CT39 | REAL | `content/fighters/BUNDLE-AUDIT.md` → False. | Write the audit. |
| CT41 | REAL | `content/fighters/LOD-REPORT.md` → True (b5cdaf64); left: GPU ms and the PF13 join. | Needs the GPU. |
| CT43 | REAL | `content/fighters/sidecars/AUDIT.md` → False. | Content audit. |
| CT44 | REAL | `content/crowd/EYES.md` → False. | Content work. |
| CT47 | REAL | `content/fighters/incoming/batch-b` → False. | Needs real clips (donor files). |
| CT48 | REAL | `content/locomotion/real-batch-b` → False. | Needs donor files. |
| CT49 | REAL | `content/fighters/sidecars/TRAVERSAL.md` → False. | Content audit. |
| CT50 | REAL | `content/crowd/SOAK-REPORT.md` → False. | After CT25. |
| CT54 | REAL | `content/fighters/incoming/batch-c` → False. | Needs donor files. |
| CT55 | REAL | `content/fighters/incoming/finisher` → False. | Needs donor files. |
| CT57 | REAL | `content/arena/NIGHT-REPORT.md` → False. | Content work. |
| CT58 | REAL | `content/fighters/CLIP-80-RECEIPT.md` → False. | After CT42/CT47/CT54/CT55. |
| CT59 | REAL | `content/fighters/CONSUMER-PROOF.md` → False. | After CT58. |
| CT60 | REAL | `content/arena/STAGED-PROOF.md` → False; also needs CT29, which C2 above shows is unprovable as written. | Re-scope with CT29 or drop the capture half. |
| CT61 | REAL | `content/arena/SOAK-BASELINE.md` → True (f2effc78, 236 MiB); left: the 8-fighter day capture ms on the GPU. | Needs the GPU. |
| CT62 | REAL | `content/maps/STREAM-REPORT.md` → False; `content/maps/urban-war` → False. | Content work. |
| CT63 | REAL | `content/sound/HIT-LIST.md` → True (b29e14b0); left: the SN10 bridge, which is itself parked (see B). | Wait on the audio bridge. |
| CT64 | REAL | `content/crowd/REACT-REPORT.md` → False. | Content work. |
| CT65 | REAL | `content/capture/PILOT-REPORT.md` → False. | Content work (offline pilot). |
| CT66 | REAL | `content/maps/urban-war` → False. | Needs UW pilot. |
| CT67 | REAL | `content/props/splat/PILOT-RECEIPT.md` → False. | Spike work. |
| CT75 | REAL | `content/fighters/incoming/STAGED-TRIAGE.md` → False. | Content triage. |
| CT76 | REAL | `content/locomotion/SCOUT-REPORT.md` → False. | Content work. |
| CT79 | REAL | `content/fighters/tools/dedupe.py` and `DEDUPE-RECEIPT.md` → False. | Write the tool. |
| SP33 | REAL | No recall@3 test or fixture; needs SP19/SP20 (SP20 still needs a model). | After SP20. |
| BET-SP57 | REAL (held) | Held moonshot; no `conflict` preset test in `crates/spatial`. | Keep held. |
| SP58 | REAL | Needs SP20 (open); no Subject+World+Viewer cone verb in `relation.rs`. | After SP20. |
| BET-SP59 | REAL (held) | Held behind SP58/SP20. | Keep held. |
| RA19 | REAL (Accept not run) | `crates/render/src/tier.rs` → True; the crate carries GPU code so its tests run on this PC, and the build directory was locked by another lane, so I did not run `cargo test -p engine-render --lib tier`. | Run that line when the build queue is free; it is the only thing between RA19 and a tick. |
| WIN7 | REAL | Parked shipping row; `crates/game/src/pad.rs` → True; the hot-plug drill is not written. | Keep parked. |
| GP25 | REAL | Row owns `camera_feed.rs`, which → **False** (only comments point at it); GP24 wiring not in. | Re-check the file name before starting. |
| GP26 | REAL | Same missing `camera_feed.rs`; needs GP21/GP11. | After the shake stage lands. |
| BET-SP31 | REAL (held) | Held moonshot; no postings ledger. | Keep held. |
| SPR4 | REAL | Harness landed (390d65c2, `spatial_eval/llm.rs`, echo gold 50/50); left: record a real model. | Needs a model seat. |
| SPR5 | REAL | `git grep -n "place_options\|dry_run" -- crates/spatial` finds only `diff::dry_run_diff` (`crates/spatial/src/diff.rs:102`), no `place_options`. | Build the `dry_run` placement option and its Accept test. |
| SPR8 | REAL (blocked by another lane) | The tests exist (`crates/endpoint/src/scene_query/compact_tests.rs:55`); the Accept could not pass today: `cargo test -p engine-endpoint --features mcp --lib query_compact` died compiling `engine-arena` — `crates/ecs/src/pool.rs:327` `shared` is `pub(crate)` (E0603). Row note also says "no separate legend block". | Re-run after the ecs/arena lane lands; add the legend block. |
| SPR10 | REAL | `git grep -n frame_target -- crates/` → **no matches**; `spatial_frame.rs` landed d8f483de. | Build the endpoint verb + the 20-shot test. |
| SPF7 | REAL | `git grep -n scene_iq_bars -- crates/` → **no matches**; needs AD25 (open). | After AD25. |
| SPX12 | REAL | No `egress` verb in `crates/` (`git grep` matches only "regress"); `crates/spatial/src/navcheck.rs` exists. | Build `scene.query kind=egress`. |

### D7. SPRINT-08 (29)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| AM13 | REAL | `crates/anim/src/lod.rs` tier table landed (390 tests green); left: the 64-fighter <2 ms timing on the reference CPU, plus GPG8's promotion/no-pop test. | Needs the reference CPU run and the promotion rebuild. |
| AM15 | REAL | `crates/anim/src/additive.rs:4` says "consumer: the fighter look calls `AdditiveRig::tick`", but `git grep -n AdditiveRig -- crates/game` → **no matches**, so the call is still missing. | Wire `AdditiveRig::tick` into the fighter look; then the pops replay. |
| AM18 | REAL (Accept not run) | `crates/anim/src/crowd_scale.rs` landed; the p99 is a model and `thousand_agent_tick_p99_within_budget` is `#[ignore]`; I did not run `cargo test -p engine-anim --lib crowd_scale` (build lock). | Run that line; then make the p99 a measurement, not a model. |
| LM6 | REAL | `crates/anim/src/warp_ground.rs` exists; no 20-degree slope or stair fixture (`git grep -in slope -- crates/anim` finds nothing in `warp_ground`). | Add the slope/stair fixtures and the foot-slide bars. |
| AM21 | REAL | `git grep -n "warp\|strob" -- crates/anim/src/lod.rs` → **no matches**, so no `lod_warp` test. | Build the warp-window vs strobing contract. |
| LM7 | REAL | `crates/anim/src/motion_match.rs` exists; no seeded 600-tick fatigue soak, needs LM4. | After LM4. |
| AM22 | REAL | No facing arrow or lean at LOD (`git grep -n facing_arrow` → no matches). | Build the LOD readability cue. |
| LM4 | REAL | `motion_schema_test.rs` freezes the LM1 schema; left: PF1 ms workloads and recorded baselines (needs a perf→anim edge). | Add the PF1 workload entry. |
| PX5 | REAL | `crates/physics/src/learned_ctrl.rs` → False. | Build the offline controller. |
| NN4 | REAL | `crates/nn/src/ort_backend.rs` → False; `deny.toml` present. | Optional backend, behind a feature. |
| PX11 | REAL | `crates/physics/src/servo.rs` → False. | Build the fixed-point servo after PX9/PW16. |
| PX32 | REAL | `crates/physics/src/fluid.rs` → False (`crates/render/src/fluids_gpu.rs` is presentation only); parked LATER. | Keep parked. |
| DX6 | REAL | `crates/destruction/src/debris_lod.rs` landed and is wired (`40119424`); left: the 1 ms budget test is still `#[ignore]` and unrun, and the PQ4 lookcheck needs a human. | Un-ignore the budget test on the reference box. |
| DX7 | REAL | `crates/editor/src/fracture_tool.rs` → False. | Build the fracture authoring tool. |
| PX18 | REAL | `crates/physics/src/ragdoll.rs` exists; no 8-fighter clinch soak. | After PX9/PW16. |
| DX12 | REAL (Accept not run) | `crates/destruction/src/budget.rs` → True; I did not run `cargo test -p engine-destruction --lib handoff` (build lock, other lanes building). | Run that line; it is the row's whole Accept. |
| PX22 | REAL | `crates/physics/tests/servo_calib.rs` → False; and integration tests must live under `tests/it/`. | Build the calibration corpus in the right place. |
| DX17 | REAL | `crates/destruction/src/impact_table.rs` landed (486 entries × 3 shapes, replay ×2); left: a caller at spawn and the blind pick. | Wire it at spawn. |
| CT14 | REAL | `crates/assets/src/motion_source.rs` → False. | Build the provider-style motion source. |
| CT16 | REAL | `crates/endpoint/src/sandbox/content.rs` → False (`sandbox/` exists). | Build the content sandbox rule. |
| NV5 | REAL | `crates/nav/src/platoon.rs` → False; no `platoon` symbol anywhere in `crates/`. | Build platoons after NV4/PX7. |
| NV8 | REAL | No 500-agent nav soak harness (`git grep -n nav_soak` → no matches). | After NV5. |
| GA5 | REAL | Thinking LOD is not there; needs GA2/NV5. | After NV5. |
| GA7 | REAL | `crates/ai/src/diversity.rs` → False and `fixtures/fight-diversity` → False; only a comment mentions diversity (`crates/arena/src/live.rs:1194`). | Build the offline strategy search. |
| GA9 | REAL | `crates/ai/src/legion.rs` exists; no 500-vs-500 fixture (`fixtures/` has no legion file). | After GA2/NV5. |
| NV14 | REAL | `crates/nav/src/tiles.rs` → False; held on NV13/NV3. | Keep held. |
| PF5 | REAL | Row note: all four named loops were improved by allocation/algorithm work (PF33/PF35), **not** by SIMD. | If SIMD is still wanted, it is a new measurement; otherwise close the row on the note. |
| PF9 | REAL | `crates/observe/src/rerun_sink.rs` landed (observe green 39057303818); left: GPU timestamp queries, the 2,000-frame leg and the RA8 manifest (need a GPU). | Needs the GPU seat. |
| SG3 | REAL | Game half landed (`6b9bf722`), headless duel proof `ticks=300 hash=8283177473c16d92` on both seats, wind-up capture `target/captures/spear-windup.png` recorded; left: a hand-played keyboard run with one tow and the web page `spear= tether=` line. | Two proofs left, no code. |

---

## E. Rows I could not judge, and why

None of the 168 was left unjudged. Four verdicts are **REAL without its Accept run**, and I say so
in the table rather than calling them done:

- **RA19** — its test needs this PC (render holds GPU code) and the build directory was locked by
  another lane's cargo; I did not start a second build.
- **DX12**, **AM18** — same reason; both Accepts are one `cargo test --lib` line each.
- **SPR8** — its Accept is red for a reason outside the row: `crates/ecs/src/pool.rs:327`
  (`pub(crate) fn shared`) breaks `engine-arena` (E0603), so `engine-endpoint` cannot build.

Two more need a person rather than an agent, and that is recorded, not hidden:
**ED18** (a real newcomer on a clock) and the GPU/operator rows (RL5, RL18, Q1, GM3, CQ7).

The golden `d57b5c8ae3096614` was not measured by this audit: nothing here ran a sim binary.

---

## F. Board fix applied 2026-10-21 (board fixer) — 33 dated lines, 0 rows deleted, 0 rows ticked

Scope: `docs/SPRINT.md` (0 rows) and `docs/SPRINT-01` .. `SPRINT-08` only. Other shards untouched.

- **27 wrong-premise rows** got a dated `Correction 2026-10-21:` line; row kept, not ticked:
  - the 14 `crates/viewer` rows: BV3, BV5, BV6, BV7, BV8, BV9, BV10, BV11, BV12, BV13, BV14, BV15, BV16, BV17 —
    each names the dead path/PNG and that the row is dead until the owner re-aims it;
  - the 10 missing-before-capture rows: CT29, RL16, RL21, RL22, RL23, RL24, RL25, RP11, RL26, RP10 — each names
    the missing PNG/manifest and says the Accept cannot pass as written;
  - GP14 (module is `crates/combat/src/camera_framing.rs`, not `crates/anim`), PF19
    (`Test-Path tools/perf-baseline.mjs` = False), ARC13 (Accept asks for a revert the owner refused, so it needs
    re-aiming to the report-only answer).
- **4 duplicate rows** marked with a dated line naming the owner: WIN6b → WIN6, SN10 → AU3 (PB61 in SPRINT-09 names
  the same), PX9 → PW16. AU3 also carries a line saying it owns the audio pair.
- **Already done, not ticked: SPR7 and SPR9.** Accepts passed (runs 39515251019 and 39513546216) and the rows now name
  their landings — SPR7 `a2dba030` (sees.rs, plus 755cd907 and 5cc241ed), SPR9 `755cd907` (rooms.rs, plus 39b0e28a).
  They are not ticked because both carry `needs: AD7` and AD7 is open, and `workspace-check` refuses a ticked row that
  needs an open one. Dry run on the joined board: baseline board errors `[]`, after tick
  `["board: SPR7 is ticked but needs open AD7","board: SPR9 is ticked but needs open AD7"]`. Tick them when AD7 closes,
  or drop AD7 from their `needs:` first.
- **135 REAL rows** and **RA19, DX12, AM18, SPR8** untouched.

Checks after the edits, all from `C:\engine2040`:

- `git diff --stat -- docs/SPRINT-0*.md` → `SPRINT-01 16 +`, `SPRINT-02 1 +`, `SPRINT-03 9 +`, `SPRINT-07 6 +`,
  `SPRINT-08 1 +`; row count and tick state identical to HEAD in all five files (47→47, 4→4, 22→22, 213→213, 91→91,
  0 tick flips).
- `node tools/workspace-check.mjs` → **FAIL**, and the FAIL is not the board: the board section parses clean
  (1689 tasks, 0 board errors, "every needs resolves, no ticked row needs an open one"). The red lines are other lanes'
  in-flight files: `engine-arena -> engine-nav: undocumented local dependency edge`; seven undeclared modules
  (`arena/input_layers.rs`, `arena/nav_layers.rs`, `endpoint/resources.rs`, `endpoint/scenarios.rs`,
  `endpoint/verify_tasks.rs`, `game/class_sheet.rs`, `input/layers.rs`, `physfx/body.rs`, `kam2-web/build_menu.rs`);
  four unlisted genre names. Nothing under `crates/` was touched by this pass.
- `node tools/board-slice.mjs --self-check` → FAIL 8/10: VS3 10321 B and VS4 10893 B are over the 8192 B slice budget.
  Both rows are untouched by this pass (long Progress notes) and the size is identical to HEAD.
- `node tools/board-slice.mjs BV9` / `SPR9` → the correction line sits inside the row and the row still slices
  byte-identical.

The golden `d57b5c8ae3096614` was not measured: no sim binary ran, no crate or content path changed.
