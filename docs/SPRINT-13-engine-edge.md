### Engine edge (owner 2026-09-26, `docs/plan/quality.md`)

Speed, quality, AI-agnostic patterns and workspace. Every row measures
before and after and keeps its change only if the numbers move; misses
revert the same wave. OPT1–OPT13 serve the FOCUS "Now" speed pillar.
Rows marked (orchestrator) land root config or `tools/` edits.

- [x] OPT4 ops: linker A/B and PF6 re-verify — `rust-lld.exe` for Landed 631e7192.
  `x86_64-pc-windows-msvc`; record link time for `arena.exe` and test
  binaries; restate what PF6 actually delivered. (orchestrator) Tier A.
  Size S. Owns `.cargo/config.toml` hunks and a PF6 note. Accept: keep
  only if link ≥ 20% faster with every test green; the PF6 receipt names
  what exists. needs: OPT1.
- [x] OPT5 ops: profiles with one job each - `profiling` (release, line Landed 15ffd5d2.
  tables, thin LTO), `perf-test` (release, thin LTO, 16 codegen units)
  for proof `--release` legs if PF1 budgets hold, `ship` (fat LTO, one
  unit, `panic = "abort"` after an unwind audit, PGO from a recorded
  tape per PF3). (orchestrator) Tier A. Size M. Owns the profile edits.
  2026-10-21 (OPT45): `release` itself is thin LTO with 16 codegen units and
  the CI proof and release-proof legs build `ship`, the fat-LTO one-unit shape.
  2026-10-21 (OPT46): the `perf-test` profile is gone, it was the same shape as
  `release` under a second name.
  Accept: proof wall time and `arena.exe` frame time before/after; PF1
  baselines re-recorded append-only (PF11) when timings shift. needs:
  OPT1, PF3, PF11.
- [x] OPT7 ops: affected-package gate — `tools/affected.mjs` maps a diff Landed cd37e7c4.
  to changed packages plus reverse dependencies via `cargo metadata`;
  lane acceptance runs that closure; the full proof stays once per wave.
  (orchestrator) Tier A. Size M. Owns `tools/affected.mjs`. Accept: a
  seeded `combat` edit runs combat and dependents only; a `core` edit
  runs all; lane gate median wall time halved. needs: OPT1.
- [x] OPT14 ecs: spine performance bars — PF1 workloads on the ARC spine: Progress 2026-10-17: `SpineTransformBody` (10k entity-steps) and `SpineFullTick` (1,024 agent-ticks) exist in crates/perf/src/workloads.rs with recorded hash baselines, kill-bar rows 0.5 ms and 2.0 ms in `TIME_BUDGETS_MS` (baselines.rs), a zero-alloc `steady()` slice and the tests `spine_transform_body_matches_baseline` and `spine_full_tick_matches_baseline` (tests/it/canonical.rs). Left: measured ms x2 posted in the PF1 table.
  20k entities transform and body step ≤ 1.0 ms, 2k agents full tick
  ≤ 4.0 ms, 0 allocations; the ARC3 parity harness prints them. Tier B.
  Size S. Owns `crates/perf` spine workloads. Accept: numbers x2 in the
  PF1 table. needs: ARC2, PF1. Landed 8139e86d. SpineTransformBody::run_steps and SpineFullTick::run_steps time the passes alone (run_hash digests every pass and measured 1.10 ms p50 for the 0.5 ms bar, it is the equality proof); perf_time_budgets now times both spine rows through them. Measured x2, GitHub runner, release, p50: transform+body 10k entity-steps 0.037 ms (run 39103012592) and 0.022 ms (run 39104362102) against the 0.5 ms bar; full tick 1k agent-ticks 0.008 and 0.005 ms against 2.0 ms. Zero allocations: steady_windows_allocate_nothing_x2, engine-perf --no-fail-fast green run 39105354580. No PF1 table file exists; the numbers are posted here and in the commit.
  2026-10-21 (lead): the spine's own slot cap moved 16 -> 32 with the participant cap, so the canonical runs cover 20k entity-steps and 2k agent-ticks now and the two kill bars scale with the work (the same rate per unit of work): 0.5 -> 1.0 ms and 2.0 -> 4.0 ms. The p50 numbers above were measured at the 16-slot cap and are kept as they are; the release-mode re-measurement at 32 slots is now in: transform+body 20k entity-steps p50 0.044 ms, max 0.058 ms against the 1.0 ms bar, full tick 2k agent-ticks p50 0.012 ms, max 0.013 ms against the 4.0 ms bar (GitHub runner, release, one thread, run 39640250921). The two spine baseline rows were re-recorded (the fixed array size moved, which the table's own rule allows) and the 16-slot hashes live in the dated comment above them.
- [x] OPT19 render: zero first-use stutter — warm every pipeline variant
  a scene can use during load through RA5; a camera walk across every
  material shows no frame above 1.5× the median from pipeline creation.
  Tier A. Size S. Owns warm-up hunks in `crates/render/src/cache.rs`.
  Accept: the walk fixture passes x2. needs: RA5, RA7. Landed 6c5b930e. opt19_load_warms_then_walk_hits_under_bar (cold walk 2.0x median misses the 1.5x bar, warm walk 1.0x, second warm compiles 0) green x2: gh-check runs 39101438701 and 39101644878.
- [x] OPT20 render: dynamic resolution — measured GPU time drives the Progress 2026-10-17: The control law landed as `crates/render/src/dynres_gov.rs` (DynRes, GS12) wired into `aa_scale::ScaleController`; tests scale_dynres_* and scale_controller_spike_floors_through_the_governor (a 3x spike floors the scale at 50%). Left: a seeded GPU-load fixture proving frame time p95 at most 16.7 ms, the resolution-change log, and the file name (the row says dynres.rs).
  internal resolution between 50% and 100% through the TAAU upscaler to
  hold the frame budget. Tier B. Size M. Owns
  `crates/render/src/dynres_gov.rs`. Accept: under a seeded GPU load spike
  frame time p95 ≤ 16.7 ms; resolution changes logged; golden unchanged.
  needs: GX4, RA8. Landed f667e443. dynres_seeded_load_spike_holds_p95_at_the_frame_budget (seeded 600 frames: full resolution p95 over 16.7 ms, controller p95 at most 16.7 ms, back at 100 percent), dynres_resolution_changes_are_logged_inside_the_bounds (ScaleController::changes, first step down within 4 frames of the spike), dynres_seeded_run_is_identical_twice; cargo test -p engine-render --lib -- temporal dynres 19 passed run 39103216237, full cargo test -p engine-render --no-fail-fast run 39104351445. Tests live in dynres_gov.rs (the row names dynres.rs); golden unchanged (presentation only). Tick audit 2026-10-19: the row's `dynres.rs` name is `crates/render/src/dynres_gov.rs`.
- [x] OPT24 provider: response cassettes — a content-addressed store Landed e04071b6.
  keyed by (adapter, model, request digest, schema version); live runs
  record, evals and CI replay offline with no keys; a stale entry is
  refused with its reason, never reused silently. Tier B. Size M. Owns
  `crates/provider/src/cassette.rs`. Accept: the EV1 suite replays from
  cassettes byte-identical x2 with the network off; a changed prompt
  misses and says why. Landed so far (2026-10-12 check): 86d5164a (cassette store, single-call replay). Remaining: the EV1 suite recorded once and replayed twice from the cassette store gives byte-identical `to_text` with no provider calls; a changed prompt misses with its reason. needs: PV5.
- [x] OPT25 provider: structured-output ladder — one engine schema per
  action type maps to each vendor's native structured output, then JSON
  mode, then text with a bounded repair parse, then scripted; the PV5
  matrix records the rung each backend used. Tier B. Size M. Owns
  `crates/provider/src/ladder.rs`. Accept: every backend reports its rung;
  malformed output never applies; rung drops are logged events. needs:
  PV5, AX1.
- [x] OPT26 provider: scorecard router — per task class (bark, dialogue, Landed aa644693. Run 39052718180; router has no caller beyond Ask::for_request yet.
  level brief, encounter, review) pick a model from the eval scorecard
  (quality, p95 latency, cost) under budget caps, falling back local →
  remote → scripted; every decision recorded. Tier B. Size M. Owns
  `crates/provider/src/router.rs`. Accept: a seeded outage of the top
  model reroutes within the deadline; decisions replay from the record;
  a cost cap refuses loudly. needs: OPT24, EV1.
- [x] OPT27 provider: inference arbiter — local models and the game Progress 2026-10-17: provider side done (arbiter, serving adapter); left: feed live frame p95 and read_vram, window-build budget check on a GPU. Progress 2026-10-21: the provider half landed 50361c3d — `crates/provider/src/pressure.rs` (`PressureFeed`: frame-time p95 window and the VRAM reading reach the arbiter sample, typed notices, `route()` on the same reader) and `VramHeadroom::from_bytes` (the read_vram seam); 273 lib tests and 2 matrix tests green on the runner, clippy clean. Left: the window/game loop must own `PressureFeed` and the window build must hold the P6 frame budget with a local model loaded, which is the render and game lane's half — so the row is NOT ticked.
  share one 6 GB GPU: under frame pressure local inference moves to CPU,
  defers or routes remote; VRAM use reported; the tick never waits.
  Tier B. Size M. Owns `crates/provider/src/arbiter.rs`. Accept: with a
  local model loaded the window build holds the P6 frame budget; requests
  degrade with typed notices. needs: PV2, RA4.
  Correction 2026-10-21: duplicate - PW28 owns this work now; PW28's row says it closes OPT27 and LAI7. This row is not ticked and is not built on its own; retire it when PW28 lands. Unparked 2026-10-23 (owner: `unpark N GO`): the duplicate correction above read as "PW28 owns this work now", but PW28 sat in `docs/plan/parked-2026-10-22.md` with no authority, so focus group 4 had no row that could move. OPT27 owns the work again and PW28's parked text is its spec: the window pushes its frame time and its VRAM reading every frame, and the VRAM budget becomes a GPU layer count so the placement (GPU, CPU, defer, remote) needs no human. Progress 2026-10-23: both halves landed. Provider `a18f037b1` adds `LayerPlan`, `LayerNotice` (`Unknown`, `Partial{short_mb}`, `NoneFit`) and `Arbiter::plan_layers` behind `ArbiterConfig::layer_plan`, so a VRAM budget becomes a GPU layer count with a typed notice; 5 new tests, arbiter 20/20 green. Game `a0c88d000` adds `crates/game/src/pressure_watch.rs` (`PressureWatch`: the per-frame p95 window and `read_vram` into `Arbiter::observe`, placement and layers out) plus the arena wiring (`pressure`, `note_pressure`, `within_exit_deadline`); app bin check and `-p engine-game --features app` clippy green (run 39861858337). `9a843b671` skips the first 60 present frames: frame 0 compiles shaders (a real run measured one 658 ms frame) and would otherwise feed a fake p95. Real RTX 3050 window receipts (`target/logs/arena-20261022T222052Z.log`, 300 frames, 4 fighters, 5 PNGs, view ok): `arena pressure: p95_us=0 load=0 placement=cpu layers=32/32 vram_free_mb=5233 moved=none`, then `p95_us=180955 load=10857 placement=defer layers=32/32 vram_free_mb=4795 moved=cpu->defer`, so the feed, the layer plan and the typed move all fire in a live window. Frame split on that run: p50 4.6 ms, p99 252 ms, 84 slow frames of 300, with spikes at frames 0/180/240 rather than at the watch. Left, and why the row stays open: the accept needs a run with a local model loaded, this PC has no `llama-server.exe` and no `.gguf`, and that run's own split misses P6 without a model, so it cannot prove the accept. Paths: `crates/provider/src/arbiter.rs`, `crates/game/src/pressure_watch.rs`, `crates/game/src/bin/arena.rs`. Progress 2026-10-24: still blocked, the row stays open. Re-checked this PC: no `llama-server.exe` and no `.gguf` under C:\forge-media, C:\forge-data, the user cache, `~/.ollama`, Downloads or Documents; the only large weights there are non-LLM safetensors (kimodo, sf3d, dinov2). The provider half is green at HEAD a45ff8df1: `cargo test -p engine-provider --lib arbiter` 20 passed on the runner (run 40094320479). Left: the accept run needs `llama-server.exe` and one reference-size `.gguf` on this PC (the owner places them or approves the download), then one window run under the window lock. The 2026-10-22 run without a model already misses P6 (p99 252 ms, 84 of 300 frames slow), so that frame cost needs the render and game lane's fix before a model run can pass. Closed 2026-10-24, accept rewritten (owner: "my PC cant bare it ... skip it totaly or rewrite that idea"): no local language model runs beside the game. The potato target (iGPU, 2-4 GB VRAM) cannot hold one next to the fight, this PC has none, and a Colab or Kaggle notebook is no game backend (session limits, a browser tab, no serving), so in-game AI goes remote or scripted, which the arbiter already routes. The accept is now what the window proved: the frame p95 and free VRAM reach the arbiter every frame and it moves the work with a typed notice (`moved=cpu->defer` in `target/logs/arena-20261022T222052Z.log`, `a0c88d000`, `a18f037b1`), arbiter 20/20 on the runner (run 40094320479), and the game without a model holds the potato frame budget (HEAD `1a636e2cf` plus `dc2b93e56`: 1500 frames, view ok, p99 19.6 ms). A later feature that wants a local model opens a new row on top of `Arbiter::plan_layers`.
- [x] OPT28 provider: prompts as versioned assets — every prompt and Landed aa644693. Run 39052718180.
  brief template is a content-addressed asset with version, provenance
  and an eval set; an edit re-runs its evals and may not lower the score.
  Tier B. Size S. Owns `crates/provider/src/prompt.rs`. Accept: a seeded
  score-dropping edit fails the gate; the prompt digest lands in every
  provider record. needs: OPT24, EV1.
- [x] OPT31 pilot: dev window arena ≥ 30 fps on the reference GPU over
  the fixed arena camera path after OPT3, x2 within 5%, with the frame
  report under `target/evidence/build/`. Tier A. Size S. Owns the
  report. Accept: numbers x2 name adapter and driver; blocked until the
  BLOCKERS window-path entry closes. needs: OPT3, WIN1. (OPT3 VERDICT
  wave 84.) Progress 2026-10-19: operator: reference-GPU frame report x2;
  line in `docs/plan/2026-10-19-owner-queue.md`.
  Progress 2026-10-20T21:47Z: no receipt in four keeper runs (300, 300, 30, 30 frames) at 100 % CPU with 27-45 foreign `cargo` and 10-25 `rustc`: the window stops after `arena particles: per-tier ...`, never presents a frame (`Responding: False`, 3 % GPU, 1-2 s CPU per 240 s), so there is no fps, no spread and no tick. Evidence `target/evidence/build/opt31-attempt-20261020T2147Z.md`; one BLOCKERS row added.
  Progress 2026-10-20T22:02Z: the stall is the window/GPU path only — `arena.exe --view-line 2040 600 8` printed its 600-tick view line and exited on the same box, while `--backend vulkan --frames 30` stalled exactly like dx12 (no receipt in 120 s, 100 % CPU, 45 cargo / 20 rustc). So OPT33 has no leg either. Same evidence file, addendum.
  Progress 2026-10-21T23:50Z: two more keeper runs, 300 frames each, at CPU 100 / 100 / 99.9 % with 36 `cargo` and 25 `rustc`: both stopped after `arena: staged 6 dressing boxes` (before presenter creation), no `interval_ms p50=` line, `nvidia-smi` 8 % GPU. No fps, no spread — contended and unmeasured, not a floor pass, so no tick. Evidence `target/evidence/build/opt31-opt33-keeper-20261021T2350Z.md` (BLOCKERS fingerprint now x8).
  Landed 69ce744. Four interleaved 300-frame runs on `target/release/arena.exe --scene arena
  --fighters 8 --frames 300 --exit`, adapter NVIDIA GeForce RTX 3050 6GB Laptop GPU driver
  32.0.16.1062: interval_ms p50 16.874 / 17.421 / 16.829 / 16.863 ms = 57.4-59.4 fps against the
  30 fps floor, worst pair spread 3.19 % and all four within 3.52 %. Contended (CPU 61.8-66.3 %,
  0-4 transient cargo/rustc, no build storm), so a lower bound on both the floor and the spread.
  Every run printed the full receipt and exited alone, which closes the window-path BLOCKERS row.
  Evidence `docs/receipts/2026-10-22-opt31-frame-report.md`, raw stdout
  `target/evidence/build/opt31-frame-report-20261022T0202Z.md`.
