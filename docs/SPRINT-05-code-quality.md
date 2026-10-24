### Code quality (owner 2026-09-26, `docs/RUST-QUALITY.md`; one writer every wave)

Each row lowers a scorecard metric; `node tools/rust-quality.mjs --list
<metric> --crate <crate>` is its work list. When a metric reaches zero in
every crate, its clippy lint joins `[workspace.lints]` (the lint ladder).

- [x] CQ5 spatial: tick time — `staged.rs` reads `Instant::now` in Landed fb7466b1. staged.rs has no Instant; spatial 301 tests, clippy -D warnings, run 39052576461.
  production: elapsed time comes from the caller or `engine_core::Tick`;
  spatial adopts `tools/sim-clippy.toml`. Tier A. Size S. Owns
  `crates/spatial/src/staged.rs`. Accept: spatial nondet 0; config adopted;
  `cargo test -p engine-playtest spatial_eval` unchanged. needs: none.
- [ ] CQ6 ops: history out of comments — 3,068 comment lines name board Progress 2026-10-17: The `board_ids` metric reads 0 but is blind: `boardIds()` in tools/rust-quality.mjs reads only docs/SPRINT.md and the rows now live in shards, so its id set is empty; about 1,049 comment lines in crates/**/*.rs (tests/ dirs excluded, by an id-prefix regex) still name board ids, e.g. `(OPT14)` in crates/perf/src/workloads.rs. Burn-down merges 15440d59, 83bf78cb, d3ec2c25 did clear the old 3,068. Left: point boardIds() at the shards, then finish the remaining crates.
  IDs (`--list board_ids`); each becomes the invariant it protects or goes.
  One crate per packet, render, endpoint, game and editor first (768, 383,
  365, 265). Comment-only diff. Tier A. Size S per crate. Owns one crate's
  `src/`. Accept: the crate's board_ids 0; build and fmt clean. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] CQ7 render: allocation-free frame — 147 allocations inside render Progress 2026-10-17: Render hot_alloc 147 -> 121 (`node tools/rust-quality.mjs --no-cache`). Left: the other 121 and the 120-frame zero-call probe.
  step/submit/draw functions (`--list hot_alloc --crate render`) move to
  frame arenas and reused staging, uniform and draw-list buffers; a probe
  test over 120 steady frames of the window loop reads 0 calls. Tier A.
  Size L (planner splits it per pass). Owns `crates/render/src/`. Accept:
  render hot_alloc 0 outside reasoned `alloc-ok` resize and device-loss
  paths; frame p95 not worse. needs: none.
  Progress 2026-10-19: operator: hot_alloc 120 -> 0 landed cd85016d (4 real fixes: RP1 uniform blocks pack into stack arrays, `DecalPool::tick` partitions survivors in place, the transient-span refusal sits behind the cold guard `check_transient_span`, the headless graph stub writes its refusal into a sized `String`; plus 114 recorded `alloc-ok:` waivers on cold refusal/device-loss/readback/capture/probe lines, every message byte-identical; baseline entry accepted in the same commit). The 120-frame window-loop probe cannot read 0: on this PC 120 live frames read 23318 calls with all look passes on and 8 draws, 14762 with every look pass off and no draws, against a wgpu floor of 2161 (error scope+encoder+submit+poll) and 5400 (4 empty passes); those calls are wgpu's own command recording, not crate buffers. The crate's own CPU slices do read 0: `present::gpu_present::submit::cq7_present_alloc_tests::steady_120_frames_read_zero_allocs` and `live_stage_alloc_tests::live_stage_allocs_do_not_grow_with_the_draw_count` 2 passed / 0 failed. Owner call needed on the Accept: rate the crate's slices (met today) or accept a documented device floor in the probe; frame p95 was not re-measured here (no reference GPU). Left: the per-pass split of the live window frame's own allocations.
  BLOCKED: the crate's CPU slices read 0 allocations (`sq7_present_alloc_tests::steady_120_frames_read_zero_allocs` and `live_stage_alloc_tests::live_stage_allocs_do_not_grow_with_the_draw_count`, 2 passed) while the 120-frame window-loop probe reads wgpu's own command-recording floor (23318 calls against a 2161 wgpu floor on this PC), so the Accept needs the owner's reading of which number is the bar. Unblocks per docs/plan/2026-10-19-owner-queue.md.
- [x] CQ8 ops: one owner per behavior — 126 duplicated function groups Landed 15440d59. `node tools/rust-quality.mjs --no-cache` 2026-10-17: dup_fns 38 workspace-wide (was 214 bodies; bar 107), clone groups 34; golden d57b5c8ae3096614 unchanged on all five machines in golden-matrix run 39054403838.
  (214 bodies, `--list dup_fns`): byte readers (`take_u32`, `u16`...), sRGB
  conversion, digests and checkers move to one owner (engine-core for
  primitives, the owning crate otherwise). SL5 covers the FNV copies. Tier
  A. Size S per family. Owns the copies' files. Accept: dup_fns at most
  107; tests green; golden x2. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [x] CQ9 ops: split the monoliths — 12 files over 1,500 production lines
  (`render/src/present.rs` 5,704, `endpoint/src/assistant.rs` 4,129,
  `render/src/post.rs` 4,012, `game/src/camera.rs` 3,864) and 111 functions
  over 100 lines (largest `editor/src/a2_session.rs` `run_session`, 375)
  split by concern into modules and named steps, behavior unchanged. Tier B.
  Size M per file. Owns one file per packet. Accept: big_files and long_fns
  fall by the split count; public API unchanged; tests green. needs: none. Progress 2026-10-17: 9 files split (post, aa_scale, assistant, build_game, build_level, camera, keypoint, spatial_eval, index3), big_files 19 -> 10, golden unchanged (run 39063249080); commits 781ecd67 6f0229da 67a1c31f 40578154; left: render fog/gtao/ibl/presenter/live/submit and game fighter_look/lib/windowed/arena (held by other agents). Progress 2026-10-19: present.rs `decode_depth_feed` split into `check_depth_feed_inputs` + `unproject_texels` + `facing_normals` (558 -> 590 lines, long_fns 1 -> 0); assistant.rs `check`/`apply` split into `conflict_refusal`, `stage_root`, `parse_proposal`, `verify_proof`, `proof_verifiers` plus free helpers `observed_state`, `want_index`, `solve_placement`, `staged_gate`, `preview_batch` (long_fns 2 -> 0), public API unchanged both; endpoint assistant tests 39 passed (run 39229642335), render depth_decode 3 passed (run 39232734605) and render `--lib present` 61 passed on the PC; commit c792dfe1; remaining big files still held by other agents. Landed 188d2fe2f: ibl.rs 4,417 -> 1,380 lines, split into ibl/{brdf,chart,cube,error,forward,light,tests_core,tests_look}.rs + present/ibl_tests_gpu.rs; ibl 72 passed (v1, run 40118404322) and 90 passed (window, run 40118919531); WGSL chunk hash unchanged; the ibl clippy rotate fixed.
- [x] CQ24 quality: split post jitter plus motion leaf out — `halton`, `jitter_*`, Landed f737386e. The leaf is `crates/render/src/post_jitter_motion.rs` (flat file, not post/jitter_motion.rs): `pub fn halton` is there, `post.rs` re-exports it; render lib post tests green (`cargo test -p engine-render --lib -- shadow_cascade post ibl` 91 passed, PC run); golden unchanged.
  `MotionField`, `check_dims`/`check_image` move verbatim to new
  `crates/render/src/post_jitter_motion.rs`; facade re-exports. Accept: grep for
  `pub fn halton` in the new file fails before, passes after; render lib post
  tests green; golden unchanged. needs: none. Tick audit 2026-10-19: the leaf's real path is `crates/render/src/post_jitter_motion.rs` (flat file).
- [x] CQ25 quality: split post TAA resolve plus ghost scene out — `TaaParams`,
  resolve helpers, `taa_resolve`, `taa_resolve_jittered`, `GhostingScene` move
  verbatim to new `post/taa.rs`; trail under `GHOST_MAX_TRAIL`. Accept: render
  lib post tests green; golden unchanged. needs: CQ24. Landed 781ecd67 (the CQ9 post split). post/taa.rs holds TaaParams, taa_resolve, taa_resolve_jittered and GhostingScene (grep -c 'pub fn taa_resolve' = 2); cargo test -p engine-render --lib post: 49 passed incl. neighborhood_clamp_kills_ghosting and taa_static_scene_resolves_exactly (PC run, 2026-10-18); golden unchanged per run 39063249080.
- [x] CQ26 quality: split post upscale plus FSR plus shimmer scene out —
  `taa_upsample`, `fsr_*`, `ShimmerScene` move verbatim to new `post/upscale.rs`;
  MTF50/shimmer gates still post numbers. Accept: render lib post tests green;
  golden unchanged. needs: CQ24. Landed 781ecd67 (the CQ9 post split). post/upscale.rs holds taa_upsample, fsr_* and ShimmerScene (grep -c 'pub fn fsr_upscale' = 1); cargo test -p engine-render --lib post: 49 passed incl. fsr_540p_to_1080p_posts_mtf50_shimmer_and_ms and shimmer_converges_under_the_posted_gate (PC run, 2026-10-18); golden unchanged per run 39063249080.
- [x] CQ27 quality: split post bloom chain (LF4 dependency) out — bloom/exposure/
  ACES/grade configs, `apply_bloom`, `run_chain`, `run_frame` move verbatim to
  new `post/chain.rs`; torch halo lifts neighbour, sub-knee field identical.
  Accept: render lib post tests green; golden unchanged. needs: CQ25, CQ26. Landed 781ecd67 (the CQ9 post split, same commit as CQ25/CQ26). post/chain.rs holds BloomConfig, ExposureConfig, GradeConfig, PostChain, PostLights, PostOutput, apply_bloom, run_chain, run_frame and bloom_on_off_lift; each block byte-identical to post.rs at 12925e3b (block compare: 10/10 VERBATIM); grep -c 'pub fn apply_bloom' = 1. cargo test -p engine-render --lib post: 49 passed / 0 failed, 2 ignored (PC run, GPU/window crate, 2026-10-19), incl. bloom_on_off_lift_torch_halo_lifts_neighbor_but_not_dark, bloom_on_off_lift_dark_field_is_identity_and_deterministic, bloom_on_off_lift_kino_curve_torch_halo_and_subknee_identity, chain_reads_rl1_rl2_and_is_deterministic, full_frame_names_every_pass_in_ms. Golden d57b5c8ae3096614 unchanged (target/release/engine2040.exe: ticks=600 final_hash=d57b5c8ae3096614 state_hash=2aca9338670aa0cb).
- [x] CQ28 quality: split post auto-exposure plus exposure scene out — `AUTO_*`,
  `AutoExposure`, `run_chain_auto`, `run_frame_auto`, `ExposureScene`,
  `clipped_share` move verbatim to new `post/auto_exposure.rs`. Accept: render
  lib post tests green; golden unchanged. needs: CQ27. Landed 781ecd67 (the CQ9 post split, same commit as CQ25/CQ26/CQ27). post/auto_exposure.rs holds all 10 `AUTO_*` consts, the 3 private `AUTO_*` helpers, AutoExposureConfig, ExposureHistogram, hist_stride, bin_for_scaled, histogram_log_luma, trimmed_log_mean, target_ev_for_mean, AutoExposure, run_chain_auto, run_frame_auto, clipped_share and ExposureScene; 30/30 blocks byte-identical to post.rs at 12925e3b (only `pub(super)` visibility widened), and each of the row's six named items appears in exactly one file (post/auto_exposure.rs). F2P: `grep -c "pub struct AutoExposure" crates/render/src/post/auto_exposure.rs` = 2, `post.rs` = 0. P2P: `cargo test -p engine-render --lib post` 49 passed / 0 failed / 2 ignored (PC run 2026-10-19, GPU/window crate), incl. auto_exposure_beats_fixed_on_day_sweep, auto_exposure_ignores_torch_grid_and_limits_flicker, auto_frame_serves_match_clip, auto_exposure_costs_no_heap. Golden d57b5c8ae3096614 unchanged (gh run 39234721309, golden_hash_untouched).
- [x] CQ29 quality: split post RP1 graph plus scale seam out — RP1 names,
  transient bytes, `build_rp1_graph`, `compile_rp1_for_present`, `aa_scale` seam
  move verbatim to new `post/graph.rs`; compiled order stays six RP1 names.
  Accept: render lib post tests green; golden unchanged. needs: CQ27. Landed 34fae3ba. `crates/render/src/post/graph.rs` (274 lines, byte-identical to the old `post/rp1.rs`: `git show --stat --find-renames 061667e3` reports `post/{rp1.rs => graph.rs} | 0`) holds all six `RP1_PASS_*` names, `rp1_pass_names`, `RP1_HISTORY_BYTES_PER_TEXEL`/`RP1_POST_BYTES_PER_TEXEL`, `rp1_transient_bytes`, `build_rp1_graph`, `compile_rp1_for_present` and the RP5 `aa_scale` seam (`From<AaScaleError>`, `taa_off_render_size_for_scale`, `compile_taa_off_for_scale`, `tick_scale_for_present`, `run_taa_off_frame_for_scale`). Note: the rename was swept into another session's commit 061667e3 while this row was in flight; 34fae3ba adds the `mod graph; pub use graph::*;` facade that un-reds the tree (`mod rp1;` with no `rp1.rs` did not compile) and every `post::` caller path is unchanged. F2P: `grep -c "pub fn build_rp1_graph" crates/render/src/post/graph.rs` = 1 (0 before; the old name was in `post/rp1.rs`). P2P: `cargo test -p engine-render --lib post` = 49 passed / 0 failed / 2 ignored (PC run, GPU/window crate, 2026-10-19), incl. `rp1_graph_declares_six_passes_in_order`, `rp1_graph_rejects_bad_sizes_and_counts_bytes`, `rp5_scale_seam_drives_presented_size`, `rp5_scale_seam_fails_loudly`. Golden d57b5c8ae3096614 unchanged (gh run 39236977636, `cargo test -p engine-proof --lib golden_hash_untouched` PASS).
- [x] CQ30 quality: split post GPU submitter out, facade under the bar —
  window-only `mod gpu` (`Rp1Submitter`, resolve, frame) moves verbatim to new
  `post/gpu.rs`; `post.rs` under 1500 prod lines, `big_files` clears it; LF4
  unblocked. Accept: render lib post tests green; golden unchanged. needs:
  CQ27, CQ28, CQ29. Landed 781ecd67 (the CQ9 post split; this row proves the gpu hunk, same commit as CQ25-CQ29). `crates/render/src/post/gpu.rs` holds `Rp1Submitter`, `GpuResolve`, `GpuFrame`, `resolve` and `run_frame`; `crates/render/src/post.rs:202` is the only facade (`#[cfg(feature = "window")] pub mod gpu;`). The body is the old `mod gpu` dedented: compare `git show 781ecd67^:crates/render/src/post.rs` (module at lines 2780-3660) with `git show 781ecd67:crates/render/src/post/gpu.rs`, the only textual deltas are the `shaders/` -> `../shaders/` include paths plus rustfmt reflow; later behavior commits (CQ7, 83385c43) changed lines on purpose. F2P: `grep -c "pub struct Rp1Submitter" crates/render/src/post/gpu.rs` = 1, same grep on `post.rs` = 0. Prod lines: `post.rs` = 189 (rust-quality `lines` metric; 2,277 raw with tests), and `node tools/rust-quality.mjs --list big_files --crate render` no longer lists it (6 hits: fog, gtao, ibl, live, presenter, submit). P2P: `cargo test -p engine-render --lib post` = 49 passed / 0 failed / 2 ignored (PC run 2026-10-19, GPU/window crate), incl. `single_uniform_owner`, `rp1_graph_declares_six_passes_in_order`, `full_frame_names_every_pass_in_ms`; `rustfmt --edition 2021 --check` clean on both owned files. Golden d57b5c8ae3096614 unchanged (gh run 39238279514, `cargo test -p engine-proof --lib golden_hash_untouched` PASS). LF4 is unblocked (ticked 99e624b).
- [ ] CQ10 core: checked numerics - 4,236 numeric `as` casts; simulation Progress 2026-10-24: destruction `bake.rs` and `bake3d.rs` at 0 counted casts (`usize::from`, `Mode::tag`, four reasoned `#[expect]` on exact grid math), crate casts 109 -> 70, the cook digests pinned off HEAD and unchanged, golden x2 (4e5c98a, runs 40115485832, 40116041178, 40116224632). Left in destruction: budget, csg, spread, bonded, bond_grade. Progress 2026-10-17: Numeric casts 4,236 -> 2,965 workspace-wide; the named crates now nav 168, combat 65, anim 118, destruction 116. Left: at most 5 per 1,000 lines per crate and the clippy cast lints at zero.
  crates first (nav 192, combat 190, anim 176, destruction 174): `From`,
  `TryFrom` or named, tested saturating conversions in `engine_core`, then
  `clippy::cast_possible_truncation`, `cast_sign_loss` and
  `cast_possible_wrap` join the crate at zero. Tier A. Size M per crate.
  Owns one crate per packet. Accept: crate casts at most 5 per 1,000 lines;
  golden x2 unchanged. needs: none. Progress 2026-10-17: f6c055fb numeric casts engine-core 19 -> 1 and engine-geo 2 -> 0 (the one left is the audited f64->f32 narrowing in checked.rs); arena lib run 39065884031, geo run 39065811433. Progress 2026-10-19: 013124a combat 64 -> 22 casts (1.8 per 1,000 production lines); the crate carries `#![warn(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_possible_wrap)]` and is clean under `-D warnings`; new const owners `saturating_u32_to_u16` and `saturating_u32_to_i32` in `engine_core::checked` with `saturating_u32_narrowing_holds_the_bounds`. `cargo test -p engine-combat --no-fail-fast` 306 lib + 19 it + 1 doc green (gh 39240553933), `cargo test -p engine-core --lib checked` 11 green (gh 39240741291), golden d57b5c8ae3096614 x2. Left: nav 226, anim 109, destruction 109 and the remaining named crates, one packet each. Progress 2026-10-24: destruction budget.rs 11 -> 0 casts, soak hash and Q16 ratio pinned (b7f83df91; tests 40117110945, clippy 40117342149, golden x2); crate at 59 casts. Next: csg, spread, bonded, bond_grade.
- [x] CQ14 ops: typed errors — 93 `Result<_, String>` in library code Progress 2026-10-17: string_errors 93 -> 19 (render 13, endpoint 3, game 2, scene 1; playtest and assets at 0). Left: those 19.
  (playtest 50, render 18, assets 17) become typed enums with `Display` and
  a source. Tier B. Size S per crate. Owns one crate's error module.
  Accept: the crate's string_errors 0; tests green. needs: none. Progress 2026-10-18: render string_errors 13 -> 0 and secret_launch false hits gone, landed 3f1661f1 (window check clean, window lib tests 125 + 37 green). Left: endpoint mcp.rs run_stdio (Result<String, String>) as a StartupError::transport helper, uncommitted in mcp.rs, startup.rs and bin/e2040-mcp.rs because the endpoint big_files ratchet (mcp.rs over 1500 lines) refuses the commit. Landed a50ee6e6 (mcp.rs split: the rmcp tools moved to mcp_tools.rs, endpoint bigF 0) + fce5e6f2 baseline; workspace string_errors now 0; cargo test -p engine-endpoint --features mcp --no-fail-fast green (gh run 39119478907).
- [x] CQ15 ops: Miri UB gate on non-forbid unsafe sites (sim crates
  already forbid; live sites editor/app.rs:244 + endpoint/
  secret_launch.rs:164) (READER-WILD wave 51, Miri MIT). Tier B.
  Size S. Accept: Miri green scoped (core alloc_probe + site audit),
  forbid-rollout list, 0 suppressions. needs: none. (WILD.) Landed 3c5d3641. tools/miri-gate.mjs (12 node tests), tools/miri-sites.json, .github/workflows/miri.yml; mirror run 39066930973 all four jobs green; Miri covers alloc_probe, ecs Scope::spawn (2 tests) and provider test_env; the xatlas C++ and DXGI sites are mapped as skipped.
- [x] CQ16 ops: std-only file-snapshot + redaction helper for goldens
  with a pending-gate (insta snapshot/redaction discipline
  reimplemented, Apache-2.0; no insta dependency, no vendored bytes).
  Tier A. Size S. Owns snapshot helper hunks in `tests/` + the proof
  check-mode hunk beside `tools/proof.mjs` (golden suite frozen).
  Accept: unreviewed/pending snapshots fail CI with the pending path
  named, reviewed accept passes; golden suite green x2 identical;
  suite slows at most 5% vs baseline with the delta printed; 0 pending
  snapshots committable. Kill: dev-dep-free std-only helper
  impossible, or proof suite slows >5%, or >0 pending snapshots
  committable. needs: none. (WILD corner-23 Card 1.) Landed f740c1aa. tests/common/mod.rs + proof_cli snapshot `proof-cli/default-text-receipt` (default_and_max_seed_have_real_machine_readable_receipts green x2 identical local, whole binary 13 passed on the runner gh 39136969584); pending case red names `.snap.new` (common::tests::mismatch_writes_the_pending_path_and_names_it, accept_rewrites_the_snapshot_and_clears_the_pending_file); tools/proof.mjs fails on any pending `.snap.new` with its path (node tests 34 pass, real file demo named tests/snapshots/proof-cli/pending-demo.snap.new); clippy -D warnings gh 39137405142; golden bytes untouched.
- [x] CQ-NET-API3 net: single tick-rate and codec-length owner — Landed 61dc1642. scale.rs takes `TICK_HZ` from `engine_core` and the codec lengths (`SNAPSHOT_HEADER_LEN`, `SNAPSHOT_SLOT_LEN`, `STATE_DELTA_HEADER_LEN`) from `session`; no `owned copy, no import` text left in crates/net/src; `server::TICK_HZ` went with server.rs in the net slim 3151bd46. `cargo test -p engine-net --lib` 258 passed, 2 ignored, gh run 39061111700 (scale and session filter: 39 passed, gh run 39057623054).
  `scale.rs` imports the arena tick rate and the session codec lengths
  instead of parity copies; `server::TICK_HZ` re-exports the same owner.
  Tier A. Size S. Owns `crates/net/src/scale.rs`,
  `crates/net/src/websocket/server.rs`, `crates/net/src/webtransport/server.rs`. Accept: no "owned copy, no import" in
  `crates/net/src`; scale + server tests green; golden x2. needs: none.
  (API-REVIEW wave 54.) Tick audit 2026-10-19: the old `net/src/server.rs` left in the net slim 3151bd46; its content is now `crates/net/src/websocket/server.rs` and `crates/net/src/webtransport/server.rs`.
- [x] CQ-NET-API4 net: name the three servers — rename `server::Server` Landed 3151bd46. Moot after the net slim: `server::Server` no longer exists; the one public `Server` is `session::Server`, the rest have their own names (`dedicated::HeadlessServer`, `websocket::WsServer`, `webtransport::WtServer`, `relay::RelayServer`, `relay::StunServer`). `cargo test -p engine-net --lib` 258 passed, gh run 39061111700.
  to the AOI/presence harness name with a deprecated alias for one wave;
  root docs name `session::Server` vs `dedicated::HeadlessServer` vs the
  harness. Tier B. Size S. Owns `crates/net/src/websocket/server.rs`,
  `crates/net/src/lib.rs`. Accept: no two public `Server` types under
  different modules without qualified-doc guidance; tests green; golden
  x2. needs: CQ-NET-API1. (API-REVIEW wave 54.) Tick audit 2026-10-19: the named `server.rs` left in the net slim 3151bd46; the public names are `session::Server`, `websocket::WsServer`, `webtransport::WtServer`.
- [x] CQ-NET-API5 net: collapse the rollback families — delete or
  delegate lib-root `Rollback` stub to `rollback::FrameLog`, document
  the canonical pick per use-case in `lib.rs` docs; unify
  `Sim`/`SimTick`. Tier A. Size M. Owns `crates/net/src/lib.rs`,
  `crates/net/src/netcode.rs`, `crates/net/src/rollback.rs`. Accept:
  one documented session choice per use-case in `lib.rs`; zero "stub"
  comments in public API; tests green; golden x2. needs: CQ-NET-API2.
  (API-REVIEW wave 54.) Landed a0d9e63a. `netcode::Sim` is now a re-export of `rollback::SimTick`; `lib.rs` names the session per use-case; the retired-ring gate (`one_rollback_path_no_source_names_the_retired_ring`) is green and no "stub" mention remains in `crates/net/src`. netcode 14 passed (gh run 39242923961), rollback 30 passed (39242923762), it binary 44 passed (39242499563); golden x2 d57b5c8ae3096614 / state 2aca9338670aa0cb.
- [x] CQ-NET-API6 net: units in names — `SnapEnt::{x,y}` gain cm units, Landed 3151bd46. Moot after the net slim: `SnapEnt`, `Config{budget,cell,view,world}`, `DelayBuffer` and anticheat.rs no longer exist in crates/net/src. `cargo test -p engine-net --lib` 258 passed, gh run 39061111700.
  `Config::{budget,cell,view,world}` gain byte/cm units,
  `DelayBuffer.delay` gains ticks, anticheat limits gain unit suffixes.
  Tier A. Size S. Owns `crates/net/src/websocket/server.rs`,
  `crates/net/src/lib.rs`, `crates/net/src/anticheat.rs`. Accept: bare
  unitless names gone; tests green; golden x2. needs: none.
  (API-REVIEW wave 54.) Tick audit 2026-10-19: the old `net/src/server.rs` left in the net slim 3151bd46; its content is now `crates/net/src/websocket/server.rs` and `crates/net/src/webtransport/server.rs`.
- [x] CQ-NET-API7 net: Debug plus docs pass — `#[derive(Debug)]` (plus Progress 2026-10-17: net string_errors 0, undoc_pub 50 -> 2 (interest.rs:535 `select`, tick_budget.rs:635 `capture`), score 98; the stale quic.rs wire-up comment is gone. Left: those 2 docs and the tick_budget Fnv64 fold (unchecked).
  `Clone` where cheap) on the session/server/client/log types;
  document the 50 `undoc` public items; delete the stale
  `quic.rs:10-11` wire-up comment; `tick_budget` folds go through
  `engine_core::Fnv64`. Tier A. Size S. Owns `crates/net/src/`. Accept:
  net `undoc` 0; net `str_err` 0; score improves from 75; tests green.
  needs: CQ-NET-API2. (API-REVIEW wave 54.) Landed b052e7e6. The two items the
  2026-10-17 note named as left were already documented in the tree
  (`interest.rs::select`, `tick_budget.rs::summary`); the last one, the
  `tick_budget` fold, now goes through `engine_core::Fnv64` —
  `TickBudget::fold_snapshot` spelled FNV-1a's offset and prime in decimal
  (`1_464_717_047_381_303_168`, `1_099_511_628_211`), which the hex-reading
  reimplementation gate never saw, and the value is fed to `black_box` and never
  compared, so only the work it forces is the point. Accept met:
  `rust-quality --crate engine-net --no-cache` undoc_pub 0, string_errors 0,
  hash_reimpl 0, score 99 (A); `cargo test -p engine-net --lib` 357 passed, 2
  ignored (run 39207576011); golden d57b5c8ae3096614 unchanged (run
  39208182933).
