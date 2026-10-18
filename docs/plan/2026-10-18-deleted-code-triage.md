# Deleted-code triage (2026-10-18)

**Rule: never delete a module that a board row names, open or ticked; mark the row obsolete first.**

Owner question: "what else got deleted because nothing called it? we are mid-development, maybe not wired." Scope: the 54 files in the 2026-10-16/17 rot cuts that are still gone (net and nn left out: net has its own restore wave, nn was cut by the owner). Read-only: no code was touched. Old bytes: `git show <sha>^:<path>` (sha in each section; every file below was read from there).

How each verdict was checked. Replacement: `rg` for the old public names across `crates/ content/ src/ tests/` (only three files kept names: wound_queue in `wound_map.rs`, match_db's feature names in `motion_match.rs`, blockout's `effective_seed` in `scatter.rs`; every other replacement is by job, not by name) plus a read of the candidate file. Rows: grep of every `docs/SPRINT-*.md` shard, `docs/plan/*.md`, `docs/archive/`, and `node tools/ticked-rows-audit.mjs`. Port effort: the old file's `use crate::` / `use engine_*::` names were matched against the tree today (name level; signatures and `cargo check` were NOT run, so "compiles" is unproven). Earlier pages (`wiring-triage-2026-10-17.md`, `status-2026-10-17.md`) were re-checked, not trusted; where they differ, this page says so.

## Summary

| Crate | Files | REPLACED | DEAD | LATER | RESTORE |
|---|---|---|---|---|---|
| anim | 2 | 1 | 0 | 1 | 0 |
| assets | 1 | 0 | 1 | 0 | 0 |
| destruction | 3 | 2 | 1 | 0 | 0 |
| game | 9 | 4 | 3 | 1 | 1 |
| nav | 3 | 1 | 2 | 0 | 0 |
| perf | 2 | 1 | 1 | 0 | 0 |
| physics | 10 | 7 | 2 | 1 | 0 |
| playtest | 7 | 0 | 3 | 0 | 4 |
| render | 14 | 2 | 6 | 4 | 2 |
| world | 2 | 1 | 1 | 0 | 0 |
| docs/archive | 1 | 0 | 1 | 0 | 0 |
| **total** | **54** | **19** | **21** | **7** | **7** |

Nine more files are gone and were not on the list given for this triage (two are net); section "Also gone" covers them. One more RESTORE comes out of it: `render/stream.rs`; `look.rs` and `look/manifest.rs` join RESTORE 1.

### The 8 to bring back, in order (first six are RESTORE, last two are the best LATER)

| # | What | Why now | Effort |
|---|---|---|---|
| 1 | playtest look judge: `look/metric.rs` + `look/manifest.rs` + `look.rs` + `look/judge.rs` (4.1k lines) | `look.judge` is a registered door verb that always answers `unsupported_look_judge` because no host registers a judge; G4 (machine look score) and WEB5 (browser frame vs native frame) have no SSIM in Rust | M |
| 2 | playtest `look_score.rs` + `machine_eyes.rs` (361 lines) | the per-capture machine score G4 needs; sits on #1 | S |
| 3 | game `modes.rs` | the Windows window has no fullscreen or resolution handling at all (only `with_inner_size`); WIN8 names it | S |
| 4 | render `clip_hold.rs` | the only temporal-hold (boil/ghost) gate; PQ21 and RP10 build on it | S |
| 5 | render `uploads.rs` | staging-belt model RA16 names; pairs with #6 | S |
| 6 | render `stream.rs` (extra, not on the list) | OPT17 texture streaming; `stream_pool_mb` is set per quality tier (200/350/700/1400 MB) and nothing reads it; the potato target needs it | M model, L wiring |
| 7 | physics `vehicle.rs` (LATER, urban war) | UC19 "extend the D1 raycast vehicle"; 621 std-only lines, no imports | S |
| 8 | game `ballistics.rs` (LATER, siege and urban war) | PX41 and the ticked WD10 name it; `combat/projectile.rs` has a stepper but no caller either | L |

### Surprises

1. `game/package.rs` was "restore at GM3" in the 10-17 triage. It should not be restored: `tools/ship.mjs` (2,182 lines, `pack`, `stage`, `run`) re-implements `assemble_full`, `verify_clean` and `parse_manifest` in Node and says so in comments, with `tools/ship.test.mjs` on top. GM3 is blocked by the five-minute clean run, not by a missing packager.
2. `endpoint/src/packs/view.rs` has the whole `look.judge` verb, schema check and a `register_look_judge` hook, but nothing outside a test calls the hook, and `engine-endpoint` has no `engine-playtest` dependency (that edge is open row AD8). So the verb is a promise with no judge behind it. `playtest/Cargo.toml` still carries the `image` dependency with the comment "Look judge PNG decode", and `tools/iqa-gate.mjs` mirrors the deleted `metric.rs` integer SSIM, so a parity vector exists.
3. `content/fighters/window_bind.json` is data with no loader (only a doc comment in `render/twin_fighters.rs` names it). Two shaders are orphans: `render/src/shaders/fog_vol.wgsl` and `skin.wgsl` have no Rust reader.
4. Open rows with a command that cannot pass today: WIN8 (`--lib modes`), RA16 (`--lib uploads`), PX41 (`--lib ballistics`), E38 (gi_dyn leg), GP8, CT29. See "Open rows".
5. `combat/projectile.rs` `step_volley` says "production volley step" in its header and has zero callers outside its own file, so the PX41 premise (two steppers disagree) is moot until a siege or urban-war caller exists.

## Tables

Verdicts: REPLACED = a live file does the job (cited), nothing to do. DEAD = superseded design, spike, or owner cut. LATER = a future game wants it, no caller now (file stays recoverable). RESTORE = a named caller exists today.

### anim

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| dq.rs (`65593c69`) | 434 | dual-quaternion skinning option for twist joints | none: `fighter_look.rs` skins with linear blend through `render/v1_skin.rs::SkinPipeline` | AM7 [x] (no open row) | LATER | realistic fighters (shoulder twist); caller would be the skin step in `fighter_look.rs`; S, std-only, hash-free |
| match_db.rs (`77730b11`) | 483 | float motion-match pose DB | `anim/src/motion_match.rs` (`bake`, `search`, `db_hash`, integer) and `motiondb.rs` | `plan/animation.md` s6 says delete it once LM1 lands (LM1 is ticked) | REPLACED | none |

### assets

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| inherit.rs (`77cd0a8d`) | 229 | copy-from def resolver (parent links, last key wins) | none | none | DEAD | no row, no caller; recover by sha if def inheritance is ever wanted |

### destruction

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| debris.rs (`3387a96e`) | 682 | baked debris field woken by hits | `physfx/src/fx.rs::spawn_debris_of`, `game/src/break_look.rs::BreakLook`, `destruction/src/budget.rs::DebrisLedger`, `debris_lod.rs::plan_lod` | DX6 [ ] (its own file is `debris_lod.rs`, which exists) | REPLACED | none |
| nav_dirty.rs (`3387a96e`) | 419 | per-break dirty-set record for nav tiles, plus link recook list | `destruction/src/bond_grade.rs::break_record` / `BreakRecord.dirty_tiles` (DX9 progress), `nav/src/links.rs::rebake_for_set`, `rebake_links` | DX10 [x] (archived, no code); UW13, UW22, UC4 [ ] say "dirty tiles (DX10)" | REPLACED | re-point UW13/UW22/UC4 at `break_record`; `docs/BLOCKERS.test.mjs` already guards the old path |
| wall_slice.rs (`3387a96e`) | 395 | offline siege-wall bake, bytes, decode | `destruction/src/structure.rs::oracle_wall`, `building`; `snapshot.rs::capture/restore` | none | DEAD | UC4 (building support graph) is the new design |

### game

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| ballistics.rs (`d89bd98e`) | 1174 | fixed-point siege projectile sim, impacts into destruction | `combat/src/projectile.rs` (`step_round`, `step_volley`) has the flight but no caller; `game/src/siege.rs` keeps only the wall ledger | WD10 [x]; PX41 [ ]; urban-war.md says "integrated" | LATER | siege mode or urban-war rifle fire; imports (`engine_core::fixed`, `engine_destruction::bonded/spread/budget`, `engine_net::interest`, `crate::siege`) all still exist by name; L (needs a mode to call it); touches no golden state (own digest) |
| camera_feel.rs (`d89bd98e`) | 201 | follow rig plus damping | `game/src/camera.rs::frame_pair`, `shake_offset`; `camera_rig.rs::damper_blend` | GP8 [ ] (its own note: "not restored, already covered") | REPLACED | close GP8 |
| camera_stack.rs (`d89bd98e`) | 232 | priority shot table with eased handoff | camera director in `camera.rs` | none | DEAD | header already said "retired, no production caller" |
| cine.rs (`d89bd98e`) | 651 | intro/outro cinematic player | none | none (its caller `flow.rs` is now 6 lines) | DEAD | no row; recover by sha if a title or mission-intro row appears |
| eight_fight.rs (`d89bd98e`) | 211 | 8-fighter 30 fps gate numbers | `perf/src/baselines.rs` (`eight_fighter_tick_stays_in_budget`), frame bars in `crates/perf` | none | REPLACED | none |
| modes.rs (`d89bd98e`) | 764 | display mode list, ranking, validated swap keeping last-good | none (grep for fullscreen, monitor, refresh finds only `with_inner_size` in `bin/arena.rs:2206`) | WIN8 [ ]; WIN2, GP17 [x] (ticked, no code); GP32 [x] names it, apparently by mistake (that row is about rules tables v2) | RESTORE | see detail 3 |
| package.rs (`d89bd98e`) | 1596 | versioned Windows tree, manifest, `verify_clean` | `tools/ship.mjs` (`pack`/`stage`/`run`, mirrors `assemble_full`, `verify_clean`, `parse_manifest`) + `tools/ship.test.mjs`; bundle in `assets/src/bundle_cook.rs`; `endpoint/src/build_game.rs` | SL39 [x] (FNV tidy, file gone); GM3 [ ] still open (the clean run) | REPLACED | none; if a Rust-side check is wanted, port `verify_clean` markers only |
| quest.rs (`d89bd98e`) | 247 | ladder as a quest chain | `game/src/ladder.rs` | none | DEAD | no row; urban-war missions are UW34/UW35 (planner, career) |
| window_bind.rs (`d89bd98e`) | 380 | placement JSON parse plus pin checks for the hero and two opponents | `game/src/fighter_look.rs` (skinned fighters in the presenter), `fighter_bind.rs`, `render/src/twin_fighters.rs` | CT29 [ ] (capture pair only); CT29a [x] (no code) | REPLACED | `content/fighters/window_bind.json` is now an unread file; either read it in `twin_fighters.rs` or delete it |

### nav

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| astar.rs (`3ed325c8`) | 170 | 8-dir grid A* | tiled navmesh `nav/src/mesh.rs` + `funnel.rs` | NV11 [x] ("the legacy grid A* was deleted on purpose") | DEAD | deliberate, row says so |
| hpa.rs (`3ed325c8`) | 176 | hierarchical grid A* | same (navmesh tiles) | none | DEAD | grid-only; the nav moved to meshes |
| neighbors.rs (`3ed325c8`) | 139 | uniform-grid neighbour shortlist for crowds | `nav/src/orca.rs` (own counting-sort `Grid::cell_of`) | NV4 [x] landed | REPLACED | none |

### perf

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| pgo_harness.rs (`65e2da7c`) | 551 | before/after wall ms for a frame slice | none; `Cargo.toml` `[profile.ship]` (fat LTO) | OPT5 [x] (profiles only) | DEAD | own header: "test-only, no `--pgo` flag" |
| server_budget.rs (`65e2da7c`) | 912 | 64 fighters + 500 AI server tick p99 under 4 ms | `net/src/tick_budget.rs` (`record_tape`, `TickQuantiles`, `BUDGET_P99_MS`) | PF8 [x] names the perf file | REPLACED | re-point PF8 at `tick_budget.rs` |

### physics

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| broadphase.rs (`3d6ee4a1`) | 212 | grid + SAP sweep | Rapier `BroadPhaseBvh` in `physics/src/rapier.rs:137`; `spatial/src/index3.rs` | E41 [ ] (obsolete, see `2026-10-17-obsolete-rows.md`) | REPLACED | none |
| cast.rs (`3d6ee4a1`) | 187 | named float-to-int casts | none | none | DEAD | CQ10 hygiene; the rust-quality ratchet holds the cast count |
| checked_fix.rs (`3d6ee4a1`) | 110 | saturating narrowing for the tick | `core/src/checked.rs::saturating_i64_to_i32` | none | REPLACED | none |
| fps_move.rs (`4d10c40d`) | 230 | integer Quake walk | `physics/src/controller2.rs` (`Walker`, `TickInput`, integer mm) | none | REPLACED | none |
| grapple.rs (`3d6ee4a1`) | 1171 | clinch grip servo with breakaway | different design now: `arena/src/spear.rs` (spear tether authority, `docs/plan/spear-grapple.md`), `physfx/src/tether.rs` (rope), `physics/src/clinch.rs` (parting only, new and uncommitted today) | PX3 [x] (no code) | REPLACED | the grip-servo idea has no caller; PX3 should say so |
| islands_build.rs (`3d6ee4a1`) | 247 | island topology and hashes | `physics/src/islands.rs::build_islands` (came back `bc72d03d`) | PX6 [x] | REPLACED | none |
| simd_math.rs (`3d6ee4a1`) | 150 | lane-unrolled f32 folds | none | none | DEAD | premature; Rapier path |
| solver.rs (`3d6ee4a1`) | 2059 | sequential-impulse solver | Rapier through `physics/src/rapier.rs`, `ragdoll.rs`, `physfx` | EG5 [x] (timing-assert row); E28 [ ] obsolete | REPLACED | none |
| substep.rs (`3d6ee4a1`) | 784 | explicit substep schedule | Rapier `PhysicsPipeline` / `IntegrationParameters` (`rapier.rs:22,27`) | PX31 [x] (no code) | REPLACED | PX31 should say so |
| vehicle.rs (`3d6ee4a1`) | 621 | raycast vehicle, Pacejka-lite tires, integer angle tables | none (Rapier 0.36 also ships a raycast vehicle controller; the old one is integer-angle for LAN replay) | D1 [x] (no code); UC19 [ ] (Tier C) | LATER | urban war vehicles; caller would be `physics/src/armor_vehicle.rs` (UC19); S port (no imports); own hash, not the golden |

### playtest

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| agreement.rs (`f1a4f9d7`) | 373 | advisory two-green-exports agreement probe | none | AX17 [x] (advisory, no code) | DEAD | advisory only, never wired |
| live_probe.rs (`f1a4f9d7`) | 92 | adapter to the production `engine_spatial::probe` | none (`probe_sync.rs` now scores only its mirror probe) | SP32 [x] (no code) | DEAD | tiny; recover if the live-vs-gold leg is wanted |
| look/judge.rs (`f1a4f9d7`) | 2266 | nine-item look rubric L1-L9 from pixels, `LookJudgement`, JSON | none | LJ1, AD1 [x] (no code); AD8, AD24, LAI5, PQ21 [ ] | RESTORE | detail 1 |
| look/metric.rs (`f1a4f9d7`) | 1110 | integer SSIM, deltaE, histograms, PNG decode | `tools/iqa-gate.mjs` mirrors the integer semantics (Node only); `proof/tests/capture_ssim_windows.rs` is a float test helper | EG30 [x] (no code); LF15, PQ21 [ ]; WEB5 [ ] needs an SSIM | RESTORE | detail 1 |
| look_judge.rs (`f1a4f9d7`) | 196 | first slice of the judge (L1 only) | the full `look/judge.rs` | LJ1 [x] | DEAD | superseded by the full judge; do not restore this one |
| look_score.rs (`f1a4f9d7`) | 169 | one capture scored against its reference | none | EG30 [x] (no code) | RESTORE | detail 2 |
| machine_eyes.rs (`f1a4f9d7`) | 192 | counts and scores each captured PNG | none | EG30, RL62 [x] | RESTORE | detail 2 |

### render

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| baked_halton.rs (`f737386e`) | 279 | Halton-sampled lightmap bake | `render/src/gi.rs` bake + `baked_bake.rs::bake_arena` | RL5 [ ] owns `baked_gi.rs` | DEAD | alternate sampler, no caller after `baked_gi` dropped it |
| clip_hold.rs (`f737386e`) | 392 | 120-frame hero push-in as data plus ghost/shimmer oracle | none (`post/taa.rs` has test scenes only) | RP9 [x] (no code); RP10, PQ21 [ ] name it | RESTORE | detail 4 |
| clouds.rs (`f737386e`) | 308 | volumetric clouds over the sky bake | none (`sky.rs` has no clouds) | VX2 [ ] (Tier C, frozen) | LATER | open-sky maps and weather (urban war, wet sand); M |
| draw_id_reduce.rs (`f737386e`) | 278 | per-body pixel counts from the draw-id target | `render/src/draw_id.rs` (slice 1, wired in `procedural_arena.rs`, `gpu_present/frame.rs` test) | SP63 [x] covers slice 1 only | LATER | GPU-side occlusion for `game/view_truth.rs` ("occluded" waits for SP65, which chose a CPU ray); S |
| fog_vol.rs (`f737386e`) | 1194 | analytic volumetric fog plus a cloud slice | `render/src/fog.rs`, `present/fog_fwd.rs`; VX17 plans a new froxel pass | VX1, VX17 [ ] (new file `volumetric.rs`) | DEAD | `shaders/fog_vol.wgsl` is left orphaned |
| gi_dyn.rs (`f737386e`) | 357 | SDFGI cascade CPU spike (flag-gated) | none | E38 [ ] | DEAD | spike; E38's gi_dyn leg should be dropped |
| restir_gi.rs (`f737386e`) | 201 | ReSTIR-GI reservoir oracle (spike) | `render/src/restir_di.rs` is direct light only | RT4 [ ] (Tier C), RT11, RT14 | LATER | high tier dynamic GI; caller would be the RT4 pass |
| splat.rs (`f737386e`) | 215 | Gaussian-splat backdrop rasterizer | none | VX7, VX11 [ ] (Tier C, LATER) | LATER | skyline backdrops for urban war; S |
| sprite_pack.rs (`d6adbd2f`) | 155 | shelf rect packer for sprite atlases | `geo/src/uv.rs::unwrap` (xatlas chart packing) | none | DEAD | its `mod` was removed before the cut |
| tonemap.rs (`f737386e`) | 243 | CPU mirror of `tonemap.wgsl` for hue/dither gates | the shader stays (`present/gpu_present.rs:39`, `lib.rs:436`) | SL3 [ ] (CPU twins go) | DEAD | test mirror only |
| tunnel_mouth.rs (`f737386e`) | 642 | bind glue: sunlit sand vs dim tunnel mouth exposure | `render/src/arena_day.rs`, `sky.rs::exposure_ev100`, `fog.rs` | RL20 [x] (no code), RL22 [ ] says "untouched" | DEAD | RL20 never had its GPU capture pair; say so on the row |
| uploads.rs (`f737386e`) | 408 | generational staging-belt model, eviction counters | none | RA16 [ ] owns it; OPT17 [ ] "uploads on the RA6 belts" | RESTORE | detail 5 |
| v1_vfx.rs (`f737386e`) | 871 | CPU dust/torch particles plus blood decals on the v1 path | `render/src/particles.rs`, `particles_gpu.rs`, `decals.rs` (`GoreEvent`, `ScreenDecal`) | SL3 [ ] wants CPU twins gone | REPLACED | none |
| wound_queue.rs (`f737386e`) | 204 | wound paint order and UV dilation | `render/src/wound_map.rs` (same names: `WoundKind`, `UvRect`, `dilate_uv`) | none | REPLACED | none |

### world

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| blockout.rs (`a7d2af50`) | 61 | recipe-keyed seed for the height bake | `world/src/scatter.rs::effective_seed` (same function) | E64 [ ] ("blockout fold" done) | REPLACED | close that leg of E64 |
| wetness_spike.rs (`a7d2af50`) | 371 | EXPERIMENT virtual-pipes wetness | none | none | DEAD | its own header: "spike only, never the tick" |

### docs/archive

| file | lines | did | replacement | rows | verdict | caller / effort |
|---|---|---|---|---|---|---|
| `.../archive-root/crates/physics/src/character.rs` (`750435eb`) | 165 | old kinematic controller timers | `physics/src/controller2.rs` | VX3 matched the name by accident | DEAD | an archive copy, not a code cut |

## RESTORE detail

Frozen hash: every file here is presentation or tool side, none touches hashed integer state, so golden `d57b5c8ae3096614` cannot move. "Imports" means names found in the tree today, not a compile.

1. **Look judge bundle** (`look/metric.rs` 1110, `look/manifest.rs` 476, `look.rs` 203, `look/judge.rs` 2266). Effort M. Imports: `crate::frame_check::{check_frame, CapturedFrame, FrameFault, FrameMetrics, Thresholds, MAX_FRAME_DIM}` exist, `image` 0.25 png is still in `playtest/Cargo.toml`. Caller: `crates/endpoint/src/bin/e2040-mcp.rs` calls `register_look_judge(...)` (`endpoint/src/packs/view.rs:128`); needs the AD8 optional `engine-playtest` edge first. F2P: `cargo test -p engine-playtest --lib look_metric` (EG30 Accept: a frame equal to its reference gives SSIM 10000 and deltaE 0, a sand region brightened 20 percent gives SSIM under 9800), `look_judge` (LJ1), and a vector test that SSIM x10000 on one fixed PNG pair equals `node tools/iqa-gate.mjs`; then `cargo test -p engine-endpoint --test look_judge_hook` against the real judge. Order: metric, then manifest and `look.rs`, then judge, then host glue. Unblocks G4, WEB5, LF15, PQ21, AD1, AD8, AD24.
2. **Machine score** (`look_score.rs` 169, `machine_eyes.rs` 192). Effort S, after 1. Caller: the hook seat already exists, `endpoint/src/play.rs::capture_with_look_hook` (`LookHook`, one call per validated PNG; its doc names `arena.rs::capture_offscreen` as the remaining caller), so `MachineEyes::note_frame` plugs in there; needs the same playtest edge as 1. F2P: EG30 Accept "the hook runs once per captured frame" plus a scored capture x2 equal.
3. **`game/modes.rs`** (764). Effort S: std-only, zero imports. Caller: window creation in `crates/game/src/bin/arena.rs` (around line 2206) and the settings screen `ui/src/settings.rs`; swaps only at frame boundaries (old module doc). F2P: WIN8 Accept `cargo test -p engine-game --lib modes` (a bad mode swap keeps the last good one). The wrong GP32 mention must be edited out of GP32 first.
4. **`render/clip_hold.rs`** (392). Effort S. Imports `crate::post::{MotionField, TaaParams}` exist. Caller: `render/src/present/look.rs` (its old consumer, twin establishing view). F2P: `cargo test -p engine-render --lib clip_hold` (the old file had 3 tests, among them `hold_clip_resolves_without_boiling`). PQ21 then adds the flicker model beside it.
5. **`render/uploads.rs`** (408). Effort S for the model. Caller: the heal path `render/src/heal.rs` (old doc: `invalidate_all` on resize or device loss) and the texture upload sites in `ibl.rs`, `baked_gi.rs`. F2P: RA16 Accept `cargo test -p engine-render --lib uploads` (a 64-player storm holding 64 MiB live, with eviction telemetry). Do it with the stream.rs pair in "Also gone".

## Ticks with no code

Ticked rows whose file is gone (board lies). The verdict column is the verdict of the file.

| row | file | verdict | note |
|---|---|---|---|
| AM7 | anim/dq.rs | LATER | test `dq_option` gone |
| PF8 | perf/server_budget.rs | REPLACED | `net/tick_budget.rs` holds the bars |
| AX17 | playtest/agreement.rs | DEAD | not in the audit (path not named) |
| SP32 | playtest/live_probe.rs | DEAD | not in the audit |
| LJ1 | playtest/look_judge.rs, look/judge.rs | RESTORE | `look_judge` filter finds 0 tests |
| EG30 | playtest/look/metric.rs, look_score.rs, machine_eyes.rs | RESTORE | `look_metric` filter finds 0 tests |
| AD1 | playtest/look/judge.rs | RESTORE | stale-capture guard |
| RP9 | render/clip_hold.rs | RESTORE | |
| RL20 | render/tunnel_mouth.rs | DEAD | GPU capture pair was never made |
| D1 | physics/vehicle.rs | LATER | |
| PX3 | physics/grapple.rs | REPLACED | spear.rs + tether.rs |
| PX31 | physics/substep.rs | REPLACED | Rapier |
| EG5 | physics/solver.rs | REPLACED | Rapier |
| DX10 | destruction/nav_dirty.rs | REPLACED | `bond_grade.rs::break_record` |
| WD10 | game/ballistics.rs | LATER | |
| SL39 | game/package.rs | REPLACED | `tools/ship.mjs` |
| CT29a | game/window_bind.rs | REPLACED | json left without a loader |
| WIN2, GP17 | game/modes.rs | RESTORE | only in the archived shards (the audit reads them) |
| HR2 | render/live.rs (extra) | see "Also gone" | |
| NV2, NV10 | nav/crowd.rs (extra) | REPLACED | `nav/orca.rs` (NV4); NV10's own text admits the missing parts |

NV11 (astar) and NV6 (funnel) are not lies: NV11 says the grid A* was deleted on purpose.

## Open rows that name a gone file

| row | names | state |
|---|---|---|
| WIN8 | modes.rs | keep; RESTORE 3 |
| RA16, OPT17 | uploads.rs, stream.rs | keep; RESTORE 5 and 6 |
| PQ21, RP10 | clip_hold.rs, look/judge.rs, look/metric.rs | keep; RESTORE 4 and 1 |
| LF15, AD8, AD24, LAI5 | look.rs, look/metric.rs, look/judge.rs | keep; RESTORE 1 |
| PX41 | ballistics.rs | mark obsolete or re-scope: file and premise gone, `step_round` has no caller |
| GP8 | camera_feel.rs | mark obsolete: its own note says covered by `camera_rig.rs` |
| CT29 | window_bind.rs | re-scope to the capture pair only |
| E28, E41 | solver.rs, broadphase.rs | already in the obsolete-rows page |
| E38 | gi_dyn.rs | drop the gi_dyn leg (spike) |
| E64 | blockout.rs | close the blockout leg (folded into `scatter.rs`) |
| UW13, UW22, UC4 | "dirty tiles (DX10)" | re-point at `bond_grade.rs::break_record` |
| UC19 | vehicle.rs | keep (LATER, Tier C) |
| RT4, VX2, VX7, VX11, VX1, VX17 | restir_gi, clouds, splat, fog_vol | keep (LATER, Tier C); VX1 and VX17 do not need `fog_vol.rs` |

## Also gone, not on the list given for this triage

| file | lines | sha | verdict | note |
|---|---|---|---|---|
| render/stream.rs | 566 | `f737386e` | RESTORE (6) | demand-paged tile LRU for textures (the ladder spike: 74.6 percent fewer uploads); OPT17 owns it; `quality.rs` `stream_pool_mb` has no reader; caller: texture upload path in `render/src/present/gpu_present/`; M model, L wiring |
| render/live.rs | 1389 | `f737386e` | REPLACED in progress | HR2 [x] has no code; assets `recook.rs` still publishes `ReloadNotice` and `game/reload.rs` applies tuning at frame boundaries; NS31c3 [ ] writes a small `MaterialLive` in `material_pipeline.rs` "the HR2 pattern". Use `git show f737386e^:crates/render/src/live.rs` as the donor, do not restore all of it |
| render/skin.rs | 628 | `f737386e` | LATER | skin, eye and hair shading; VX3 (Tier C, characters up close) owns `character.rs`; `shaders/skin.wgsl` is orphaned |
| nav/crowd.rs | 1491 | `3ed325c8` | REPLACED | `nav/src/orca.rs` (NV4 landed) |
| physics/verify.rs | 70 | `3d6ee4a1` | REPLACED | NS4 progress: `physics/tests/it/oracle.rs` (new, in the working tree) |
| playtest/look.rs, look/manifest.rs | 203, 476 | `f1a4f9d7` | RESTORE (1) | needed by `look/judge.rs` |
| net/guard.rs, server.rs | 846, 497 | `3151bd46` | not triaged | net wave owns them |
