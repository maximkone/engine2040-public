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
  Accept: proof wall time and `arena.exe` frame time before/after; PF1
  baselines re-recorded append-only (PF11) when timings shift. needs:
  OPT1, PF3, PF11.
- [ ] OPT6 ops: nextest spike — proof legs under nextest with per-test
  processes; retries reported, never hidden; the `-j4` flake fingerprint
  re-checked. (orchestrator) Tier A. Size S. Owns the proof-leg switch.
  Accept: wall time and flake counts before/after; kept only if faster
   with the same failures visible. needs: OPT1.
   (DO NOT DISPATCH 2026-10-05: PARKED, cargo-nextest absent exit 101. Retry only after owner installs and posts version. Never re-probe whole, per 510 deep-dive.)
   Progress 2026-10-21: the proof-leg switch is landed and refuses cleanly instead of pretending: `node tools/nextest-spike.mjs --crate engine-core` -> "RESULT could not run: cargo nextest is not installed (error: no such command: 'nextest')", exit 2; `node --test tools/nextest-spike.test.mjs` -> 7/7. The bar (wall time and flake counts before and after, same failures visible) needs the owner's install, so the row stays parked exactly as the 2026-10-05 note says. CI already runs nextest.
- [x] OPT7 ops: affected-package gate — `tools/affected.mjs` maps a diff Landed cd37e7c4.
  to changed packages plus reverse dependencies via `cargo metadata`;
  lane acceptance runs that closure; the full proof stays once per wave.
  (orchestrator) Tier A. Size M. Owns `tools/affected.mjs`. Accept: a
  seeded `combat` edit runs combat and dependents only; a `core` edit
  runs all; lane gate median wall time halved. needs: OPT1.
- [ ] OPT8 ops: feature unification spike — a cargo-hakari workspace-hack Progress 2026-10-17: `crates/workspace-hack` (OPT36) and `tools/feature-unify.mjs` are on disk and `engine-workspace-hack` is a workspace dependency; spike evidence is in docs/SMOOTH-BUILD.md (16 syn, 12 serde_json, 14 num_traits copies in target/debug/deps before the hack). Left: the before/after dependency rebuild count across five lane builds (bar: drops by a third).
  Correction 2026-10-21: the "16 syn / 12 serde_json / 14 num_traits" figures quoted above were 2026-10-17 build-artifact counts in `target/debug/deps`, not `Cargo.lock` facts, and they drift with every build. The real numbers (parsed from `Cargo.lock` at HEAD, now recorded in `docs/SMOOTH-BUILD.md` "Build speed facts"): 1,032 packages, 80 names carry more than one version, 101 extra version copies; `serde_json` is one version (1.0.151), `num-traits` is one version (0.2.19) and is not a direct dependency, `syn` is three (1.0.109, 2.0.119, 3.0.6). The unification target is the 80 multi-version names, not serde_json/num-traits.
  crate (license read first). (orchestrator) Tier B. Size S. Owns the
  Progress 2026-10-21 (the spike ran, the bar is NOT met, so the row stays open and the union shape is in doubt): across five lanes (provider, scene, nav, spatial, observe) the dependency units went from 33 to 183 after `engine-workspace-hack = { workspace = true }` — 5.5x WORSE, not a third less. The union drags serde, serde_json, uuid and image into lanes that use none of them. One union is the wrong shape: the feature set has to be split per group, and the bar (a third less) still has to be met before this row can tick. Proof: `cargo tree --manifest-path <tree>/Cargo.toml -p <crate> -e normal,build --prefix none --format "{p}"` -> 33 before, 183 after; `node --test tools/dep-rebuild-count.test.mjs` -> 6/6. The crates' `Cargo.toml` one-liner is in `docs/research/powerups-2026-10-20/landing-speedup.md` and is NOT applied (a union that costs 5.5x must not be merged). The true rebuild count (the `Compiling` lines of five lane builds in one target dir) is still unmeasured: the cold build ran past 40 min on a loaded box and was cut.
  spike report. Accept: dependency rebuild count across five lane builds
  before/after; kept only if it drops by a third. needs: OPT1.
  Correction 2026-10-21: duplicate - PW45 owns this work now; PW45's row says it closes OPT8. This row is not ticked and is not built on its own; retire it when PW45 lands.
- [ ] OPT9 ops: compile cache spike — sccache across lane pools. Progress 2026-10-17: docs/SMOOTH-BUILD.md holds the OPT9 evidence: sccache 0.18.0 is opt-in per run (the pool wrapper was removed on purpose, .cargo/config.test.mjs guards it) and the GitHub check workflow runs with sccache. Left: a cold lane build before/after number (bar: at least 30% faster).
  Progress 2026-10-21 (the spike ran, the bar is NOT met by the cache, so the row stays open): cold 21.6 s vs warm 13.6 s on the clock — 37% faster — but 0 of 2 cacheable units hit, so the cache itself earned none of that. The bar (at least 30% faster) is not met by the cache. A `rebuild` cell in the CI shape (same target-dir path, artifacts removed, cache primed) is landed and pinned; its run did not finish on a loaded box (cold cell 17 min with 24 foreign cargo builds), so the number is open. Proof: `node --test tools/compile-cache-spike.test.mjs` -> 10/10. The rebuild-cell number on a quiet box and the Linux-runner hit rate are what is left.
  (orchestrator) Tier B. Size S. Owns the spike report. Accept: cold lane
  build time before/after; kept only if ≥ 30% faster. needs: OPT1.
  Correction 2026-10-21: duplicate - PW46 owns this work now; PW46's row says it closes OPT9. This row is not ticked and is not built on its own; retire it when PW46 lands.
- [ ] OPT11 ops: crate fan-out cut — reverse-dependency closure times
  compile seconds from OPT1 names the costliest edges; split or
  feature-gate the worst one or two. (orchestrator + owning lanes) Tier C.
  Size M. Owns the report plus the named split. Accept: touch-rebuild of
  the chosen crate down ≥ 30%, or the row closes with its numbers. needs:
  OPT1, OPT7.
  Correction 2026-10-21: duplicate - PW44 owns this work now; PW44's row says it closes OPT11. This row is not ticked and is not built on its own; retire it when PW44 lands.
- [ ] OPT13 ops: gameplay code hot-patch spike — subsecond or
  hot-lib-reloader (licenses read first) on one gameplay system in the
  window build. Tier C. Size M. Owns the spike report. Accept: an edit
  reaches the running game in ≤ 2 s with state kept, or the row closes
  with the failure. needs: OPT3.
- [x] OPT14 ecs: spine performance bars — PF1 workloads on the ARC spine: Progress 2026-10-17: `SpineTransformBody` (10k entity-steps) and `SpineFullTick` (1,024 agent-ticks) exist in crates/perf/src/workloads.rs with recorded hash baselines, kill-bar rows 0.5 ms and 2.0 ms in `TIME_BUDGETS_MS` (baselines.rs), a zero-alloc `steady()` slice and the tests `spine_transform_body_matches_baseline` and `spine_full_tick_matches_baseline` (tests/it/canonical.rs). Left: measured ms x2 posted in the PF1 table.
  10k entities transform and body step ≤ 0.5 ms, 1k agents full tick
  ≤ 2 ms, 0 allocations; the ARC3 parity harness prints them. Tier B.
  Size S. Owns `crates/perf` spine workloads. Accept: numbers x2 in the
  PF1 table. needs: ARC2, PF1. Landed 8139e86d. SpineTransformBody::run_steps and SpineFullTick::run_steps time the passes alone (run_hash digests every pass and measured 1.10 ms p50 for the 0.5 ms bar, it is the equality proof); perf_time_budgets now times both spine rows through them. Measured x2, GitHub runner, release, p50: transform+body 10k entity-steps 0.037 ms (run 39103012592) and 0.022 ms (run 39104362102) against the 0.5 ms bar; full tick 1k agent-ticks 0.008 and 0.005 ms against 2.0 ms. Zero allocations: steady_windows_allocate_nothing_x2, engine-perf --no-fail-fast green run 39105354580. No PF1 table file exists; the numbers are posted here and in the commit.
- [ ] OPT15 app: CPU frame pipelining — sim tick N+1 overlaps render
  extraction and submit of tick N; the extraction view is the only sync
  point. Tier C. Size M. Owns the frame loop in `crates/app`. Accept: CPU
  frame p95 −25% in the window build; replay hash unchanged x2. needs:
  ARC6.
- [ ] OPT16 render: GPU feature ladder — probe bindless texture arrays,
  multi-draw indirect count and ray query at startup beside RT0; draw
  through the fastest supported path with a tested fallback. Tier B.
  Size M. Owns `crates/render/src/ladder.rs`. Accept: 10k instances of 200
  materials in ≤ 50 draw calls on the fast path; fallback matches
  within tolerance on llvmpipe. needs: VG1, RA8.
- [ ] OPT17 render: texture streaming — mip residency from a GPU feedback
  buffer read back asynchronously, a VRAM budget for 6 GB, LRU eviction,
  uploads on the RA6 belts; never stalls. Tier B. Size L. Owns
  `crates/render/src/stream.rs`. Accept: a dense fixture stays under
  budget; no frame hitch > 2 ms from uploads; sharp within 1 s of a cut.
  needs: RA4, RA6.
  rot: crates/render/src/stream.rs was removed in the render wip sweep f737386e; the work now lives in crates/assets/src/stream.rs (StreamError) and crates/render/src/present/cache.rs (CpuCache), so re-home Owns to those two.
- [ ] OPT18 render: frame pacing and latency — present mode per setting,
  frame latency 1 in low-latency mode, input sampled late for the camera;
  measured by the PQ1 meter. Tier B. Size M. Owns
  `crates/render/src/pacing.rs`. Accept: input-to-photon p95 −20% at equal
  fps; frame-time variance reported. needs: WIN1, PF2.
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
- [ ] OPT21 ops: mutation testing ratchet — cargo-mutants on `core`, Progress 2026-10-17: `tools/mutants.mjs` (+ mutants.test.mjs) runs cargo-mutants on a patch and fails closed on surviving mutants in the changed lines. Left: the per-crate survival ratchet over core, combat, nav, spatial and net, and the deleted-assertion drill.
  `combat`, `nav`, `spatial` and `net`; surviving mutants listed with
  rows for the worst gaps; the survival rate per crate may never rise.
  Tier A. Size M. Owns the report and the ratchet leg. Accept: seeding a
  deleted assertion fails the ratchet. needs: none.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] OPT22 proof: property tests — proptest (license read first, dev
  dependency) for ten invariants: codec round-trips, replay equality,
  snapshot restore, spatial check symmetry, fixed-point bounds; fixed
  seeds in proof, shrinking reported. Tier B. Size M. Owns
  `tests/properties/`. Accept: 10k cases each x2; a seeded bug is found
  and shrunk. needs: none.
  Wave 57 detail (WILD corner-23 Card 3, no new row): spatial_eval
  to_text output gets a normalized compare (timings/paths redacted);
  0 volatile-field golden churn over 5 waves, seeded volatiles caught
  10/10; kill if >2 seeded volatiles escape or normalization touches
  semantic bytes.
- [ ] OPT23 ops: API-break check — cargo-semver-checks (license read
  first) on every public engine crate against the last tagged release.
  Tier C. Size S. Owns the check leg. Accept: a seeded breaking change
  fails; a doc-only change passes. needs: API2.
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
- [ ] OPT27 provider: inference arbiter — local models and the game Progress 2026-10-17: provider side done (arbiter, serving adapter); left: feed live frame p95 and read_vram, window-build budget check on a GPU. Progress 2026-10-21: the provider half landed 50361c3d — `crates/provider/src/pressure.rs` (`PressureFeed`: frame-time p95 window and the VRAM reading reach the arbiter sample, typed notices, `route()` on the same reader) and `VramHeadroom::from_bytes` (the read_vram seam); 273 lib tests and 2 matrix tests green on the runner, clippy clean. Left: the window/game loop must own `PressureFeed` and the window build must hold the P6 frame budget with a local model loaded, which is the render and game lane's half — so the row is NOT ticked.
  share one 6 GB GPU: under frame pressure local inference moves to CPU,
  defers or routes remote; VRAM use reported; the tick never waits.
  Tier B. Size M. Owns `crates/provider/src/arbiter.rs`. Accept: with a
  local model loaded the window build holds the P6 frame budget; requests
  degrade with typed notices. needs: PV2, RA4.
  Correction 2026-10-21: duplicate - PW28 owns this work now; PW28's row says it closes OPT27 and LAI7. This row is not ticked and is not built on its own; retire it when PW28 lands.
- [x] OPT28 provider: prompts as versioned assets — every prompt and Landed aa644693. Run 39052718180.
  brief template is a content-addressed asset with version, provenance
  and an eval set; an edit re-runs its evals and may not lower the score.
  Tier B. Size S. Owns `crates/provider/src/prompt.rs`. Accept: a seeded
  score-dropping edit fails the gate; the prompt digest lands in every
  provider record. needs: OPT24, EV1.
- [ ] OPT29 mcp: agent onboarding budget — tokens an outside agent
  spends from discover to its first applied edit, per vendor, with a
  budget and a ratchet beside the AX3 ledger. Tier B. Size S. Owns the
  measurement test. Accept: counts per vendor x2; the budget holds.
  needs: MCP7, EV1.
  Correction 2026-10-21: duplicate - PW43 owns this work now; PW43's row says it closes OPT29. This row is not ticked and is not built on its own; retire it when PW43 lands.
- [ ] OPT30 ops: crate cards — every crate root header states owner,
  consumers, invariants, budgets and public entry points in one checked
  format that Engine Insight indexes. Tier B. Size S. Owns the
  `workspace-check` rule (orchestrator) plus header edits. Accept: 30 of
  30 crates pass; a seeded missing field fails. needs: API1.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
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
- [ ] OPT32 ops: build-baseline promotion on the quiet box — rerun the
  OPT1 matrix back-to-back x2 (clean plus 4-crate touches x 3 profiles),
  single cargo, lanes halted, load <10%, root `target/`.
  (orchestrator) Tier A. Size S. Owns the promotion. Accept: per-crate
  pairs within 10% with the critical path named; on success the OPT1
  table is marked baseline and OPT4–OPT9 plus OPT12 unpark. HELD for
  the OPERATOR quiet-box window; until then OPT1 stays observed-only.
  needs: OPT1. (OPT1 VERDICT wave 84.)
- [ ] OPT33 operator: DX12 vs Vulkan A/B on the reference GPU with the
  OPT2 harness over the fixed 600-frame arena camera path, x2 within 5%;
  the default backend follows the numbers with the other as fallback and
  startup logs why. Tier B. Size S. Owns the report. Accept: report under
  `target/evidence/backend/` names adapter and driver with CPU and GPU
  p50/p95/p99 per pass, VRAM, draws and validation errors; HELD for the
  OPERATOR quiet-box window (shared with OPT32/PF18b/Q1); Q1 reuses the
  harness. needs: OPT2. (OPT2 VERDICT wave 84.) Progress 2026-10-19:
  operator: reference-GPU DX12 vs Vulkan A/B; line in
  `docs/plan/2026-10-19-owner-queue.md`.
  BLOCKED: needs the reference-GPU DX12 vs Vulkan A/B (operator). Unblocks per docs/plan/2026-10-19-owner-queue.md.
  Progress 2026-10-21T23:50Z: both legs ran 600 fixed-camera frames on the reference adapter (`NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `610.62`) and neither presented a frame — no `interval_ms p50=` receipt, stop after `arena: staged 6 dressing boxes`; dx12 CPU 99.7 % avg with 50 `cargo` / 43 `rustc`, vulkan 99.6 % with 51 / 40. Both contended, so no 5 % A/B, no per-pass column and no default named; the startup line still logs the compiled-in default (`fallback=dx12`). Evidence `target/evidence/backend/opt33-ab-keeper-20261021T2350Z.md`. No tick.
  Progress 2026-10-21T03:37Z: four 600-frame receipts this time, interleaved A,B,A,B on `--scene arena --fighters 8 --frames 600 --exit`, adapter `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062` (dx12) / `NVIDIA 610.62` (vulkan): `interval_ms p50` dx12 16.967 / 16.701 ms (median 16.834 = 59.4 fps) and vulkan 16.854 / 16.984 ms (median 16.919 = 59.1 fps); x2 spread 1.59 % dx12 and 0.77 % vulkan, A/B gap 0.50 %, `validation_errors=0`, `fight_draws=40`. Still contended (CPU 66.8-86.7 % avg per leg, 7-12 `cargo`/`rustc`), and GPU `p50` does not repeat: dx12 2.328 vs 2.090 ms, vulkan 1.088 vs 1.979 ms (82 %), so no default is named. The Accept's p95 column cannot be filled at all: the `arena frame-split` line prints only p50 and p99 for `cpu_ms`/`gpu_ms`/`interval_ms` (`crates/game/src/bin/arena_frame/mod.rs:743`), so a p95 column is code work before this row can tick. Evidence `target/evidence/backend/opt33-ab-keeper-20261021T0330Z.md` with raw stdout x4. No tick.
  Progress 2026-10-21T05:39Z: four more 600-frame receipts, interleaved dx12,vulkan,dx12,vulkan, adapter `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062` / `NVIDIA 610.62`: `interval_ms p50` dx12 18.156 / 16.726 ms (median 17.441 = 57.3 fps) and vulkan 16.756 / 16.874 ms (median 16.815 = 59.5 fps); dx12 x2 spread **8.55 %** over the 5 % bar, vulkan 0.70 %, A/B gap 3.72 %. Still contended, no quiet leg (CPU 59.7-68.2 % avg, 2-5 `cargo` on the first two legs), dx12 GPU `p50` 2.213 vs 2.070 ms (6.92 %) does not repeat, VRAM prints budget only (`usage_mb=0`), and the p95 column stays unprintable (receipt prints p50/p99 only), so no default is named and no tick. Evidence `target/evidence/backend/opt33-ab-keeper-20261021T0539Z.md` + raw stdout x4.
  Progress 2026-10-21T07:37Z: four 600-frame receipts again, interleaved dx12,vulkan,dx12,vulkan, adapter `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062` (dx12) / `NVIDIA 610.62` (vulkan): `interval_ms p50` dx12 16.810 / 17.001 ms (median 16.906 = 59.2 fps) and vulkan 17.141 / 17.054 ms (median 17.098 = 58.5 fps); x2 spread 1.13 % dx12 and 0.51 % vulkan, A/B gap **1.14 %** inside the 5 % bar, `validation_errors=0`, `fight_draws=40`. Contended on every leg (CPU avg 97.7-99.2 %, 15-28 `cargo`/`rustc` + `arena`), so the interval pair is a lower bound; the Accept still fails: GPU per-pass does not repeat (dx12 forward 0.342 vs 0.861 ms, gpu `p50` 0.997 vs 2.072 ms; vulkan forward 0.863 vs 0.221 ms, gpu `p50` 2.055 vs 0.582 ms), VRAM prints budget only (`usage_mb=0`), and the p95 column stays unprintable (receipt prints p50/p99 only), so no default is named and no tick. Evidence `target/evidence/backend/opt33-ab-keeper-20261021T0737Z.md` + raw stdout x4.
  Progress 2026-10-21T09:38Z: four 600-frame receipts, interleaved dx12,vulkan,dx12,vulkan, adapter `NVIDIA GeForce RTX 3050 6GB Laptop GPU`, driver `32.0.16.1062` (dx12) / `NVIDIA 610.62` (vulkan): `interval_ms p50` dx12 16.564 / 16.560 ms (median 16.562 = 60.4 fps) and vulkan 16.664 / 16.559 ms (median 16.611 = 60.2 fps); x2 spread 0.02 % dx12 and 0.63 % vulkan, A/B gap 0.30 % inside the 5 % bar, `validation_errors=0`, `fight_draws=40`, both legs exited alone. Contended on every leg (CPU avg 99.4-99.9 %, 44-49 `cargo`/`rustc`, GPU util 8.8-14.6 %), so still no quiet window. Accept still fails on four counts: p95 per pass unprintable (`frame-split` prints p50/p99, `per-pass` one mean), per-pass numbers do not repeat (vulkan cpu p50 8.490 vs 7.720 ms = 9.97 %, gpu p50 0.921 vs 0.856 = 7.59 %, forward 0.331 vs 0.309 = 7.12 %), VRAM prints budget only (`usage_mb=0`), and no default follows the numbers: two `--backend`-free 300-frame runs logged `backend=auto fallback=dx12 reason=compiled-in default; no pilot report yet` because `crates/render/src/backend.rs:345` calls `pick_backend(resolved, None)`, so the pilot winner is never read. Evidence `target/evidence/backend/opt33-ab-keeper-20261021T0938Z.md` + raw stdout x6. No tick.
