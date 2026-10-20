# Render look (Look)

Outcome: moving fighter captures on the RTX 3050 stand beside the references.
Proof: (1) `node tools/finish.mjs` shows A4, A5, A8 met; (2) Q1: V1-V3 PNGs from one release window build, rubric L1-L9 beside `docs/VISUAL-TARGET.md`, 8 of 10 hero shots pass, 0 validation errors; (3) arena capture at frame 60 shows kit, textures and two fighters in loco poses, refuses the cube SHA and the 64x48 noise SHA, frame p95 at most 16.6 ms at 1080p, golden `d57b5c8ae3096614` unchanged.
Order: EG8 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14, then RL21 VS1 VS4 VS3 RL26 WIN6 RA14 GX13 Q1 GM3 (FOCUS Now, Look).
Crates: render, game, anim, assets, proof, playtest.
Open board IDs: section 7 (Now rows first, then the table).

Tag NOW (Look). Prefixes: RL, RP, RA, GX, Q1; merged `VS` (arena wiring) and `WIN`, `GM` (window and packaged run). Plain summary: the arena lights on a real device and the frame graph submits forward, shadow, post, sky, material, particle and decal passes. Still open: the look gate (fog depth feed, sky/IBL/fog drift, baked GI capture, TAA-off seam), the arena wiring (the window still draws cubes) and the clean-machine packaged run (GM3, BLOCKED). The live frame still runs forward + blit only: sky/IBL binds staged, GTAO/bloom/MSAA-targets/overlay-draw open (EG/LF chain, section 7).

Status 2026-10-17: disk shows the row S5 false half still holds. `rg -n encode_post crates/render` returns 0 hits (exit 1), so the `:9` "submits forward, shadow, post, sky, material, particle and decal passes" line still overstates the graph; the live frame runs `crates/render/src/present/gpu_present/live.rs:1172` `encode_forward` then `:1221` `encode_blit` with no post pass between (also `:1352` then `:1430`); the only `run_frame` caller outside `post.rs` is `crates/render/src/aa_scale.rs:2603`, inside `#[cfg(test)] mod tests` opened at `:1833`, so post has no live caller; particles and decals stay bind probes (`crates/render/src/particles_gpu.rs:764` "The bind probe below still owns the submit; this slice stages math only." and `crates/render/src/decals.rs:979` "Bind probe over the decal uploads"), so the S5 gap still holds.
Status 2026-10-21: the S5 gap still holds, but the line numbers above have drifted - `crates/render/src/present/gpu_present/live.rs` is 3,191 lines and `encode_forward` is at `:1298` (then `encode_blit` at `:1358`, then `:1548` / `:1647`); `crates/render/src/aa_scale.rs` is 2,485 lines, its `#[cfg(test)] mod tests` opens at `:1232` and the only `run_frame` caller outside `post.rs` is `aa_scale.rs:2000`; `run_frame` itself now lives in `crates/render/src/post/chain.rs:923` (the CQ27 post split moved it out of `post.rs`, so the old `post.rs:2262` cite no longer resolves). `rg -n encode_post crates/render` still returns 0 hits (exit 1) and the particles/decals bind probes are still probes, so the gap is unchanged.

Status 2026-10-17 (render code removed on purpose): the render dead-code cut removed 16 source files and 2 test files in `f737386e`, whose subject is "render wip sweep": `live.rs`, `tunnel_mouth.rs`, `clip_hold.rs`, `v1_vfx.rs`, `clouds.rs`, `restir_gi.rs`, `splat.rs`, `stream.rs`, `uploads.rs`, `skin.rs`, `fog_vol.rs`, `gi_dyn.rs`, `baked_halton.rs`, `tonemap.rs`, `draw_id_reduce.rs`, `wound_queue.rs`. This was a cut, not a sweep accident: rot packets `0-rot-render-orphans` and `0-rot-render-dead-r2` list them as modules with no production user; the first cites the owner's 2026-10-16 rule "delete the code, keep the row, note the recover path". Their `mod` lines and tests left with them. Recover with `git show f737386e^:crates/render/src/<file>`. Rows still ticked with no code behind them: HR2 (live shader and material swap with last good), RL20 (`tunnel_mouth.rs`), RP9 (`clip_hold.rs`). Open rows that still name removed files: RA16, CQ23, RT4, VX2, VX7, OPT17, PQ21. (`present/gpu_present/live.rs`, the live frame, is a different file and exists.)

Status 2026-10-17 (HR2, owner call): nothing in `crates/` names `LiveSet` or `apply_notice` now (0 grep hits); the removed `live.rs` had 1,389 lines. Open row NS31c3 copies its last-good pattern and finish gate A10 lists HR2. Choose: restore `live.rs` with a real caller, or re-open HR2.

Status 2026-10-17 (board vs this plan): WIN6 is ticked on the board (staged-fight raster frames, commit `3697e065`), but the list in section 7 still shows it open. RL21, RL26, VS1, VS3, VS4, RA14, GX13, Q1, GM3 and the chain EG8 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 are open. Checked today on the GitHub runner: `cargo test -p engine-render --lib` 704 passed, 0 failed, 13 ignored (run 39025228340). Not checked: any capture on the RTX 3050.

Status 2026-10-17 (capture seal, owner call): `seal_capture_cut_log` in `crates/game/src/bin/arena_capture.rs` prints a flagged director cut log (jump, line cross, short hold, slew, stale) and still seals the capture. A plain 600-tick brawl already shows jump flags, so refusing would fail normal captures. The doc on `App::director_audit` ("gates the capture seal") and a test comment ("this session is refused") say refuse; the code reports. Choose refuse or report, then fix the other text.

Status 2026-10-17 (potato mip bias, owner call): `crates/render/src/quality.rs` (GS10; no caller yet, PLF10 is unbuilt) holds `mip_bias_x100` 100 on Potato, Comfort and High, so its "potato <= comfort <= high" test passes flat. The GS10 row asked for +1.0 on Potato only. That is larger than the other tiers, so the test would have to run the other way for this knob. Choose: keep it flat, or Potato +100 with Comfort and High lower and the test reversed.

Status 2026-10-17 (owner: the arena looked meh, and the game must run in a browser): the web build needs `engine-render` split into `gpu` and `window` (WEB5), our compute shaders checked against WebGPU default limits (VR3), a web quality tier (WEB8) and KTX2 textures (VR2). VR1 is the join of LF1 LF2 LF3 EG11 EG12 EG13 into one pass-truth table that must read LIVE on native and web. One render writer at a time still holds. Plan: `docs/plan/web-and-realism.md`.

Merged 2026-10-10 (batch A of the plans review): `docs/archive/2026-10-10/plans/arena-slice.md` as `### arena wiring`, `docs/archive/2026-10-10/plans/shipping.md` as `### window and packaged run`, `docs/archive/2026-10-10/plans/deepdive-Q1-2026-10-08.md` and `docs/archive/2026-10-10/plans/deepdive-RL5-2026-10-07.md` as notes under section 7. Dated status lines, the donor table and the deep-dives are in `docs/archive/2026-10-10/plans/render-look-cut.md`.

Pre-merge text: `git show d96091cd:docs/plan/render-look.md`; also `docs/archive/2026-10-10/plans/arena-slice.md`, `docs/archive/2026-10-10/plans/shipping.md`, `docs/archive/2026-10-10/plans/deepdive-Q1-2026-10-08.md`, `docs/archive/2026-10-10/plans/deepdive-RL5-2026-10-07.md` (each `git show d96091cd:docs/plan/<name>`; the four are archived under `docs/archive/2026-10-10/plans/`).

## 1. Where it stands

GPU-live column per the GX6 truth gate: a `render` row is done only when
its pass submits on a real device in the window build. CPU math plus
llvmpipe parity is the merge gate, never done. Rows that are ticked are
dropped here (git history and the board keep them).

Refresh 2026-10-15 (code as it is, no builds): rust-quality FAIL on
render — casts 1011->1013, dup 8->9, passthrough 1->2 (hot lane,
blocked), long 26->28, big 6->7; wiring islands up (render +251 lines);
CQ9 12 files over 1500 lines, CQ10 4236 casts, CQ14 93 string errors,
CQ7 147 render allocs, CQ24 post jitter split. Focus rows LF1-LF4,
EG11-EG14, SP63, FC1/FC3, RL21, VS1 all open; ready queue holds 5
packets (EG14/EG45/FC3/LF2/VS1 closes). Live frame still forward + blit
only; sky/IBL binds staged, GTAO/bloom/MSAA-targets/overlay-draw open.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| IBL live (GX3) | landed-but-OPEN | code submits; row open pending `FORWARD_IBL` wiring proof + reference-GPU material chart (truth rule) |
| GTAO live (GX5) | landed-but-OPEN | code submits; row open pending wiring proof + 0.8 ms bar on reference GPU |
| Height fog + contact math (RL7) | unit-tested, landed-but-OPEN | `fog.rs` (`FOG_PASS`/`CONTACT_PASS`, `from_sky_capture` ramp, `compile_fog_for_forward`, `submit_gpu_fog`; 7/7); row open: production depth-feed + device evidence missing |
| Sky-forward sampling (RL8) | coded, awaiting acceptance | `sky.rs` (`RL8_LOOK_BAKE_THREAD`, `SKYBOX_LOOK_BUDGET_MS`, `FORWARD_SKYBOX_WGSL`, `SkyLookBake`, `skybox_sample`); board `[ ]` open |
| Sun-scatter tint spike (RL10) | missing (rowed, Tier C) | boarded open; needs RL9 |
| Baked GI math (RL5) | unit-tested, landed-but-OPEN | `baked_gi.rs` (mask + tunnel seal + frame bind over `ibl.rs` SH, never a second SH path; bake x2); row open: 60 fps + reference-GPU capture missing |
| SMAA + dynamic resolution math (RP4) | unit-tested, landed-but-OPEN | `aa_scale.rs` (SMAA 1x edge/blend, hysteresis controller, `compile_taa_off_for_present/scale`; 11/11); row open: production seam + scale-hold proof missing |
| Per-bin draw with transform (RL60) | missing (rowed, Tier A) | boarded open; base RL32 needs it |
| One hue-keeping tonemap (RL34) | missing (rowed, Tier A) | boarded open; six ACES copies today |
| Compiled material in forward (RL46) | missing (rowed, Tier A) | boarded open; needs RL46a interface |
| V1-V3 reference captures (Q1) | open | zero reference PNGs on disk per 2026-09-25 truth note; P1 hero look 0/10 until it lands |
| CPU-twin retirement (SL3) | open, owns look oracles | `v1_ibl_shadow.rs`, `v1_lighting.rs`, `v1_vfx.rs`, `gi.rs`, `shadow_post.rs` still reachable from non-test builds; Status 2026-10-17: `v1_vfx.rs` was removed in `f737386e`, the other four are used by production code (checked today), so SL3 stays open |
| Live-frame wiring (code-checked 2026-10-13) | 9 boarded rows, all open | chain EG8 LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14; one-line states below, order in section 7 |
| EG8 pass-truth gate | open, table tests landed | `6ea9df22`, `d8b80809`; masked on/off diff + report still open |
| LF1 live sky | bind landed, proof open | `forward.wgsl:565` `fs_sky` samples the bake; sky-view at forward 15 (`presenter.rs:95`) |
| LF2 live IBL | binds landed, proof open | prefilter + BRDF + SH9 in the forward group (`forward.wgsl:101`, `presenter.rs:1084`) |
| EG11 sun depth | sample side landed, 2048 map + submit open | 4-tap compare (`forward.wgsl:157`); staged map is 256 (`twin_shadow.rs:57`) |
| EG12 ambient + torches | uniforms staged, feed + maps open | 8 torch slots (`forward.wgsl:45`); tangent data in assets, no forward normal/ORM sample |
| LF3 live GTAO | probe-only | live frame runs forward + blit only (`live.rs:253`); `ao_tex` stays 1.0 (`blit.wgsl:49`) |
| LF4 bloom + grade | not wired | no post fields on `Presenter`; the blit does ACES alone |
| EG13 MSAA | switch landed, targets + resolve open | switch + math (`msaa.rs:27`), default off (`presenter.rs:614`) |
| EG14 HUD | ring staged, draw open | `overlay.rs:35` `OverlayRing`; the live frame draws no overlay yet |

Module notes constraining the rows below: one writer for the 32-byte
look uniform (mirrors are a slim violation); `sky.rs` takes plain args
on the twin and reads `gtao` plus graph bytes only behind `window`
(RL8/RL9 move binds, never imports); budgets live once in section 3,
evidence is boosted release clocks or it does not count; SH lives in
`ibl.rs` (RL5 never forks it; UV/pack work belongs to CT2).

### arena wiring

The window build still draws one copper cube and six boxes, although the
arena kit (8 glTF models), four skinned fighters, their clips, a glTF
importer and GPU skinning are all in the repo. No plan owned the wiring
between them; this subsection does. Prefix VS. Owner lanes: `game`, with
one `DrawItem` change in `render`, pose-to-matrices in `anim`, import only in `assets`.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Window and headless twin draw list | integrated, draws placeholders | `crates/game/src/windowed.rs` `smoke_presenter` uploads `engine_render::v1::cube()` as mesh key 1 and `stage_dressing` uploads 6 scaled cubes |
| Arena binary | integrated, draws placeholders | `crates/game/src/bin/arena.rs` uploads `v1::cube()` at three call sites |
| Staged fight frames | integrated, cubes | `crates/game/src/staged_fight_frames.rs` builds its bodies from `v1::cube()` |
| glTF import | unit-tested, not reachable from `game` | `crates/assets/src/mesh_import.rs` `import_gltf_bytes` / `import_gltf_path` behind the `import` feature; `crates/game/Cargo.toml` has no `engine-assets` dependency |
| Fighter "bind" | integrated, checks only | `crates/game/src/fighter_bind.rs` pins bundle and body SHA-256 and checks the files exist; it loads no mesh |
| Imported fighter pose | deferred in code | `crates/game/src/lib.rs` `App::pose`: `PoseSource::ImportedFighter => Err(GameError::ArtDeferred)` |
| Presentation poses | integrated, re-simulates | `crates/game/src/lib.rs` `App::presentation` builds a shortened `ArenaConfig` and calls `run_loco_poses` + `run_presentation` from tick 0 on every call |
| Fighter positions for the camera | scripted | `crates/game/src/camera_feed.rs`: "Scripted two-fighter orbit ... Presentation positions only" |
| Texture upload | GPU-verified, not drawable | `upload_texture` in `crates/render/src/present/gpu_present/presenter.rs` (held for the RL2 material bind group); `DrawItem { mesh, material }` carries factors only, no texture slot |
| GPU skinning | unit-tested, unused by `game` | `crates/render/src/v1_skin.rs` `SkinPipeline` + `GpuSkinned` (behind `window`), magenta error frame on missing skin |

### window and packaged run

The arena runs windowed with display modes, gamepads, crash reports and
menus. What is missing is the real clean-machine run: no packaged run has
ever ticked the 18,000-frame proof, so GM3 stays BLOCKED. Prefixes WIN, GM.
Owner lanes: `game` (`crates/game`), with `ui` and `input` only where a row names them.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Windowed loop (WIN1) | done, integrated | `crates/game/src/windowed.rs` + `crates/game/src/bin/arena.rs`; kept because GM3 needs it |
| Packaged exe + bundle (GM3) | BLOCKED, row open (30-frame REAL/REAL PASS, 5-min still open) | stock 30-frame PASS `C:/e2040-clean/clean-run.log` 2026-10-01 (REAL/REAL, 92 MB 11 entries, RTX 3050, exit 0); the 18000-frame proof needs an exclusive quiet box; needs GM2, WIN1, AS1 |
| Packaged tree vs clean install | REAL dist exists, 5-min clean install NEVER run | `crates/game/src/package.rs`: `target/as7-dist` REAL/REAL; the 18000-frame run was never attempted (binary 5-min wall bound blocks it, see WIN13) - code cut 2026-10-16, recover via git log --follow -- crates/game/src/package.rs |

Module note: GM3 stays OPEN and BLOCKED until the 18000-frame clean-machine
run (A8). Scale cap, read in code: `MAX_PARTICIPANTS: u8 = 32`
(`crates/input/src/fighter.rs`, owner decision GE12 2026-10-20); every
WIN row proves at 16 and names what 64 needs.

Status 2026-10-17 (window and packaged run): the game dead-code cut (`d89bd98e`, an `auto: backup` commit; rot packet `0-rot-game-dead-r1`) removed `package.rs` and its test (the GM3 packaging check, row SL39), `modes.rs` (display-mode manager: WIN2 and GP17 are ticked, WIN8 is open and owns it) and `window_bind.rs` (CT29a). Section 2 text that says `package.rs` checks framing and records digests describes removed code. GM3 is still BLOCKED and open; no packaged run exists.

## 2. Finish line

Tier A alpha (arena readable, seams closed): GX3 landed-but-OPEN, not ticked, GX5 dropped with no board row (wave-56 verdict, not ticked), RL7 feed
(RL9) ticked, RL8 one-bake-three-readers ticked, RP5 seam ticked, plus
new RL11 (look-gate bundle) and RL12 (boosted re-baseline). Bar: every
Tier A pass submits with 0 validation errors; llvmpipe parity within
tolerance; fog matches the sky ramp within `FOG_SKY_RAMP_TOL`; 1080p
frame p95 under 16.6 ms on the reference GPU (PF2 names per-pass ms).

Live-frame chain first: EG8 sets the live/probe flags, then LF1 LF2 EG11 EG12 LF3 LF4 EG13 EG14 land in FOCUS order. Bar: each landing flips its `arena per-pass` slot above 0 ms with 0 validation errors; masked on/off frames differ above the EG8 epsilon; golden `d57b5c8ae3096614` unchanged.

Finish 2026-10-15 = LF1-LF4 + EG11-EG14 + SP63 + FC1/FC3 + RL21 + VS1
ticked in FOCUS order (Look group 1). FC1 posts the cost table first so
every landing names its ms; FC3 (30 fps, 8 fighters, 1080p RTX 3050) is
the Tier A exit; SP63 replaces PNG judgement with GPU numbers; RL21/VS1
put the arena and kit on screen before GX13/Q1 close.

Tier B v1 (look): RL5 bake + RP4 fallback open on board,
plus RP6 (TAA/SMAA selection + Q1 manifest wiring, landed). Bar: baked
arena + 8 fighters holds 60 fps on the reference GPU when RL5/RP4 land;
TAA-off shimmer under `SMAA_SHIMMER_MAX_VAR` and scale holds 60 fps
under load; Q1 V1-V3 captures green with RA8 manifests; bake bytes x2.

Tier C taste (only on measured demand): RL10 tint spike, volumetrics,
parallax probes, AgX A/B. Bar: each lands only with a PF2 profile plus
a reference-GPU A/B pair within tolerance, else killed with numbers. No
rows proposed here beyond the boarded RL10.

CUT (never, no rows): ocean FFT, large-scale vegetation, CDLOD terrain,
meshlets without a profile (VG rows own that), hardware ray queries (RT
rows own that), open-world streaming, multi-GPU, console backends,
visual-scripting hooks. R6 FSR stays parked until a license verifies.

### arena wiring

Tier A alpha (the arena reads as an arena): VS1-VS6 and VS9 ticked. Bar:
the `arena` capture at frame 60 shows sand, stands, walls and gate from the
kit with base-color textures, two fighters at their simulated positions in
their locomotion poses; the frame refuses the cube SHA and the 64x48 noise
SHA; the judge scores the Look rubric L1-L9 beside `docs/VISUAL-TARGET.md`;
golden `d57b5c8ae3096614` unchanged.

Tier B v1 (the arena reads at night and in a fight): VS7, VS8 and the row
changes RA14, WIN6, CT29, GP25 on top of VS. Bar: torches are lights in the
clustered list; the staged 600-tick fight draws real bodies; frame p95 under
16.6 ms at 1080p on the RTX 3050 with 2 fighters.

CUT here: new cook formats (the importer reads the glTF files as they are),
a second scene graph, editor work, streaming.

### window and packaged run

Tier A (shippable, GM3): GM3 closes on the operator run, and only with a
genuine package: profile `release` (`cargo build --release -p engine-game
--features app`); `E2040_PACKAGE_BIN` = the release `arena` exe;
`E2040_PACKAGE_BUNDLE` = the real AS1 `E204BNDL` v1 bundle packed from
`content/` bytes by `engine-assets` (`package.rs` checks framing and records
digests, never decodes payloads); manifest headers flip `STUB` to `REAL` with
per-file digests; launch manifest pins `--frames N --exit`. Bar: the package
is copied to a clean Windows PC with no source checkout, no Rust or Node, no
dev env vars, plays the scripted run and exits 0 with its log. Linux stays
headless-proof only. The 16-byte stub exists only so tests stay deterministic
with env vars unset.

CUT (never): consoles, mobile/2D, WASM ship, server fleet/browser, crossplay,
mod store, phone-home/telemetry, liveops, analytics, chat infra, crash-upload
services (local file only).

## 3. Target design

Data flow per frame (one direction, no cycles):

```
sky bake (RL4, worker thread, bytes x2) -> one capture, three readers
  (RL8: skybox bind, RL2 IBL ambient, RL7 fog ramp)
  -> GTAO half-res + bilateral upsample (RL3)
  -> forward PBR (RA1 twin + window share one executor) reading draws
     (RA3), cascades + cluster grid (RL1), IBL (RL2), lightmap/probe
     ambient (RL5), layered BRDF (RL6), fog + contact (RL7/RL9)
  -> GPU particles vs depth (RP2) -> TAA resolve (RP1, RP5 owns the
     scale seam, RP4 owns the TAA-off fallback) -> decals (RP3)
  -> post bloom/exposure/ACES/grade (RP1) -> present (RA1)
```

Ownership: `crates/render/src/` one file per row (RL3 `gtao.rs`, RL4
`sky.rs`, RL5 `baked_gi.rs`, RL6 `material_layer.rs`, RL7/RL9 `fog.rs` +
`present.rs` seam, RL8 forward skybox bind, RP2 `particles_gpu.rs`, RP3
`decals.rs`, RP4/RP5 `aa_scale.rs` + `post.rs`/`graph.rs` seams). RA2
owns pass order and transient aliasing; look rows allocate through it,
never raw. New rows deepen these files, never fork them.

Threads: look code runs on the render thread; bakes (RL2 prefilter, RL4
capture, RL5 lightmaps) run on worker threads with byte-identical x2
outputs and join before bind. The renderer reads snapshots, never writes
sim state.
Determinism: fixed row-major bake orders, fixed tap orders, f64
accumulation on CPU oracles; same inputs give bit-identical bytes x2.
Per-frame GPU pixels are presentation (HASH-LAW): captures compare with
tolerances on the pinned machine, never the sim hash.

Budgets at 1080p on the reference GPU (boosted clocks, PF2 enforces):
GTAO 0.8 ms, sky 0.4, full post 1.5, particles 1.0, decals 0.5, fog 0.2,
contact 0.3, SMAA 0.5. Transient: HDR 16.6 MB + depth 8.3 + bloom 11 +
history 8.3 (~25 MB after aliasing); look adds at most half-res AO
(4.2 MB) + decal tile (2 MB) + fog pair (20 MB budget) inside the RA2
plan. Resident ceiling 4.5 GiB on the 6 GB tier.

API sketch (landed, rows deepen — never mirror):

```rust
pub struct LookFrame { /* 32-byte image, one writer in graph.rs */ }
pub fn bind_look(pass: &mut RenderPass, frame: &LookFrame);
// sky bake once per frame (worker, bytes x2) -> RL8 binds it three ways;
// RL9 feeds RA1 depth+height into compile_fog_for_forward;
// RP5 feeds the ScaleController into compile_taa_off_for_scale.
```

Landed hunks vs probe-only (2026-10-13): sky-view at forward 15 (`presenter.rs:1097`), IBL prefilter + BRDF + SH9 (`forward.wgsl:101`), sun 4-tap compare (`forward.wgsl:157`), torch 8-slot uniform (`forward.wgsl:45`), MSAA switch default off (`msaa.rs:27`), overlay ring staged (`overlay.rs:35`). Probe-only or missing: GTAO in the live frame, bloom + grade, normal/ORM sampling, 2048 sun map with per-frame submit, MSAA targets + resolve, overlay draw. New work deepens these hunks, never forks them.

LF3 GTAO slice detail (deepen 2026-10-14, direction kept: half-res + bilateral upsample, forward reads `ao_full`).
Status: CPU oracle + GPU submitter landed (`gtao.rs`: `gtao_half_res`, `upsample_bilateral`, `GtaoSubmitter`, `GTAO_HALF_WGSL`/`GTAO_UPSAMPLE_WGSL`); live frame probe-only — `present_frame_live` encodes forward + blit only with `LookPassReceipt::skipped`, `ao_tex` stays 1.0 (`blit.wgsl:49`, `GTAO_OPEN`), and blit slice-1 still multiplies whole HDR by AO (second landing moves it to forward ambient-only, `blit.wgsl:50-53`).
Numbers: 0.8 ms bar (`GTAO_BUDGET_MS`); half 960x540x4 = 2,073,600 bytes inside the 4,200,000 transient cap; CPU/GPU parity tol 0.002; kernel 4 slices x 2 sides x 3 taps, tap step `ceil(radius/3)`, radius <= 64 px, falloff <= 32 m, intensity 0..=1, half is ceil-divide by 2, kernel uniform 40 bytes, graph `gtao-half` -> `gtao-upsample` (`ao_half` -> `ao_full`).
Donor PORT refs: none with bytes — `gtao.rs` is engine-owned (Jimenez 2016 school reimplemented, no donor code; no `PORT()` marker in `gtao.rs`); only pattern ref is Bevy GTAO pass-order ideas per section 4.

### arena wiring

Data flow per frame (one direction, presentation only, never hashed):

```
startup (app leg only):
  placement.json --parse--> KitActor[] --import_gltf_path--> MeshData per primitive
    --Presenter::upload_mesh--> MeshKey; base-color image --upload_texture--> TextureKey
  fighter_bind::bind_all (SHA guard) --> *_body.glb --import--> skinned MeshData + skin
per tick:
  App::tick --> App::fighter_views() (slot, pos_m, yaw, loco state)   [authoritative dataset]
  App::pose_cursor (rolling, VS6) --> LocalPose per fighter
  engine_anim skinning matrices (45 joints) --> GpuSkinned uniform
  DrawList = kit draws (static) + fighter draws (skinned) + torch lights (VS7)
  --> forward + look passes (render-look.md) --> present / capture
```

Ownership: `crates/game/src/arena_kit.rs` (new: placement parse, import,
upload, draw push), `crates/game/src/fighter_draw.rs` (new: body import,
skinned draws, positions from `App::fighter_views`), `crates/game/src/lib.rs`
(`fighter_views`, the rolling pose cursor, `ImportedFighter` live),
`crates/render/src/present.rs` (one optional `TextureKey` on `DrawItem`),
`crates/anim` (pose to matrices, reuses AN1 `skinning_matrices`). The cube
stays only as the visible error mesh for a missing asset, drawn magenta.

Dependencies: `engine-assets` with `import` joins `crates/game` behind the
existing `app` feature only, so the headless default stays std-only (the
`crates/game/Cargo.toml` header promise). Tests that need an adapter skip on
`PresentError::NoAdapter`, as the smoke does today.

Determinism: nothing here enters the sim or the golden hash. Kit import is
deterministic (same bytes, same meshes, same key order); fighter positions are
read from the authoritative dataset, never re-derived; pixels compare with
tolerances on the pinned machine.

Budgets (1080p, RTX 3050, boosted clocks): kit 8 meshes, at most 60k triangles
and 16 draws; fighters hero 40k, opponents 25k triangles (CT7 budgets); skin
upload 45 joints x 64 B per fighter per frame; startup import under 1.5 s cold;
presentation cost per frame flat over a 5-minute match (VS6).

API sketch:

```rust
pub struct KitActor { pub asset: String, pub pos_m: [f32; 3], pub size_m: [f32; 3], pub yaw: f32 }
pub fn kit_actors(json: &str) -> Result<Vec<KitActor>, KitError>;          // placement.json
pub fn stage_kit(presenter: &mut Presenter, root: &Path) -> Result<KitReport, KitError>;
pub fn push_kit_draws(list: &mut DrawList, kit: &KitReport);
pub struct FighterView { pub slot: u8, pub pos_m: [f32; 3], pub yaw: f32 }
impl App { pub fn fighter_views(&self) -> &[FighterView]; }                 // from the tick
pub struct DrawItem { pub mesh: MeshKey, pub material: PbrMaterial, pub base_color: Option<TextureKey> }
```

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). The live window frame draws little of what is built; these rules set the target.
- Draw model: DrawItem = (mesh, material, texture, model). Opaque draws are binned by one u64 key (pass, pipeline, texture slot, mesh); blended draws sort back to front; per-instance data (model, material id, tint) rides an instance-step vertex buffer (RL60, then RL32). Never read instance_index into storage on DX12 count draws (wgpu-hal 30.0.1 drops first_instance there).
- Frame order: sun maps (static kit map once, dynamic fighter map each frame) -> depth prepass -> GTAO half-res -> forward (LessEqual, AO on ambient only) -> sky -> TAA v2 or SMAA -> particles and trails -> bloom -> exposure -> one tonemap -> lens -> dither -> present (RL35, RL41, RL38, RL52, RL53, RL36, RL34, RL39).
- One material text: forward.wgsl keeps vertex, sky, fog, shadow and the IBL fetch; the lit part is the module `crates/material/src/compile.rs` generates; a BRDF is never written twice (RL46). Skin is thin transmittance plus a terminator LUT (RL50); metal gets compensated IBL, specular AO, geometric specular AA and an arena-baked probe (RL47); sand is hex-tiled, walls biplanar (RL51).
- Light layers: ambient = sky SH at the bent normal x sky visibility x multi-bounce AO, plus the baked bounce of the current sun angle and night torch layers; direct = live sun plus up to 32 torch point lights with a finite range. The live sun and torches are never baked (RL40-RL44). Time of day is one integer minute in the match setup.
- Surface state at three sizes, all from the event tape: per part (5 coverage channels, RL48), per world texel (a planar ground-state map over the pit, RL49), per decal for walls and props.
- Crowd: seats are static, cooked into about 256-seat sectors with VAT states and a reaction wave uniform; zero per-seat CPU work (RL54, RL55). Volumetrics move to Tier B for arch shafts only (RL56); potato keeps closed-form fog.
- Pacing is part of the look: vsync by default, uncapped only as a capture flag (OPT35). Quality tiers are WGSL override constants and a three-mode feature ladder, not copied shaders (GS35, GS36). Shaders pass a CPU naga gate in cargo test (CQ23). Debug lines and named debug views read copies taken after the tick (RL58, RL59).
- Budgets to add (1080p, RTX 3050, targets until PF25 measures): shadows 1.0 ms, prepass plus forward at least 10% under today's forward, bloom 0.5, TAA plus post 1.5, particles 1.0, trails 0.15, crowd 1.5 at 20k, froxel fog 0.8, 32 torches 1.5. Render targets 25 MB at 1080p (GS34); the ~25 MB aliasing figure is withdrawn: wgpu 30 has no aliased resources.

## 4. Hard problems

1. One bake, three readers, zero drift. Problem: sky, IBL ambient and
   fog each sample "the sky" and drift apart. Technique: RL8 samples the
   bound RL4 day capture in the skybox, the RL2 ambient and the RL7 ramp
   from the same bake per frame — one capture cannot disagree with
   itself; first-frame hitch avoided on the worker thread. Reference:
   Hillaire 2020 (reimplemented). Donor: none (own bake).
2. Fog that sits on the ground, not in the air. Problem: height fog
   without depth is a fullscreen tint. Technique: RL9 feeds per-frame
   depth + height (+normals) from the RA1 presenter into
   `compile_fog_for_forward`, with boosted-clock budgets vs the
   FOG/CONTACT bars — wiring the seam instead of re-deriving fog.
   Reference: Frostbite / Decima school (reimplemented). Donor: none.
3. AO that does not smear in motion. Problem: full-res SSAO costs ~2 ms
   and trails. Technique: GTAO at half res with depth-aware bilateral
   upsample (Jimenez 2016 school), proven on ghost/shimmer scenes —
   half-res compute, full-res edges. Donor: Bevy GTAO pass order ideas
   only (MIT/Apache-2.0, verified, pattern only).
4. Baked light without leaks on fighters that move. Problem: static sand
   vs moving fighters need one ambient truth. Technique: engine-baked
   lightmaps with validity mask for statics plus SH probes for fighters
   reusing the RL2 SH path (never a second SH) — one SH evaluator,
   tunnel-seal fixture, bake bytes x2. Reference: Stupid SH Tricks.
   Donor: xatlas UV ideas + OIDN offline CLI only (no runtime dep).
5. TAA-off that still holds 60 fps. Problem: temporal AA fails or is
   forced off; raw shimmer is ~0.16 variance. Technique: SMAA 1x
   edge-detect + area blend (Jimenez school) with a hysteresis scale
   controller; TAA stays default, SMAA is the fallback (floor ~0.05 in
   `aa_scale.rs`) — must beat raw ~3x and hold the frame. Donor: Bevy
   SMAA shapes as pattern only (verified).
6. Budgets that survive contact with boost clocks. Problem: debug
   idle-clocks misread every bar by ~7x. Technique: RL12 re-baselines
   all section-3 bars on boosted release clocks with quarantine-split
   timing asserts. Reference: PF2 capture discipline. Donor: none.
7. Passthrough blocked (2026-10-15). Problem: rust-quality passthrough
   1->2 sits in the hot lane, so every render landing risks a red gate.
   Technique: freeze new passthrough helpers, remove or baseline the 2,
   land free-lane rows first. Donor: none.
8. Post split size (2026-10-15). Problem: CQ9 lists 12 files over 1500
   lines and CQ24 splits post jitter; one big post landing will not
   review. Technique: split bloom/exposure/grade into slices under 1500
   lines each behind the same seam, then LF4 binds the chain.
   Donor: none (own split).
### window and packaged run

9. Launch flags that never lie → closed CLI surface parsed as data,
   unknown flags exit non-zero (WIN10, headless argv-in/exit-out).
10. Suspend without desync → pause on suspend, same-tick resume with
    last-good mode in one frame (WIN11, reuses the WIN8 drill shape).
11. Crashes that reproduce → settings digest + mode beside the report so
    the pre-crash tick regenerates (WIN12, fixed bytes).

GM3 is a time problem before it is a code problem: about 91 min of run
time are needed against a 45 min keeper slot, and the reaper kills runs at
30 min (BLOCKERS 2026-10-02).

**Steals 2026-10-12, hard problems.** (a) Fast blades vs history: a sword at 100+ px per frame ghosts under a box clamp; per-object motion vectors from the previous skin palette and a velocity-widened clip box (RL37, RL38). (b) Hue of fire: per-channel ACES moves a torch flame +20 deg toward yellow; one hue-keeping curve (RL34). (c) One ambient truth at every hour: sky, ambient and fog read the same sky capture (RL40). (d) Light leaks through thick stone: bake-time unocclude, dilate and validity, storage-time padding and a linear sampler, run-time bent normal and specular occlusion (RL43, RL44). (e) Blood that stays: a 10-second view-space pool cannot show a bout's history; the ground-state map can (RL49). (f) Fifty thousand spectators without a per-seat cost (RL54).

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. Donor notes
2026-10-15: any donor code may be ported (no licence check); GTAO/SMAA
stay pattern-only (Jimenez school, Bevy pass-order shapes); xatlas/OIDN
offline CLI only, never a runtime dep; `accel.rs` and `gtao.rs` are
engine-owned (no PORT marker); new ports carry `PORT()` + `EVIDENCE.md`.

## 6. Slim

Slim order 2026-10-15 (free lanes first, hot lanes second; at most 8 rows a packet).
- Free lanes first (no passthrough/cast fight): `aa_scale.rs` (RP4/RP5
  seam, TAA-off fallback), `fog.rs` (RL7/RL9 depth+height feed),
  `graph.rs` (RA2 pass order + transient plan), `material_layer.rs`
  (RL46a interface, then RL46 compile), `accel.rs` (proxy/TLAS refit,
  tier-gated, CPU mirror is truth).
- Hot lanes second, unblock order: `post.rs` split first (CQ24 jitter:
  bloom/exposure/grade slices under 1500 lines each), then `submit.rs`
  (EG13 targets + resolve), then `live.rs` (LF3/LF4 bind order), then
  `presenter.rs` (passthrough 1->2 BLOCKED: no new passthrough until the
  2 are removed or baselined), then `ibl.rs` (LF2 constant ambient),
  then `gtao.rs` (LF3 half->upsample->forward ambient-only), then
  `sky.rs` (LF1 bake bind).
- Delete: nothing new in this refresh. SL3 owns the CPU-twin retirement
  (`v1_ibl_shadow`, `v1_lighting`, `v1_vfx`, `gi`, `shadow_post` under
  `#[cfg(test)]` or deleted); look rows never duplicate that execution.
- Merge: RL9 owns fog + presenter seam in one row. RP5 owns the post +
  graph scale seam in one row (the `aa_scale` API stays frozen). RL11
  owns the whole look-gate capture bundle (never per-pass rows).
- Freeze: the 32-byte `LookFrame` image (one writer), the `ibl.rs` SH
  path (RL5 reuses, never forks), `v1.rs` capture format, the section-3
  budget bars (changes arrive with PF2 numbers, never silently).
- What we will not build: a second frame machine, second post chain,
  second SH/AO paths, open-world streaming, multi-GPU, ray tracing (RT
  rows), meshlets (VG rows), ocean/vegetation/terrain, console backends,
  visual-scripting hooks. Composition taste (SP13) never gates rows.
- What we will not build here (EG/LF scope): TAA history (LF5 owns it), crowd (RL54 owns it), volumetrics (RL56 Tier B owns it).

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): RL34-RL62 (display end, shadows, light, materials, particles, crowd, fog, draw model, LOD, temporal history, debug views), GS34-GS36 (live-frame hygiene, feature ladder, override constants), OPT34, OPT35, CQ23, PF25. Row changes: RL27 folds into RL35 (camera cascades stay for the stands only); RL32 needs RL60 (a transform first); R6 FSR unparks (FSR3 is MIT, read 2026-10-12); the expired TAA and PCSS steal lines in steals.md are now RL38 and RL35; RL5 is unfrozen for the RL43 storage fix only.

Rows from the books (2026-10-12, board shard `../SPRINT-20-game.md`; `BOOK-GAPS-2-2026-10-12.md` 2.2): RL32 the live loop sorts and groups draws (opaque front to back by pipeline, material, mesh; blended back to front), RL33 fighter cull bounds follow the posed body (before W7 culls fighters).

Rows live on the board; this section names this plan's open ones. Slice
any row with `node tools/board-slice.mjs <ID>`; the Now rows come first,
in FOCUS order, with their Accept line verbatim.

- [ ] WIN6 staged fight to the presenter. Tier A. needs: -
  Accept: one staged 600-tick run emits 4+ frames with fighters visible; frame manifests differ across frames; CT7 SHAs pinned.
- [ ] RL26 twin 8-fighter draw bind. Tier A. needs: -
  Accept: capture pair before `target/pilot-owner-view/r5-20260929/arena/frame_000014.png` vs after on the reference GPU scores higher by this seat on silhouette + composition with 0 validation errors; golden unchanged.
- [ ] RL21 twin arena-bundle bind. Tier A. needs: -
  Accept: capture pair before `target/pilot-owner-view/r2-20260929/arena/frame_000000.png` vs after on the reference GPU scores higher by this seat on silhouette + materials with 0 validation errors; golden unchanged.
- [ ] VS1 arena kit from glTF in the window and headless twin. Tier A. needs: -
  Accept: F2P a test stages 8 kit meshes with at least 5,000 triangles in total (today 7 cubes, 84 triangles) and the frame-0 capture SHA differs from the cube capture SHA; P2P golden `d57b5c8ae3096614` unchanged, `cargo test -p engine-game --lib` green, headless default builds with no `gltf` in `cargo tree -p engine-game -e normal`.
- [ ] VS4 fighters at their simulated positions. Tier A. needs: VS1
  Accept: F2P for a 2-fighter 120-tick run every frame's fighter positions equal `fighter_views` (test) and at least 2 meshes of over 5,000 triangles are drawn; P2P golden unchanged.
- [ ] VS3 kit textures. Tier A. needs: VS1
  Accept: F2P the sand region of the frame-60 capture has a luminance variance at least 4 times the untextured run; P2P golden unchanged.
- [ ] RA14 arena content through the presenter. Tier A. needs: VS1 VS3 VS4
  Accept: window + twin show sand, stands and fighters on the reference GPU with 0 validation errors; q1-arena120 sequence_fnv moves with motion, never 120 identical bytes; llvmpipe parity within tolerance.
- [ ] GX13 IBL release-capture close. Tier A. needs: -
  Accept: window presents N frames with 0 validation errors; reference-GPU chart beside the RL6 chart within tol; per-pass ms posted; Q1 V1-V3 consume the same set.
- [ ] Q1 V1–V3 reference captures and a perf report on the reference GPU. needs: RL21 RL26 RA14 WIN6
  Accept: HDRI vendored (`content/incoming/studio_small_09_1k.hdr` on disk or RL2 fallback landed) AND release pilot `--scene arena --fighters 8 --camera-path arena600 --frames 120 --exit` posts V1 hero plus V2 arena-day plus V3 fight action (3 distinct PNGs, RA8 manifests, PF2 per-pass ms quiet-window, VRAM/draw counts); placeholder refusal (no 64x48 noise, no single-cube sha); `node tools/proof.mjs` PASS golden `d57b5c8ae3096614` x2; `node tools/finish.mjs` P1/A4 moving.
- [ ] GM3 packaged Windows build runs clean (A8). needs: BLOCKED
  Accept: a scripted clean run plays 5 minutes and exits 0 with its log; a missing dependency fails it loudly.

Tier A live-frame chain (FOCUS Now Look order; board text wins, nothing duplicated here): EG8 (gate, needs EG2) then LF1 LF2 (needs EG9 each) then EG11 EG12 (needs EG9 each) then LF3 (needs EG2) LF4 (needs EG9) then EG13 EG14 (needs EG9 each). Each closes only with its board bar: masked on/off diff, per-pass slot above 0 ms live, 0 validation errors, render lib green, golden unchanged.

Row changes: none in this packet. EG/LF rows keep their board text; this plan only sets their order and the shared bar.

Row changes 2026-10-15 (proposals only, no board edits here): add RL63
post split slice 1 (bloom out of `post.rs`, under 1500 lines, LF4 needs
it); add RL64 presenter passthrough removal (2->0 or baseline, unblocks
hot lanes); merge RL27 into EG11 (cascades live in EG11, kill RL27);
merge WIN6b into WIN6 (one staged-fight row); kill RL31/RL22/RL24/RL25
(LATER with no consumer this round) once the lead confirms.

Steals head (FOCUS Next order; board text wins, nothing duplicated here): RL60 then RL34 then RL46a into RL46. Each closes only with its board Accept: render lib green, 0 validation errors, golden unchanged.

Other open rows of this plan (table from the board; `needs` is what each
waits on):

| ID | Title | Tier | Needs |
|---|---|---|---|
| VS5a | fighter pose to joint matrices | A | VS4 |
| VS5b | skinned, animated fighters | A | VS5a |
| VS7 | torches are lights | B | VS1 |
| VS9 | daily arena shot | A | VS1 |
| WIN10 | arena CLI surface | B | LATER |
| WIN9 | arena --exit watchdog | A | - |
| WIN11 | minimize/restore resilience | B | LATER |
| GX14 | GTAO boosted-release proof | A | - |
| RL5 | baked lightmaps with validity mask + SH probes for fighters reusing the ... | B | - |
| RL14 | fixed-step capture pacing + release receipt for RL5 | B | RL5 |
| RL11 | look-gate capture bundle | A | RL5 |
| RL13 | glTF-anchored material/exposure chart for GX3 IBL parity (READER-LOOK53 card 3). | A | Q1 |
| RL16 | twin material + IBL bind | A | LATER |
| RL18 | percentile-trimmed histogram auto-exposure with temporal adaptation vs fixed exposure (LADDER P4 CANDIDATE ... | B | - |
| RP8b | variance-clip re-spike, ONLY if the lead overturns the RP8a REJECT | B | - |
| RL22 | editor viewport day-rig bind | A | LATER |
| RL23 | twin grade + exposure bind | A | LATER |
| RL24 | viewport starburst source removal | A | LATER |
| RL25 | twin dome facet removal | A | LATER |
| RP11 | twin silhouette fringe removal | A | LATER |
| RP10 | twin frame-advance proof | A | LATER |
| RL27 | cascaded shadow maps for the arena-day sun | B | - |
| RL29 | RL5 boosted-release capture pilot | B | RL5 |
| RL30 | RL27 day-sweep parity pilot | B | RL27 |
| RL31 | Bevy-atmosphere Bruneton LUT bake beside the analytic sky - port the transmittance ... | A | LATER |
| RL60 | DrawItem transform + per-bin live draw | A | - |
| RL34 | one hue-keeping tonemap + dither | A | - |
| RL46a | material interface first | A | - |
| RL46 | forward uses the compiled material | A | RL46a |
| RA16 | belt + cache pressure drill | - | - |
| RA17 | pipeline-cache variant storm | - | - |
| RA18 | heal chaos at load | - | - |
| RA19 | tier gate at 6 GB | - | - |
| WIN6b | staged fight to the presenter | A | LATER |
| WIN7 | pad edge hardening | A | LATER |
| WIN8 | mode-switch last-good drill at lobby load | A | - |
| WIN12 | crash-plus-settings repro bundle | B | UIX6 |

Notes kept from the two deep-dives (verdicts only; the dives are in the archive):

- Q1 (deep-dive 2026-10-08): stop sending lane writers at Q1. Land the binds
  first, in this order: RL21, RL26, WIN6, RA14, RL11, GX13. Then one release
  pilot posts V1 hero, V2 arena-day and V3 fight PNGs with RA8 manifests. One
  pilot, one release window build, three PNGs. Proof: `node tools/board-slice.mjs
  RL21,RL26,WIN6,RA14,RL11,GX13`; `node tools/proof.mjs`; `node tools/finish.mjs P1,A4`.
- RL5 (deep-dive 2026-10-07): frozen. The CPU bake legs are green
  (`cargo test --locked -p engine-render --lib baked_gi::tests::bake_identical_bytes_twice`).
  Close only with bake x2 plus the 60 fps arena + 8 fighters reference-GPU
  receipt plus the pacing log (RL14, RL29 pilot); golden `d57b5c8ae3096614` unchanged.

## 8. Quality gates


- `cargo test -p engine-render --all-targets` green; every row adds or
  reuses CPU-reference golden bytes x2 before its GPU claim; llvmpipe
  parity within tolerance; 0 wgpu validation errors on every capture.
- `cargo clippy -p engine-render --all-targets -- -D warnings` clean;
  `cargo fmt --check` clean; `cargo deny check` green (allow-list §5).
- `node tools/proof.mjs` PASS, golden `d57b5c8ae3096614` x2 (any golden
  change needs a HASH-LAW bump receipt); GX6 truth gate green (no ticked
  row submits `Unsupported` on a device).
- Budgets from section 3 enforced in PF2 boosted captures; exactness
  asserts default, wall-clock asserts quarantined; a row that blows its
  bar fails the wave even with parity.
- Reference-GPU captures per look gate on the RTX 3050; Tier A merges
  on llvmpipe parity, the look gate needs the reference capture (RL11).
- No mirrored `LookFrame`/uniform/SH/post/AO types; every pattern port
  carries `PORT()` + `EVIDENCE.md`; UNVERIFIED stays idea-only.
- Material chart, shimmer A/B, day ramp + tunnel seal, fog-ramp match
  stay green forever; Q1 stays the roll-up gate.

### arena wiring


- `cargo test -p engine-game --lib` and `cargo test -p engine-anim --lib`
  green; adapter tests skip on `NoAdapter`, never fail.
- Golden `d57b5c8ae3096614` unchanged by every VS row (`node tools/proof.mjs`).
- `node tools/arena-shot.mjs` (VS9) green every day from VS1 on; the judge
  opens the three PNGs before PASS.
- Headless default: `cargo tree -p engine-game -e normal` shows no `gltf` or
  `image`.
- Frame p95 under 16.6 ms at 1080p on the RTX 3050 with kit and 2 fighters
  (PF2 numbers in the VS7 receipt).

### window and packaged run

- GM3 clean-machine run stays the roll-up gate; no silent failures; CUT grep gate (no console, mobile, fleet, liveops, analytics strings).

## 9. Session estimate

Moved to git history: `git show d96091cd:docs/plan/render-look.md` (section 9); each row carries its own estimate on the board.

