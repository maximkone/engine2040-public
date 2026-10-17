# Wiring triage 2026-10-17: what exists but nothing calls

Owner question, 2026-10-17: did we remove good work, is there more hidden stuff, can we finish much faster by wiring what already exists? Read-only analysis by one session; every claim below was checked by reading code or running a grep unless it says "not checked". Nothing was built or run with cargo. Tree state: `master` with other sessions' edits in the working tree (snapshot at about 20:15; section 8 lists the files they hold).

## 0. The short answers

1. **Did we remove good work?** Mostly no, with four exceptions worth restoring when their row starts. Of the 38.9k cut lines, 14.4k are physics; about 5.2k of those (the hand-made solver stack and its tests) were replaced by Rapier (physfx, powered ragdoll, wasm parity proven) and 4.2k came back rewritten (cloth, islands, query). The net cut (lag compensation, guard, replicate, server) was written for an older server shape and a different input shape; the live design (tick-stamped inputs, rollback with an 8-tick resim cap, `LateInput` refused in `session.rs`) does not need it. The four worth restoring: `game/package.rs` (1,596 lines, the Windows package tree that GM3 and EG41 need), `playtest/look/metric.rs` (1,110 lines of integer SSIM and deltaE that WEB5 and G4 need), `nav/crowd.rs` (1,491 lines of ORCA that NV4 asks for), and the choice on `render/live.rs` (HR2, owner decision 3). Details in section 2.
2. **Is there more hidden stuff?** Yes, and it is more than the scan says, in a different shape than the scan says. The item scan (5,566 items) overstates (37% are `pub const`s, and a helper used only inside its own file counts) and also understates (names that appear in a comment elsewhere count as used: `BreakLook` and `PhysFx::spawn_ragdoll` have no production caller and are not on the list). My module-level pass (section 1) finds **106 files, about 80k lines, whose public names nobody else's code uses**; 40k of that is in crates a player can see or hear (anim 9.0k, render 6.7k, assets 6.2k, game 3.9k, combat 3.3k, physics 3.2k, spatial 2.8k, net 2.5k, ai 2.3k, audio 2.1k). The most important single fact: **`engine-physfx` has no production caller in any binary.** `break_look.rs` (the only game code that names it) is never instantiated; ragdoll, limb, blast and thrown are also unused. The window draws no physics at all.
3. **Can we finish much faster by wiring?** For the physical layer, the HUD, sound, LAN play and the pad: yes, days not weeks, ten wirings in section 4. For the whole plan: no. Roughly 45% of the FOCUS Now rows are wire-dominant, about a quarter to a third of all 960 open rows, and the biggest gates still need real writing: G10 (browser frame, browser net), the dismemberment authority (nothing in the live duel ever calls a cut), the lit frame's remaining passes, G4's score. Estimate and method in section 6.

Two corrections to the premises in the question: (a) "only debris is wired from game" is not so: `BreakLook` is built and tested (10 tests) but no binary creates one, so even debris is unwired; (b) dismemberment is not "decided but never drawn": the cut resolver `combat::resolve_swing` and `arena::limbs::spawn_limbs` have no caller in the live duel (only tests and `proof/break_corpus`), so the authority never decides a cut during play either (section 3, F8).

## 1. What the 5,566 means (and a better count)

`node tools/wiring-scan.mjs --json` today: 5,566 items (baseline file pins 5,317; the scan is regex and name based).

| Fact | Evidence |
|---|---|
| By kind: 2,065 `const`, 2,260 `fn`, 929 `struct`, 301 `enum`, 11 `trait`. So the type count is 1,241 and the constant count is 37%. | JSON run for this page |
| An item is "used" if its name appears as a token in any other file's non-test text, comments and doc lines included. A helper used only in its own file counts as unwired. | `tools/wiring-scan.mjs` `scanIslands`, lines 175-230 |
| `prodOf` cuts a file at its first `#[cfg(test)]`, even when that attribute sits on a statement in the middle of a function. 66 files lose 29,318 lines to this (arena `lib.rs` has its first `#[cfg(test)]` at line 78 of 4,732, so the scan reads almost none of it). With a module-aware cut the total is 5,589 and ai drops 239 to 167. | my re-run of the same logic with the cut at `#[cfg(test)] mod` |
| Under-count: `BreakLook`, `spawn_ragdoll`, `spawn_limb`, `apply_blast` are not on the list but have no production caller, because their names appear in comments elsewhere. | `physfx/src/lib.rs` doc, `render` text |
| endpoint's 669 are mostly tool-name consts, DTOs and pack internals; the packs are registered through `packs/mod.rs` (`web::pack()` added by another session right now). Not a backlog. | `packs/mod.rs:457-470`, kinds table above |

Better unit: files whose distinctive public names (CamelCase types and snake_case fns longer than 7 characters) are used by no other file's code, comments stripped, at least 3 such names and 200 lines. Result: **106 files, 79,996 lines**; by crate: endpoint 14.8k (registry packs, discount it), anim 9.0k, render 6.7k, assets 6.2k, proof 4.1k and perf 4.0k (tool crates, run by tests and CI), game 3.9k, editor 3.7k, combat 3.3k, physics 3.2k, spatial 2.8k, net 2.5k, ai 2.3k, audio 2.1k, ecs 1.8k, provider 1.8k, arena 1.4k. The per-feature file lists in section 3 come from this pass; the full list is the appendix.

Reachability roots, for the record: the binaries are `arena` (window), `arena_capture`, `staged_fight`, `e2040-mcp` (door), `e2040-editor`, `net-bout`, `net-relay`, `bundle_cook`, `e2bundle`, `distill_duel`, `arena_line`, `matrix_line`, `ragdoll_line`. No crate depends on `engine-nav`, `engine-playtest` or `engine-perf` (their dependents list is empty), so those three crates are libraries for tests and tools only today.

## 2. Did we remove good work? Verdict per cut group (2026-10-16)

Source of the list: `docs/archive/2026-10-16-rot/RECOVERY.md` (81 files, 38,904 lines). I read the first lines of the cut files in git (`git show <sha>^:<path> | head`) and checked the live replacement.

| Group (commit, lines) | What it was | What replaced it or why nobody calls it | Verdict |
|---|---|---|---|
| physics solver, broadphase, substep, islands_build, cast, simd_math, checked_fix, verify (`3d6ee4a1`, 3.8k source plus 1.4k tests) | hand-written sequential-impulse stack | Rapier through `physics/rapier.rs` and `physfx`; the golden tape and the Rapier ragdoll tape are equal on 3 native targets and wasm | stay deleted |
| physics vehicle 621, grapple 1,171, lod 844 | vehicles, hook, army LOD | no plan row has a consumer except PX16 (LOD scale proof) | stay deleted; `git show 3d6ee4a1^:crates/physics/src/lod.rs` if PX16 is pulled |
| physics cloth 2,654, islands, query | cloth, sleep islands, ray queries | `cloth.rs` rewritten fresh today (1,587 lines, 20 tests, `331dca1`), slim `islands.rs` and `query.rs` came back | done, nothing to restore |
| net lagcomp 828, guard 846, anticheat 478 (`3151bd46`) | attacker-view window, admission checks, input sanitize | rollback with tick-stamped inputs: `session.rs` refuses `LateInput`, resim cap `RESIM_CAP_TICKS = 8`. Lag compensation is not needed when late inputs are refused and re-simulated; `anticheat` sanitizes FPS look deltas, a shape the duel input no longer has | stay deleted; re-open MP5 and MP20 as "not needed", write a thin tick-and-button admission check (about 200 lines) at ON5 |
| net replicate 1,501, server 497, resim 139 | entity replication, an old headless server | `snapshot.rs` + `delta.rs` + `dedicated.rs::HeadlessServer` (MP17 landed) | stay deleted; MP12 should be re-pointed or dropped |
| net nat 409, steam_relay 278 | NAT plan, Steam transport | its own header says it reuses `courier`/`split`/`relay`, which stay | stay deleted; Steam only if a Steam ship is chosen (`game/steam.rs` is 353 lines and unwired too) |
| game camera_feel 201, camera_stack 232, eight_fight 211 | follow rig, shot stack, gate shape | their own headers say "retired, no production caller"; `camera.rs` (4,504 lines) has the director and LF11 shake landed | stay deleted |
| game cine 651 | intro/outro cinematic player | no row asks for it now | restore on demand (standalone, presentation only) |
| game modes 764, window_bind 380, quest 247 | display mode list, twin staging, quest chain | superseded or no game uses them | stay deleted |
| game ballistics 1,174 | siege-engine projectile sim | only a siege mode would use it; physfx does cosmetic arrows | stay deleted until a siege mode is chosen |
| game **package 1,596** (+ `package_portable_vectors` 162) | versioned Windows tree with the `arena` binary and bundle, `verify_clean` | GM3 (clean-machine run, BLOCKED) and EG41 (packaged smoke) have no packager now | **restore at GM3**: it is a finished 1.6k-line module plus vectors; a rewrite is the same size |
| nav astar 170, hpa 176, neighbors 139 | grid A* and hierarchy | no crate depends on nav | stay deleted |
| nav **crowd 1,491**, funnel 824 | ORCA avoidance, portal funnel | NV4 asks for ORCA, NV6 for the funnel; no crate depends on nav, so a caller must come first | **restore when NV4/NV6 starts**, not before |
| destruction debris 682, bond_grade 993, wall_slice 395, nav_dirty 419 | baked debris, offline strain grading, siege wall slices, nav dirty sets | debris is superseded by `physfx` + `break_look`; bond_grade fed an offline corpus (DX14); the others need a siege map or nav | stay deleted; wall_slice if siege walls are chosen |
| perf pgo_harness 551, server_budget 912 | PGO harness, server budget (PF8) | `net/tick_budget.rs` carries the server tick bars | stay deleted (re-open PF8) |
| playtest **look/metric 1,110**, look/judge 2,266, manifest 476, machine_eyes 192, look_score 169, look_judge 196, look 203, live_probe 92, agreement 373 (`f1a4f9d7`, 5,077) | integer image metrics (SSIM, deltaE, histograms), nine-item look rubric, machine score | G4 has no machine look score without it; **WEB5 needs an SSIM** for "browser frame beside native frame" | **restore `look/metric.rs` first** (std-only, runs on a runner), the judge after the look chain settles |
| render (16 files, `f737386e`): live.rs 1,389 (HR2), tunnel_mouth, clip_hold, v1_vfx, clouds, restir_gi, splat, stream, uploads, skin, fog_vol, gi_dyn, baked_halton, tonemap, draw_id_reduce, wound_queue | various | only the file list was read (`docs/plan/render-look.md`); not checked file by file | owner decision 3 stands for `live.rs`; the rest stay deleted, `wound_queue` is worth a look when GS19 starts |

Net: the cuts removed little that a plan needs. The losses that will cost time are the four bolded. The status page's "25 rows ticked with no code" is a bookkeeping problem (re-open the rows) more than a code problem.

## 3. The features (ranked by payoff to: heavy realistic multiplayer game, Windows and browser, destruction, ragdoll, dismemberment, good graphics, AI-easy setup)

Legend. Cost: S under a day (about 200 lines plus a test), M one to three days, L a week or more. Hash: does the change touch integer simulation state (Y/N). Test counts are `#[test]` attributes in the file (counted, not run). "Held" = a file in the list in section 8.

### F1. The break reaches the window (RZ4)
- Unwired: `game/break_look.rs` 1,066 lines, 10 tests; `physfx` debris path (`spawn_debris_of`), 52 tests across `physfx/src/tests*`; `destruction` bonded 19 tests, fracture 13, shards3d 10, all green on runners.
- Tests it: `cargo test -p engine-game --features app --lib break_look` (scripted wall break, cap, sleep, hash equal).
- Rows: RZ4 (open), RZ1 (done), NS11 (dust feed).
- Missing wire: `App::simulate_tick` (`crates/game/src/lib.rs:1134`) has `stepped.breaks` and drops it; add a field to `TickOutcome` (`lib.rs:461`) and fill it there; call `BreakLook::observe` in `bin/arena_frame/mod.rs::run_sim_ticks` after `tick_into`; upload `BreakLook::meshes()` in `ArenaApp::init_presenter` (`bin/arena.rs:1804`); append `BreakLook::frame(alpha)` draws in `ArenaApp::push_fighters` (`arena.rs:1536`). `BreakLook::new` needs the crate's chunks, bonds and anchors: `arena` keeps `CRATE_CENTER_M` private (`lib.rs:2185`) and offers no accessor, so add one `pub fn` (additive, no state change). Not checked: whether the kit's `crate_a` placement is the same crate as the bonded prop at `(0, 0.5, 0)`.
- Cost S-M. Hash N (picture only). Conflicts: `break_look.rs` has a 297-line uncommitted edit by another session right now; `arena.rs` and `windowed.rs` carry a small watchdog edit.
- Cut: n/a.

### F2. HP bars and HUD on screen (EG14)
- Unwired: `windowed.rs::stage_hp_bars` and `duel_overlay_for_app` (called only from `bin/arena_capture.rs:783`), `hud_cells_quads`.
- Tests it: `duel_overlay_tracks_live_hp_and_pips`, `hp_bars_for_overlay_pairs_rows_in_order` in `windowed.rs`; render `overlay.rs` and `Presenter::push_hp_bar`.
- Rows: EG14 (open; it describes the missing pipeline, which now exists), EG25.
- Missing wire: `ArenaApp::present_frame` (`arena.rs:1417`) calls `stage_hp_bars(presenter, &duel_overlay_for_app(&self.app))` before `render_surface`. `arena.rs:1189` says "the overlay rows stay parked". The capture path (offscreen) already does this. Not checked: whether the surface encode path draws the overlay quads (`submit.rs:1304` is where the plan is built).
- Cost S. Hash N. Conflicts: `arena.rs` (small); if the surface path needs a render change, `present/gpu_present/*` is held.

### F3. Ragdoll when a fighter is floored (RZ2)
- Unwired: `physfx` `spawn_ragdoll`, `Rig`, `step`, `transforms` (22+ physfx tests, `ragdoll_tape` digest equal on 4 targets); `physics/powered.rs` 1,959 lines 16 tests; `physics/ragdoll.rs` 2,740 lines 22 tests; no humanoid bone-shape table exists anywhere (`rg Rig::new crates` finds no production use).
- Rows: RZ2 (open), LF17 (open, "the dead lie on the sand", superseded by this), WEB3 (done).
- Missing wire: trigger is `FighterLook::observe_live_balances` (`fighter_look.rs:1422`), which already reads `GaitPhase::Fallen` from `LiveMatch::balance`, plus `FightCue::Knockdown`; velocity from two `FighterView`s; impulse from attacker-to-victim direction times damage (the cue carries no impulse, so this is a few lines of write). `PhysFx::step()` once per accepted tick in `run_sim_ticks`; `slot_pose` (`fighter_look.rs:1597`) reads `transforms(alpha)` for a ragdolled slot. Needs a `Rig` for the CT7 skeleton: a `BoneShape` per bone (write, S).
- Cost M. Hash N. Conflicts: `fighter_look.rs` is not held now but is a one-writer file (5,143 lines; RZ2 and RZ3 own hunks of it); serialize F3, F8-picture, F9, F10.

### F4. The window hears space, surface and tension (EG21, SN12, SN13)
- Unwired: `audio/director.rs` tension layers (the game computes `TickOutcome.layers` and `bed_gain` every tick and nothing in the arena binary reads them); `FeedTx::tension` and `stinger` have no caller; `feed_tick_audio` (`windowed.rs:858`) sends a fixed listener and flat `play` only; `combat_cues` surface picker (landed 3b01824 with RZ8) unused; `reverb.rs` 1,055 lines 20 tests, `hrtf.rs` 860 lines 14 tests, `music.rs` 763 lines 12 tests, `voice.rs` 452 lines 8 tests, `coherence.rs`; `mix_tick_audio` (spatial, tunnel send, wall muffle) is called only by the legacy `run_windowed` and smokes, not by the `arena` binary.
- Rows: EG21, SN12, SN13, SN4/SN5 (HRTF), GS23 (fetch list with Roman sound).
- Missing wire: `feed_tick_audio` (listener follows the camera, `play` carries the hit position from `FighterView`, `tension` from `outcome.layers`, crowd bed from `bed_gain`); or call `mix_tick_audio` from `arena_frame::run_sim_ticks` where `self.tick_scratch.audio` is played. Content gap: `content/audio` holds footsteps, impacts and ambience only, no crowd beds, music or stingers.
- Cost S-M. Hash N. Conflicts: none (audio and game audio files are not held; `audio/src/heal.rs` has one small edit).

### F5. Two people play from the window (EG44, UIX3b, MP6 real leg)
- Unwired: `game/netgame.rs` 1,626 lines 13 tests (`NetGame::host`/`join`, `tick_local`, `poll`, `flush`, `remote_frame`); the `arena` binary has no `--host` or `--join` and never constructs it (only `windowed.rs` uses the lobby entry helpers, `netgame::windowed_lobby_entry`); `net-bout mp6-serve/mp6-join` plays the same protocol without a window.
- Tests it: loopback UDP with a latency/loss matrix in `netgame.rs`; `net_bout mp6` PASS 24/0 on loopback only.
- Rows: EG44 (open; its title is "rules version on the wire, window plays v2"), UIX3b, MP6 (loopback ticked), MP25, GM2.
- Missing wire: another session is already doing the first half: `arena/src/spec.rs` (untracked, `MatchSpec`) says "the window, the net server and the net client all build from it". The second half: make `ArenaApp.app` either an `App` or a `NetGame`, and have `arena_frame::run_sim_ticks` call `tick_local` and the per-frame `poll`/`flush`. Limits: 2 human slots, UDP, LAN.
- Cost M. Hash N (same sim, same config). Conflicts: `net/*` held (15 files) but this row stays on the game side; `arena/live.rs` and `lib.rs` held by the `MatchSpec` work.

### F6. The pad works in the `arena` binary (EG27)
- Unwired: `game/pad.rs` 711 lines (`poll_gilrs_once`, `merge_pressed`, `fighter_input_from_pad`); `windowed.rs:1298` creates `Gilrs` only inside `run_windowed`, which no code calls.
- Missing wire: `Gilrs::new()` in `ArenaApp::new`, one poll per wall step, `merge_pressed` into `self.pressed` before `fighter_input_from_pressed` in `run_sim_ticks`.
- Cost S. Hash N. Conflicts: none beyond `arena.rs`.

### F7. Blood, sparks and dust (LF6, LF7, NS11, GS19)
- Unwired: `render/decals.rs` 1,873 lines 18 tests (it already defines a `GoreEvent` consumer), `render/wound_map.rs` 469 lines 5 tests, `render/particles_gpu.rs` 2,524 lines 31 tests, `combat/gore.rs` 937 lines 8 tests (`WoundBoard`, `GoreSetting`); the presenter holds bind probes for decals and particles (`present/gpu_present/frame.rs`, `presenter.rs`), not draws.
- Missing wire: game side, `Hit` cue to `GoreEvent` and `BreakEvent` to emitters in `run_sim_ticks` (S); render side, replace the probes with draws in `gpu_present/submit.rs` (M, one render writer).
- Cost M. Hash N. Conflicts: render held (`present/gpu_present/live.rs`, `presenter.rs`, `present/look.rs`).

### F8. Limbs come off: authority, then picture (RZ3, BX rows)
- Unwired, authority: `combat/cut.rs` `resolve_swing`/`cut_one` (1,187 lines 9 tests, used by `proof/break_corpus` only), `combat/limbloss.rs` 891 lines 10 tests, `combat/anatomy.rs`, `combat/hitsim.rs`, `arena/limbs.rs` 590 lines 10 tests (`spawn_limbs` has only a test caller; `Sim` steps the pool and folds it into the V2 hash, but nothing ever spawns a limb into it), `physics/limb.rs`.
- Unwired, picture: `physfx` `spawn_limb_of`, `assets/sever_cook.rs` 655 lines 7 tests (no caller), `render` stump cap (not started), `combat/gore.rs`.
- Rows: RZ3 (open), BX4 (done), BX series, PX17 (snapshot proof done).
- Missing wire: this is half write. In `Sim::try_step_into` (`arena/src/lib.rs`, after the hit loop near line 1625) build a `BladeSwing` from the sweep (`WeaponSweep` is already used in `match_wire.rs`) and `JointTarget` capsules from the victim's pose in integer mm, call `resolve_swing`, then `limbs::spawn_limbs`; expose the cuts in `LiveTick` like `breaks`. Then the picture: one `spawn_limb_of(Flesh, ...)` per cut, a stump cap, a decal.
- Cost L (authority M-L, picture M). Hash **Y**: V2 only (the pool is already folded in `rules.fold`), V1 golden `d57b5c8ae3096614` must not move, a new V2 golden to pin; needs the HS2 matrix run. Conflicts: `arena/lib.rs` and `live.rs` held now; `combat/exchange.rs` and `fight.rs` have small deletions in the tree.

### F9. Capes and banners (RZ7, VX16, PX35)
- Unwired: `physics/cloth.rs` 1,587 lines 20 tests (rewritten fresh today), no fighter or kit item references it.
- Missing wire: pin to the shoulder bones in `fighter_look`, step in `run_sim_ticks`, re-upload the cape mesh per frame (`Presenter::upload_mesh` exists); banners are kit items.
- Cost M. Hash N. Conflicts: `fighter_look.rs` hunk (serialize with F3, F10).

### F10. Sword in hand and eyes on the rival (LF12 rest, LF13, AM25)
- Unwired: `anim/fbik.rs` 1,334 lines 12 tests (arm chains, grip, reach), `anim/face.rs` 836 lines 13 tests, `anim/springs.rs` 689 lines 10 tests (hair and cloth bones); LF12's foot half landed through `solve_limb` in `anim/ik.rs` and `fighter_look.rs` (`3065d343`).
- Missing wire: `slot_pose` (`fighter_look.rs:1597`), after the foot solve. Not checked: whether `fbik`'s chain names match the CT7 skeleton.
- Cost M. Hash N. Conflicts: `fighter_look.rs` (serialize).

### F11. The lit frame's remaining passes (EG11, EG12, EG13, LF5, LF3 landing 2)
- State: the live frame now runs forward + sky (LF1) + IBL (LF2) + GTAO (LF3 first landing) + bloom (LF4); `encode_sun_depth` exists (`submit.rs:387`), `sun_shadow.rs` 987 lines is in progress. Unwired: `shadow_cascade`, `shadow_cluster`, `fog` (47 items), `aa_scale` (TAA/SMAA, 33), `gi_probe`, `baked_gi`.
- Rows: EG11, EG12, EG13, LF5, EG8, VR1.
- Cost M each. Hash N. Conflicts: render is the most held area (one render writer at a time, per `render-look.md`); do not start a second writer.

### F12. A crowd in the stands
- Unwired: `game/crowd_gpu.rs` 677 lines 10 tests (the reacting crowd sim; `App` runs it and `staged_fight` only exports its 64 levels), `render/crowd_inst.rs` 1,323 lines 14 tests, `render/crowd_impostor`, `render/crowd_seats`, `anim/crowd_scale.rs` 581 lines 9 tests.
- Rows: none for the draw (CT38, CT44, CT50 are content). A new row is below in spirit; not in the top 10 because it needs a render pass.
- Missing wire: an instanced draw fed by `app.crowd_levels` in `present_frame`. Cost L. Hash N. Conflicts: render held.

### F13. Quality tiers (potato and web) as a real switch (GS10, WEB8, PLF10)
- Unwired: `render/quality.rs` 320 lines 4 tests (no caller; the row says PLF10 is unbuilt), `physfx` `Tier` caps (used by `BreakLook::new(tier, ..)` only).
- Missing wire: one tier value chosen at startup in `arena.rs`, read by the presenter (scale, MSAA, shadows, GTAO) and by `PhysFx::new`. Cost M. Hash N. Conflicts: `presenter.rs` held. Open owner decision 5 (mip bias) sits here.

### F14. Dedicated server and internet play (ON30, MP6 real leg)
- Unwired: `net/dedicated.rs` 1,352 lines (`HeadlessServer`, `Beacon`), `quic.rs` 2,168, `relay.rs` 2,033, `online.rs`, `scale.rs`, `interest.rs` 1,767, `courier.rs`, `wan_soak.rs`; only the `net-bout` proof harness runs them.
- Missing: a server binary the window joins (write M), the real two-machine run (human). Cost M-L. Hash N. Conflicts: net is the most held crate now (15 files plus untracked `browser`, `websocket`, `connect_race`). Do not touch.

### F15. The browser (WEB5 to WEB9, WEB12)
- State: `crates/web` shell with WebGPU device exists (WEB4 done); `web/src/arena_view.rs`, `audio.rs`, `input.rs`, `browser.rs` and `net/src/browser*`, `websocket*` are untracked, so another session is mid-work. Unwired: `assets/bundle.rs` 1,397 lines 16 tests (`BundleLoader`; the web needs content by fetch, WEB6), `assets/stream.rs`.
- Cost L (write, mostly). Hash N. Conflicts: held. Skip in this triage beyond noting `look/metric.rs` (section 2) for WEB5.

### F16. Explosions, arrows, thrown objects (RZ6)
- Unwired: `physfx` `apply_blast`, `spawn_thrown`, impact (landed `43e800d`, 11+7 tests), `combat/projectile.rs` 1,241 lines 17 tests, `destruction/bonded/blast.rs`.
- Missing: the duel is melee; nothing in the arena emits a blast or a projectile, so the authority event has no producer. Needs a game decision first. Cost M. Hash Y if the blast becomes authority. Conflicts: `physfx` held.

### F17. Collapsing buildings
- Unwired: `destruction/structure.rs` (used only by an endpoint pack), `spread.rs`, `csg.rs`, `shape_match.rs`, `cluster.rs`, `crack.rs`, `shards.rs`. The arena breaks one bonded crate (`BondedCollection::crate_prop`). Rows: DX series, PX16 to PX19.
- Missing: an arena prop set (walls, towers) as bonded collections, then F1's picture for each. Cost L. Hash Y. Conflicts: `destruction/bonded.rs` has a 6-line edit.

### F18. Material table readers (RZ8 rest)
- State: table landed and `physfx` plus the walker read it (`3b01824`). Remaining: the cue picker in the window is F4. Cost S. Hash N for the picker; the controller read is hash-sensitive (default material must equal today's constants).

### F19. Machine eyes (G4, LF15, WEB5, LJ1, EG30)
- Cut, see section 2: `playtest/look/*`, `machine_eyes`, `look_score`. Restore `metric.rs`, add the caller in `arena_capture.rs` (the capture path writes a manifest). Cost M. Hash N. Conflicts: playtest has 3 small edits.

### F20. Animation depth
- Unwired: `anim/motion_match.rs` 1,189 lines 13 tests, `traversal.rs` 1,049 lines 17 tests, `exert.rs` 902 lines 11 tests, `pack.rs` 1,488 lines, `loco_bake.rs`, `space.rs` 945 lines, `motion_net.rs`, `net_pose.rs` (used by `netgame` only), `injured_double.rs`. Rows: AM, LM, TV, NS23. Gain is feel, not a gate; park until F3, F9, F10 land. Cost M each. Hash N.

### F21. AI brains beyond the depth brain
- Wired: `DepthBrain` (arena), `Mind` (`ludus_npcs`, playtest). Unwired: `ai/dialogue.rs` 1,829 lines, `bt.rs` 1,262 lines (used by `bout.rs`), `goalstack.rs` (no user, 2 tests), `legion.rs` 1,582 lines (only `proof/online_soak`), `eqs.rs`, `memory.rs`, `gladiator.rs` NetPolicy (`distill/fps.rs`, `nn`). Rows: GA5, GA9, NV5, LW, PLN19. Frozen with urban war (FOCUS D7) except what the duel needs. Cost M-L. Hash Y for anything that changes AI decisions in a recorded match.

### F22. Navigation
- `crates/nav` (tiles 2,151 lines 24 tests, mesh, links, bake) has no dependents. The arena has no rooms, the depth brain steers directly. Rows: NV4, NV6, NV10. Park until a map needs paths; restore `crowd.rs` and `funnel.rs` then. Hash Y.

### F23. World cells, terrain, scatter
- `world/cells.rs` 1,524 lines 11 tests, `terrain.rs`, `scatter.rs`, `ground/bake.rs`; consumers are `game/modes_world.rs`, `siege.rs` (which the door's `build_game` uses), `net/cells.rs`. Not reachable from the window. `parked.md` says parked. Cost L. Park.

### F24. Asset pipeline for web and potato (RZ5, VR2, VR4, WEB6)
- Unwired: `assets/bc.rs` 777 lines (BCn), `meshlet.rs` 999 lines 3 tests, `optimize.rs`, `collider_cook.rs` 1,365 lines 14 tests (RZ5 asks for it), `recook.rs`, `lod_swap.rs` (used by `windowed_dressing`), `vat.rs`, `museum.rs`, `clip_receipt.rs`, `archive.rs`. Wire path is `bundle_cook` and the kit import. Cost M each. Hash N, except collider cook feeding the controller (Y). Conflicts: assets not held.

### F25. First-person view
- `game/fps_view.rs` 614 lines 12 tests (no external reference), `anim/trails.rs` `FpsRig` 1,061 lines 10 tests, `spatial/fps_visibility.rs`. Plan: `fps-view.md`. Needs a look channel beside the hashed input (hash Y if it enters the tape). Park unless the game is an FPS.

### F26. Second game modes: siege, Ludus world, Steam
- `game/siege.rs` 1,167 lines, `modes_world.rs` 1,204 lines, `ludus_npcs.rs` 1,193 lines, `steam.rs` 353 lines, `ladder.rs` 980 lines. Reachable only from the door (`endpoint/build_game.rs`). AIS1's `siege` template wants them. Cost M-L. Park.

### F27. AI-easy setup (AIS1 to AIS4, DOC3)
- `endpoint` packs are the registry; `packs/web.rs` is being added now; `playtest` (237 items, 5,000+ lines of eval, tournament, frame_check, keypoint) has no dependent crate, so the door cannot call it. Wire path: an endpoint dependency on playtest for the rows that name it, or a CLI. Mostly write (AIS1 `game.toml`, templates). Conflicts: endpoint held (27 files).

### F28. Native-only render extras
- `render/accel.rs` (RT acceleration, 43 items), `baked_gi`, `gi_probe`, `rt_shadows`, `restir_di`, `fluids_gpu`, `gpu_cull`, `executor`, `graph`. Potato target and the browser rule these out of the first pass. Park; keep tests running.

### F29. The Rapier settling track (`arena` feature `sim`)
- Enabled by nothing (`game/Cargo.toml:14` says so on purpose); `cargo test -p engine-arena --features sim --lib` has 1 red test (`loc2_wired_slope_step_air_course_x2`, owner decision 7). It is a second, hash-different movement path. Do not wire; decide to keep or drop.

## 4. (a) The ten wirings that change the picture soonest

Order is payoff over time-to-first-result. 1, 2, 4, 6 can run in parallel (separate files). 3, 9, 10 all write `fighter_look.rs`: serialize them (3, then 10, then 9). 8 waits for 1 and 3 so the picture half has something to extend. Rows use `RZ4w`-style IDs: the parent ID plus `w` (wire slice), the pattern `finish-line.md` already uses for slices. Heavy cargo goes through `node tools/gh-check.mjs cargo test ...`.

1. RZ4w (F1, break picture), S-M, no hash risk.
2. EG14w (F2, HP bars), S.
3. RZ2w (F3, ragdoll), M.
4. EG21w (F4, sound), S-M.
5. EG44b (F5, LAN from the window), M, waits for `MatchSpec`.
6. EG27w (F6, pad), S.
7. NS11w (F7, blood and dust), M.
8. RZ3w (F8, dismemberment authority), L, hash risk, V2 only.
9. RZ7w (F9, capes), M.
10. LF12w (F10, sword in hand), M.

Paste-ready rows (sprint file in brackets):

```
[docs/SPRINT-22-web-realism.md]
- [ ] RZ4w game: the break reaches the window (slice 2 of RZ4) — `break_look.rs` is finished and unused. `engine-arena` gets one additive `pub fn` that returns the crate prop's rest chunks, bonds and anchors before the first step (no state change); `App::simulate_tick` (`crates/game/src/lib.rs:1134`) copies `stepped.breaks.events` into a new `TickOutcome.breaks`; `bin/arena_frame/mod.rs::run_sim_ticks` calls `BreakLook::observe(tick, &outcome.breaks)` after `tick_into`; `ArenaApp::init_presenter` uploads `BreakLook::meshes()`; `ArenaApp::push_fighters` appends `BreakLook::frame(alpha)` draws and drops the intact kit mesh named in `hidden`; `--no-break-look` turns the picture off. Diff: regular. Tier A. Size S (landings: the accessor and the outcome field; the window calls). Owns the call sites named here and the accessor; `break_look.rs` is held by another session, land after it. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib break_wire`: a 600-tick scripted duel breaks the crate, the checkpoint hashes are equal with the picture on and off, after the break tick the draw list holds debris pieces and not the intact mesh, bodies stay at or under the tier cap; `arena --exit --frames 300` prints a `physfx: bodies=.. contacts=.. us=..` line with bodies above 0. F2P: `rg -n BreakLook crates` hits only `break_look.rs`. P2P: golden `d57b5c8ae3096614` and `cargo test -p engine-proof break_corpus` unchanged. needs: RZ1.

[docs/SPRINT-21-look-feel.md]
- [ ] EG14w game: hp bars in the live window (slice of EG14; the overlay pipeline and `Presenter::push_hp_bar` exist) — `ArenaApp::present_frame` (`bin/arena.rs:1417`) calls `stage_hp_bars(presenter, &duel_overlay_for_app(&self.app))` before `render_surface`, as `bin/arena_capture.rs:783` already does for captures; remove the "overlay rows stay parked" comment at `arena.rs:1189`. If the surface encode path skips the overlay, the render hunk waits for the render writer and this row stays open with that named. Diff: regular. Tier A. Size S. Owns the call in `arena.rs` and a test. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --bin arena hp_bars_ride_the_surface_list`: the surface draw list for a live duel stages one back-and-fill pair per fighter row, and an `arena --exit --frames 60` capture shows hp-bar pixels in their screen rect. F2P: the live window stages no hp bar (`rg -n stage_hp_bars crates/game/src/bin` hits only `arena_capture.rs`). P2P: golden unchanged. needs: EG9.

[docs/SPRINT-22-web-realism.md]
- [ ] RZ2w game: a floored fighter falls as a ragdoll in the window (slice 2 of RZ2) — new `crates/game/src/ragdoll_look.rs` (about 250 lines): a `Rig` for the CT7 skeleton with a `BoneShape` per bone from one table; `PhysFx::spawn_ragdoll(slot, rig, pose, velocity, impulse)` on the tick `FighterLook::observe_live_balances` (`fighter_look.rs:1422`) sees `GaitPhase::Fallen` or the slot's `FightCue::Knockdown` (velocity from the last two `FighterView`s, impulse from the attacker-to-victim direction times damage); `PhysFx::step()` once per accepted tick in `run_sim_ticks`; `slot_pose` (`fighter_look.rs:1597`) reads `transforms(alpha)` for a ragdolled slot; the get-up clip blends back through `Inertial` when balance leaves Fallen. Diff: good. Tier A. Size M. Owns `ragdoll_look.rs` and the `fighter_look` ragdoll hunk (one writer with the Look agents). Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib ragdoll_look`: a scripted knockout settles within 3 s with every bone above the sand, the checkpoint hashes are equal with the layer on and off, 8 simultaneous knockouts stay under the tier's ragdoll cap. F2P: no binary names `spawn_ragdoll`. P2P: golden unchanged. needs: RZ1.

[docs/SPRINT-21-look-feel.md]
- [ ] EG21w audio: the window hears space, surface and tension (slice of EG21) — `feed_tick_audio` (`crates/game/src/windowed.rs:858`) moves the listener with the camera (`FeedTx::listener`), gives each `play` the hit position from `FighterView` so `SpatialMixer` pans and attenuates, picks the cue name from `combat_cues::Surface` (SN12), sends `FeedTx::tension` from `outcome.layers` and the crowd bed gain from `outcome.bed_gain` (both computed every tick and read by nothing in `bin/arena*`). `content/audio` holds footsteps, impacts and ambience only: crowd, music and stinger cues stay absent and are counted loudly as `unknown_cue`, never silent, until GS23 fetches them. Diff: regular. Tier A. Size M. Owns `feed_tick_audio` and the call in `arena_frame::run_sim_ticks`. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib window_audio_space`: two hits at opposite sides pan opposite, a far hit is quieter, tension rises with a knockdown and decays, the feed's `sent` equals `applied`. F2P: `rg -n "tension\(" crates/game/src` finds no caller. P2P: golden unchanged. needs: none.

[docs/SPRINT-20-game.md]
- [ ] EG44b game: two people play from the window (slice of EG44, after `MatchSpec` lands) — `arena --host` and `arena --join CODE@ADDR` build a `NetGame` (`netgame.rs:342` and `:378`) from the same `MatchSpec` the window uses; `ArenaApp` holds either an `App` or a `NetGame`; `arena_frame::run_sim_ticks` calls `NetGame::tick_local` and the per-frame `poll` and `flush` instead of `App::tick_into`; the remote body draws from `remote_frame(alpha)`. Two human slots, UDP, LAN. Diff: good. Tier A. Size M. Owns the flags in `bin/arena.rs`, the session switch in `bin/arena_frame/mod.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib window_net_loopback`: a host and a guest built through the window's own constructor end 600 ticks on equal checkpoints through a 3-tick-latency 2% loss link; then the human run: `node tools/mp6-check.mjs` plus one two-machine run posted under `target/`. F2P: `arena --join` does not parse. P2P: golden unchanged. needs: EG44 (MatchSpec).

[docs/SPRINT-20-game.md]
- [ ] EG27w game: the pad works in the `arena` binary (slice of EG27) — `Gilrs::new()` in `ArenaApp::new` as `run_windowed` does at `windowed.rs:1298` (that function has no caller; keep it as the smoke leg or delete it in the same commit), one `pad::poll_gilrs_once` per wall step, `pad::merge_pressed` into `self.pressed` before `fighter_input_from_pressed` in `run_sim_ticks`. Diff: regular. Tier A. Size S. Owns the pad hunk in `bin/arena.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --bin arena pad_moves_the_human_slot`: recorded stick events move slot 0 on the tape the same as the keyboard wish; with no device nothing changes. F2P: `rg -n Gilrs crates/game/src/bin` finds nothing. P2P: golden unchanged. needs: none.

[docs/SPRINT-21-look-feel.md]
- [ ] NS11w game and render: hits and breaks leave marks (slices of NS11, LF6, LF7, GS19) — landing 1 (game, S): `Hit` cues become `decals::GoreEvent`s and `BreakEvent`s become dust emitter requests in `run_sim_ticks`, gated by `combat::gore::GoreSetting` (Off, Reduced, Full give one golden); landing 2 (render, M, one render writer): the bind probes for decals and particles in `present/gpu_present/submit.rs` become draws, wound maps from `wound_map.rs` for the CT7 bodies. Diff: good. Tier A. Size M. Owns the cue mapping in `arena_frame` and the two draw hunks. Accept: `node tools/gh-check.mjs cargo test -p engine-render --lib decal_live` (pass-truth reads LIVE for decals and particles: pixels differ on vs off) and `cargo test -p engine-game --features app --lib gore_cues` (the same tape gives the same marks twice, gore Off draws none). F2P: pass-truth lists decals and particles as PROBE. P2P: golden unchanged. needs: LF3.

[docs/SPRINT-22-web-realism.md]
- [ ] RZ3w arena: a cut happens in the live duel (authority half of RZ3, rules v2 only) — in `Sim::try_step_into` (`crates/arena/src/lib.rs`, after the hit loop near line 1625) build a `BladeSwing` from the sweep (`WeaponSweep`, already used in `match_wire.rs`) and `JointTarget` capsules from the victim's balance pose in integer mm; call `combat::resolve_swing`; then `limbs::spawn_limbs(&mut self.limbs, &cuts, debris_live)`; expose the tick's cuts in `LiveTick` beside `breaks`. The picture half stays RZ3: one `spawn_limb_of(Flesh, ..)` per cut, a stump cap, a decal. Diff: hard (hash). Tier A. Size L (landings: the swing builder; the spawn and the `LiveTick` field; the V2 golden and the HS2 matrix). Owns `arena/src/lib.rs`, `arena/src/limbs.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-arena --lib live_cut`: a scripted V2 duel loses at least one limb, twice equal; the V1 golden `d57b5c8ae3096614` does not move; the new V2 golden is pinned; the HS2 matrix is equal on the three native targets. F2P: `rg -n "spawn_limbs|resolve_swing" crates/arena/src` hits only tests. P2P: `break_corpus` x2 unchanged. needs: RZ1, GP32 (live duel runs v2).

[docs/SPRINT-22-web-realism.md]
- [ ] RZ7w game: eight capes on the fighters (slice 2 of RZ7) — pin `physics::cloth` at the CT7 shoulder bones in `fighter_look`, step at 60 Hz inside `run_sim_ticks`, re-upload the cape mesh each frame (`Presenter::upload_mesh`); the arena kit banners are the second user. Diff: good. Tier A. Size M. Owns the cape hunk in `fighter_look.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib cape_look`: eight capes step under 0.5 ms in release on a runner (printed), no NaN after 600 ticks in wind, the checkpoint hashes are equal with capes on and off; a capture shows a cape behind a running fighter. F2P: no code in a binary names `Cloth`. P2P: golden unchanged. needs: RZ2w (one writer on `fighter_look.rs`).

[docs/SPRINT-21-look-feel.md]
- [ ] LF12w anim and game: the sword stays in the hand and the head follows the rival (slice of LF12 and LF13) — run `anim::fbik` arm chains and the grip solve from `fighter_look::slot_pose` (`fighter_look.rs:1597`) after the foot solve of `3065d343`, with a soft-limit look-at on head and chest toward the rival (AM25). Not checked: that `fbik` chain names match the CT7 skeleton; if not, a name map is the first landing. Diff: good. Tier A. Size M. Owns the arm and look-at hunk in `fighter_look.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib hand_look`: hand-to-grip distance stays under 2 cm across a swing clip, the head yaw tracks a circling rival within the soft limit and never snaps past 120 degrees per second, hashes equal. F2P: `rg -n BodyChains crates` hits only `fbik.rs`. P2P: golden unchanged. needs: LF12 (feet half), RZ2w.
```

## 5. (b) Code that is truly dead and should stay deleted

- physics solver stack: `solver`, `broadphase`, `substep`, `islands_build`, `cast`, `simd_math`, `checked_fix`, `verify`, and their `tests/it/physics_*` (about 5.2k lines with tests). Rapier replaced them. Reason on file: parity of the golden and ragdoll tapes on four targets.
- net `lagcomp`, `guard`, `anticheat`, `replicate`, `server`, `resim` (about 4.4k lines). Rollback with refused late inputs makes lag compensation a no-op in this design; replication and the old server are replaced by snapshot, delta and `HeadlessServer`. Write a thin input admission check at ON5 instead of restoring `guard`.
- net `nat`, `steam_relay` (687): unless a Steam ship is chosen.
- game `camera_feel`, `camera_stack`, `eight_fight`, `modes`, `window_bind`, `quest` (about 1.3k lines), `ballistics` (1.2k) until a siege mode exists.
- destruction `debris` (682), `bond_grade` (993): superseded by `physfx` and `break_look`, and an offline calibration that never ran.
- perf `pgo_harness`, `server_budget` (1.5k): `net/tick_budget.rs` holds the bars.
- nav `astar`, `hpa`, `neighbors` (485). Keep `crowd` and `funnel` in git for NV4 and NV6.
- Not cut but dead for the plan today, keep the tests running and do not wire: the Rapier settling track (F29), `render/accel` and the ray-tracing family (F28), `ai` `dialogue`/`goalstack`/`legion` until the living-world or urban-war plans unfreeze (F21), `world` (F23), `game/steam.rs`.

Do not delete anything else on the strength of the scan; section 1 shows how it misleads.

## 6. (c) How much of the remaining plan is wire and how much is write

Method (a sample, not an audit): the 30 rows of `node tools/board-slice.mjs --focus --open`, read for the file they name, plus the status page's static check of the 960 open rows (207 name a file git has never held).

- FOCUS Now, 30 rows: wire-dominant 13 to 15 (LF2, LF3 landing 2, EG14, RL21, VS1, EG21, LF14, EG25, EG27, EG29, GS10, GS19, ON30 partly, EG11 and EG13 half), write-dominant 15 to 17 (SP63, EG12, EG17, OPT27, GS5, GS6, GS13, GS14, GS2, GS18, GP37, EG41, FC1, FC3, LF17 partly). About 45% wire.
- All 960 open rows: at least 207 (22%) are new files, so write. My estimate for the rest: a third wire, two thirds content, tools and tuning. So about **25 to 30% wire overall**.
- The ten wirings in section 4: eight are wire (1, 2, 4, 5, 6, 7 game half, 9, 10), two have a real write inside (3 needs a bone-shape table and an impulse rule, 8 is a new authority path). Measured in lines I expect under 1,500 new lines for rows 1, 2, 3, 4, 6 together and about 3,000 for all ten without 8 and the render half of 7.
- By finish-line gate: G11 (physical) is about 85% wire if limbs are excluded; G8 (multiplayer) is wire plus a human run; G3 (lit frame) is wire in progress, one writer; G4 (look score) is restore plus a caller; G10 (web) is mostly write; G7 (AI door) is mostly done and not a wiring question.

So: wiring makes the next two weeks of visible progress cheap (the window would show breaks, bodies that fall, hp bars, sound with space, a second player, a pad), but it does not shrink the plan much because the plan is mostly things that do not exist yet.

## 7. Method, limits, and what I did not check

- Greps used token matching on non-test text with `#[cfg(test)] mod` as the cut, tests directories and `*_test.rs` files excluded. "No caller" means no other file's code names it, comments stripped. Reflection, macros and string-named registries would hide a caller; the endpoint packs are the known case, so I did not judge endpoint or perf from the scan.
- Not checked: any run of cargo or a window; the surface path of the HP bar overlay; whether the kit's `crate_a` is the bonded crate; whether `fbik` names match the CT7 skeleton; the per-file contents of the 16 removed render files; `editor`, `provider`, `observe`, `ecs`, `save`, `ui` unwired items (grouped under F27, F28 or left out); cost numbers are my reading, not measured.
- `tools/wiring-scan.mjs` would count better with a module-aware cut and comment stripping; I did not edit it. A one-line fix is in the regex of `prodOf`: cut at `#[cfg(test)]` followed by `mod`.
- Index note for the lead: `tools/plan-index.test.mjs` requires one line per file in `docs/plan/README.md`, and this file has none (I was told to edit nothing else). Suggested line: ``- `wiring-triage-2026-10-17.md` — NEXT — all four — what exists but nothing calls: the cut verdicts, 29 unwired features with their missing call sites, ten wirings with paste-ready rows``. The VISION.md Plans map may want a row too.

## 8. Files other sessions hold now (working tree, about 20:15)

Modified or untracked, by area: **render** `src/lib.rs`, `present/gpu_present/live.rs`, `presenter.rs`, `present/look.rs`, untracked `capture.rs`, `scope_ledger.rs`, `procedural_arena.rs`; **net** 15 files (`dedicated`, `delay`, `delta`, `interest`, `lib`, `lobby`, `online`, `quic`, `scale`, `session`, `Cargo.toml`) plus untracked `browser*`, `websocket*`, `connect_race.rs`, `session/`; **web** the whole crate; **physfx** the crate (blast, impact, caps, check, types, tests); **physics** `controller2.rs`, `material.rs`; **game** `break_look.rs`, `bin/arena.rs`, `windowed.rs`; **arena** `lib.rs`, `live.rs`, untracked `spec.rs`; **destruction** `bonded.rs`; **combat** `exchange.rs`, `fight.rs`; **ai** `goalstack.rs`; **nav** `mesh.rs`, `tiles.rs`; **endpoint** 27 files; **audio** `heal.rs`. Not held: anim, assets (tests only), fighter_look and the rest of game, combat except two files, playtest except three.

Collisions for the ten rows: 1 (`break_look.rs`, `arena.rs`), 2 (`arena.rs`), 5 (`arena/live.rs`, `lib.rs`), 7 (render), 8 (`arena/lib.rs`, `live.rs`). Rows 3, 4, 6, 9, 10 are free today.

## Appendix A. Module-level islands (106 files), by crate

Each line: unused distinctive names / distinctive names, file lines, path. Generated by my pass in section 1; endpoint, perf, proof and editor are tool or registry crates and are listed for completeness only.

```
ai: dialogue 12/15 1829, goalstack 8/8 427
anim: blend 4/4 512, crowd_scale 12/13 581, events 5/6 625, exert 6/7 902, fbik 16/16 1334, injured_double 6/7 397, net_pose 7/7 409, pack 15/18 1488, space 8/10 945, trails 16/20 1062, warp_ground 18/21 756
app: live_proof 3/3 245, snapshot 5/6 442
arena: kit_zone 3/3 778, limbs 4/4 591
assets: bc 15/18 777, clip_receipt 9/11 874, extras 3/3 253, gltf 7/7 533, meshlet 9/9 999, museum 13/14 806, optimize 7/7 394, recook 6/7 882, sever_cook 10/10 656
audio: coherence 10/12 822, hrtf 11/12 861, voice 14/16 453
combat: feel_instrument 8/10 1073, hitsim 3/3 361, projectile 14/14 1241, reaction 7/7 633
ecs: bridge 9/11 1235, parallel 3/3 589
editor: app 13/15 2261, durable 6/7 1440
endpoint: assert_run, build_level_session, canon, door_dispatch, intents, packs/{anim,break_,gym,material,net,perf,physics,play,replay,view,web}, plan_goals, secret_launch, watch_ledger (14.8k lines, registry)
game: break_look 6/7 1067, fps_view 9/10 615, ladder 13/16 981, modes_world 14/17 1205
geo: recipe 4/4 711
material: channels 4/5 342
nav: links 5/5 263
net: online 8/10 734, scale 8/10 1106, webtransport/cert 4/5 635
observe: event_queue 16/20 733
perf: baselines, core_bench, feel, fight_cost, profiler (4.0k lines, tool crate)
physics: cloth 14/15 1587, kcc 4/5 899, ragdoll_tape 5/6 669
playtest: charter 8/9 377, spatial_shapes 6/6 283
proof: dataset, fb_ship, m1, matrix, tape_fuzz, trace (4.1k lines, tool crate)
provider: arbiter 12/14 919, router 6/6 468, stub 9/9 420
render: arena_day 6/7 368, debug_draw 7/8 285, draw_id 4/4 207, executor 15/18 1142, gpu_cull 4/5 627, material_pipeline 7/8 358, pass_truth/harness 3/3 696, present/gpu_present/sun_shadow 6/7 987, twin_arena 11/12 899, v1_temporal 13/16 662, wound_map 9/10 470
save: persist 5/5 207, rollback 6/7 313
scene: prefab 6/7 1229
spatial: compose 7/8 1432, exposure 5/5 534, fps_visibility 3/3 340, probe 6/7 491
ui: text 11/13 869
world: ground/bake 5/6 421
```

Some of these are false positives of the name heuristic (for example `anim/warp_ground` and `physics/kcc` have users through type paths I did not resolve, and `arena/kit_zone` is used by the arena through `use` of short names). Treat the appendix as a list to open, not a list to delete.
