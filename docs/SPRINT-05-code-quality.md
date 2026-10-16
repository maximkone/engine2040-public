### Code quality (owner 2026-09-26, `docs/RUST-QUALITY.md`; one writer every wave)

Each row lowers a scorecard metric; `node tools/rust-quality.mjs --list
<metric> --crate <crate>` is its work list. When a metric reaches zero in
every crate, its clippy lint joins `[workspace.lints]` (the lint ladder).

- [ ] CQ5 spatial: tick time — `staged.rs` reads `Instant::now` in
  production: elapsed time comes from the caller or `engine_core::Tick`;
  spatial adopts `tools/sim-clippy.toml`. Tier A. Size S. Owns
  `crates/spatial/src/staged.rs`. Accept: spatial nondet 0; config adopted;
  `cargo test -p engine-playtest spatial_eval` unchanged. needs: none.
- [ ] CQ6 ops: history out of comments — 3,068 comment lines name board
  IDs (`--list board_ids`); each becomes the invariant it protects or goes.
  One crate per packet, render, endpoint, game and editor first (768, 383,
  365, 265). Comment-only diff. Tier A. Size S per crate. Owns one crate's
  `src/`. Accept: the crate's board_ids 0; build and fmt clean. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] CQ7 render: allocation-free frame — 147 allocations inside render
  step/submit/draw functions (`--list hot_alloc --crate render`) move to
  frame arenas and reused staging, uniform and draw-list buffers; a probe
  test over 120 steady frames of the window loop reads 0 calls. Tier A.
  Size L (planner splits it per pass). Owns `crates/render/src/`. Accept:
  render hot_alloc 0 outside reasoned `alloc-ok` resize and device-loss
  paths; frame p95 not worse. needs: none.
- [ ] CQ8 ops: one owner per behavior — 126 duplicated function groups
  (214 bodies, `--list dup_fns`): byte readers (`take_u32`, `u16`...), sRGB
  conversion, digests and checkers move to one owner (engine-core for
  primitives, the owning crate otherwise). SL5 covers the FNV copies. Tier
  A. Size S per family. Owns the copies' files. Accept: dup_fns at most
  107; tests green; golden x2. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] CQ9 ops: split the monoliths — 12 files over 1,500 production lines
  (`render/src/present.rs` 5,704, `endpoint/src/assistant.rs` 4,129,
  `render/src/post.rs` 4,012, `game/src/camera.rs` 3,864) and 111 functions
  over 100 lines (largest `editor/src/a2_session.rs` `run_session`, 375)
  split by concern into modules and named steps, behavior unchanged. Tier B.
  Size M per file. Owns one file per packet. Accept: big_files and long_fns
  fall by the split count; public API unchanged; tests green. needs: none.
- [ ] CQ24 quality: split post jitter plus motion leaf out — `halton`, `jitter_*`,
  `MotionField`, `check_dims`/`check_image` move verbatim to new
  `crates/render/src/post/jitter_motion.rs`; facade re-exports. Accept: grep for
  `pub fn halton` in the new file fails before, passes after; render lib post
  tests green; golden unchanged. needs: none.
- [ ] CQ25 quality: split post TAA resolve plus ghost scene out — `TaaParams`,
  resolve helpers, `taa_resolve`, `taa_resolve_jittered`, `GhostingScene` move
  verbatim to new `post/taa.rs`; trail under `GHOST_MAX_TRAIL`. Accept: render
  lib post tests green; golden unchanged. needs: CQ24.
- [ ] CQ26 quality: split post upscale plus FSR plus shimmer scene out —
  `taa_upsample`, `fsr_*`, `ShimmerScene` move verbatim to new `post/upscale.rs`;
  MTF50/shimmer gates still post numbers. Accept: render lib post tests green;
  golden unchanged. needs: CQ24.
- [ ] CQ27 quality: split post bloom chain (LF4 dependency) out — bloom/exposure/
  ACES/grade configs, `apply_bloom`, `run_chain`, `run_frame` move verbatim to
  new `post/chain.rs`; torch halo lifts neighbour, sub-knee field identical.
  Accept: render lib post tests green; golden unchanged. needs: CQ25, CQ26.
- [ ] CQ28 quality: split post auto-exposure plus exposure scene out — `AUTO_*`,
  `AutoExposure`, `run_chain_auto`, `run_frame_auto`, `ExposureScene`,
  `clipped_share` move verbatim to new `post/auto_exposure.rs`. Accept: render
  lib post tests green; golden unchanged. needs: CQ27.
- [ ] CQ29 quality: split post RP1 graph plus scale seam out — RP1 names,
  transient bytes, `build_rp1_graph`, `compile_rp1_for_present`, `aa_scale` seam
  move verbatim to new `post/graph.rs`; compiled order stays six RP1 names.
  Accept: render lib post tests green; golden unchanged. needs: CQ27.
- [ ] CQ30 quality: split post GPU submitter out, facade under the bar —
  window-only `mod gpu` (`Rp1Submitter`, resolve, frame) moves verbatim to new
  `post/gpu.rs`; `post.rs` under 1500 prod lines, `big_files` clears it; LF4
  unblocked. Accept: render lib post tests green; golden unchanged. needs:
  CQ27, CQ28, CQ29.
- [ ] CQ10 core: checked numerics — 4,236 numeric `as` casts; simulation
  crates first (nav 192, combat 190, anim 176, destruction 174): `From`,
  `TryFrom` or named, tested saturating conversions in `engine_core`, then
  `clippy::cast_possible_truncation`, `cast_sign_loss` and
  `cast_possible_wrap` join the crate at zero. Tier A. Size M per crate.
  Owns one crate per packet. Accept: crate casts at most 5 per 1,000 lines;
  golden x2 unchanged. needs: none.
- [ ] CQ14 ops: typed errors — 93 `Result<_, String>` in library code
  (playtest 50, render 18, assets 17) become typed enums with `Display` and
  a source. Tier B. Size S per crate. Owns one crate's error module.
  Accept: the crate's string_errors 0; tests green. needs: none.
- [ ] CQ15 ops: Miri UB gate on non-forbid unsafe sites (sim crates
  already forbid; live sites editor/app.rs:244 + endpoint/
  secret_launch.rs:164) (READER-WILD wave 51, Miri MIT). Tier B.
  Size S. Accept: Miri green scoped (core alloc_probe + site audit),
  forbid-rollout list, 0 suppressions. needs: none. (WILD.)
- [ ] CQ16 ops: std-only file-snapshot + redaction helper for goldens
  with a pending-gate (insta snapshot/redaction discipline
  reimplemented, Apache-2.0; no insta dependency, no vendored bytes).
  Tier A. Size S. Owns snapshot helper hunks in `tests/` + the proof
  check-mode hunk beside `tools/proof.mjs` (golden suite frozen).
  Accept: unreviewed/pending snapshots fail CI with the pending path
  named, reviewed accept passes; golden suite green x2 identical;
  suite slows at most 5% vs baseline with the delta printed; 0 pending
  snapshots committable. Kill: dev-dep-free std-only helper
  impossible, or proof suite slows >5%, or >0 pending snapshots
  committable. needs: none. (WILD corner-23 Card 1.)
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] CQ-NET-API3 net: single tick-rate and codec-length owner —
  `scale.rs` imports the arena tick rate and the session codec lengths
  instead of parity copies; `server::TICK_HZ` re-exports the same owner.
  Tier A. Size S. Owns `crates/net/src/scale.rs`,
  `crates/net/src/server.rs`. Accept: no "owned copy, no import" in
  `crates/net/src`; scale + server tests green; golden x2. needs: none.
  (API-REVIEW wave 54.)
- [ ] CQ-NET-API4 net: name the three servers — rename `server::Server`
  to the AOI/presence harness name with a deprecated alias for one wave;
  root docs name `session::Server` vs `dedicated::HeadlessServer` vs the
  harness. Tier B. Size S. Owns `crates/net/src/server.rs`,
  `crates/net/src/lib.rs`. Accept: no two public `Server` types under
  different modules without qualified-doc guidance; tests green; golden
  x2. needs: CQ-NET-API1. (API-REVIEW wave 54.)
- [ ] CQ-NET-API5 net: collapse the rollback families — delete or
  delegate lib-root `Rollback` stub to `rollback::FrameLog`, document
  the canonical pick per use-case in `lib.rs` docs; unify
  `Sim`/`SimTick`. Tier A. Size M. Owns `crates/net/src/lib.rs`,
  `crates/net/src/netcode.rs`, `crates/net/src/rollback.rs`. Accept:
  one documented session choice per use-case in `lib.rs`; zero "stub"
  comments in public API; tests green; golden x2. needs: CQ-NET-API2.
  (API-REVIEW wave 54.)
- [ ] CQ-NET-API6 net: units in names — `SnapEnt::{x,y}` gain cm units,
  `Config::{budget,cell,view,world}` gain byte/cm units,
  `DelayBuffer.delay` gains ticks, anticheat limits gain unit suffixes.
  Tier A. Size S. Owns `crates/net/src/server.rs`,
  `crates/net/src/lib.rs`, `crates/net/src/anticheat.rs`. Accept: bare
  unitless names gone; tests green; golden x2. needs: none.
  (API-REVIEW wave 54.)
- [ ] CQ-NET-API7 net: Debug plus docs pass — `#[derive(Debug)]` (plus
  `Clone` where cheap) on the session/server/client/log types;
  document the 50 `undoc` public items; delete the stale
  `quic.rs:10-11` wire-up comment; `tick_budget` folds go through
  `engine_core::Fnv64`. Tier A. Size S. Owns `crates/net/src/`. Accept:
  net `undoc` 0; net `str_err` 0; score improves from 75; tests green.
  needs: CQ-NET-API2. (API-REVIEW wave 54.)
