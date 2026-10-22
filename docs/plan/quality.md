# Quality

Goal: the engine stays fast and honest: measured budgets, loud faults, no faked green.
Acceptance:
1. `node tools/rust-quality.mjs` PASS, render D to C or better, ratchet tightened to the new lows (keeper `--tighten`, no `--accept` without a reason).
2. CQ bar: CQ9 zero files over the 1500 bar; CQ10 render casts to zero; CQ14 zero string errors; CQ15 Miri clean on the non-forbid unsafe sites; CQ7 zero render hot-path allocs; CQ24-CQ36 facades under the bar with golden `d57b5c8ae3096614` unchanged x2.
3. PF workloads: hash and allocations exact, ms ceilings in release, `node tools/bench-compare.mjs`.
4. FB receipts: fault matrix, one-hour soak with no drift, versioned perf baseline.
Order (free-first; tree dirty on all code lanes, so cold slices before hot-lane code): CQ36 CQ14 CQ10 CQ7 CQ15 CQ5 CQ24 CQ25 CQ26 CQ27 CQ28 CQ29 CQ30 CQ31 CQ32 CQ33 CQ34 CQ9 PF19 PF18 PF5 PF9 PF9a SL3 SL5 SL21 SL35 SL44 PF16, then table order, LATER last.
Owner crates: perf, observe, proof, tools/.
Open board IDs: CQ5 CQ6 CQ7 CQ8 CQ9 CQ10 CQ14 CQ15 CQ16 FB1 FB2 FB3 FB4 FB5 FB6 FB7 OPT4 OPT5 OPT6 OPT7 OPT8 OPT9 OPT11 OPT13 OPT14 OPT15 OPT16 OPT17 OPT18 OPT19 OPT20 OPT21 OPT22 OPT23 OPT24 OPT25 OPT26 OPT27 OPT28 OPT29 OPT30 OPT31 OPT32 OPT33 PF5 PF9 PF9a PF16 PF18 PF19 SL3 SL5 SL16 SL21 SL23 SL24 SL28 SL29 SL30 SL31 SL32 SL33 SL34 SL35 SL36 SL37 SL38 SL42 SL43 SL44 SL45. AH: none open.
Status 2026-10-21: the CQ ids in the two lines above are stale - CQ24, CQ25, CQ26, CQ27, CQ28, CQ29 and CQ30 are LANDED and ticked on the board (`docs/SPRINT-05-code-quality.md:41-70`; `node tools/board-slice.mjs CQ27` and `CQ30` both print `- [x]`). The line-9 order and the CQ24-CQ30 ids in the line-11 list should come off that list; the CQ30 row below carries the detail.
Status 2026-10-22: the spatial-gold proof gate is fixed, and the ratchet is red on another lane's files. The gate "spatial-gold generator matches tasks.tsv" was failing on a fresh clone of `origin/master` with `first drift line 1: gen="# spatial-gold v1" disk="# spatial-gold v1\r"`: the four `fixtures/spatial-gold/**` TSVs carried CRLF in their blobs while the generator writes LF and compares byte-for-byte, and the checkout converted them again on a Windows box (`core.autocrlf=true`). The fix is `.gitattributes`: `fixtures/spatial-gold/** -text`, the same never-convert rule the khronos and gltf conformance fixtures already carry, plus the blobs renormalized to LF. The content does not change (every file is identical after the CR is stripped) and the Rust TSV parser already trims the CR (`crates/playtest/src/spatial_eval/tsv.rs:568,576,583`). Commits 327dba13a and b8256b511. The golden digest is unmoved: `[PASS] legacy golden - d57b5c8ae3096614` in a fresh clone at c235c63. The rust-quality ratchet is red on two files the VS7 night-rig lane grew, not on any crate this lead touched: `crates/game/src/bin/arena.rs` (5101 -> 5138) and `crates/render/src/present/gpu_present/presenter.rs` (3489 -> 3553), both "a file over 3000 lines cannot grow" (commits 89f94e710 and 016032cde). That lane owns the fix: move a section of each into its own module and shrink the file under 3000 lines. Do not accept the ratchet to make it green.
Pre-merge text: git show d96091cd:docs/plan/quality.md
Pre-merge text: git show d96091cd:docs/plan/engine-edge.md
Pre-merge text: git show d96091cd:docs/plan/finish-b.md
Pre-merge text: git show d96091cd:docs/plan/deepdive-OPT6-2026-10-08.md
Pre-merge text: git show d96091cd:docs/plan/compile-cache-spike.md

## 1. Where it stands

Newcomer first: every canonical workload runs fixed seeds and must match its recorded hash and allocation numbers exactly, while milliseconds are only ceilings measured in release. Faults heal from a bounded catalog or quarantine loudly, and chaos tests prove it. No green number is ever faked: flakes get receipts and quarantine, never silence.

Carried over from `docs/systems/observability-heal.md` (replaces it where they disagree): typed events with stable codes, bounded rings, digest-with-budget, advising-only registry, tick-boundary sim heals with tape records, quarantine over silence.

Landed and off the open list: budget workloads and baselines (PF1), profiler (PF2), release profile (PF3), server budget (PF8), arena allocation shape (PF12), zero-alloc tick (SL6, SL18-SL20, SL22), tournament measurement (PQ7), supervisor, heals and chaos (AH1, AH6, HL2), bug bundles (BUG1), GPU liveness gate (GX6), online soak harness (ON11).

Code as-is 2026-10-15: CQ9 open — 12 files over the 1500 bar (`tools/rust-quality.mjs:33`), headlined by the `post.rs` monolith (`crates/render/src/post.rs:176`, `:453`, `:1541`, `:1923`, `:2407`, `:3009`, no `post/` dir) with `present.rs` leaves still in the facade (`crates/render/src/present.rs:156`, `:530`, `:632`, no `draw_list.rs`/`feed.rs`), `camera.rs` unsplit (file only, no `camera/` dir), and `assistant/` missing `close.rs` (CQ35 `transcript.rs:31` landed, CQ36 open). CQ10 open — casts all over render (`crates/render/src/accel.rs:227`). CQ14 open — string errors live on (`crates/render/src/post.rs:78`, `:87`, `:89`). CQ15 open — non-forbid unsafe remains (`crates/render/src/backend.rs:144`). CQ7 open — render hot-path allocs. Ratchet: FAIL render-only (casts/dup/passthrough/long/big); 9 improved metrics await keeper `--tighten`. Wiring islands grow on every lane; tree dirty on all code lanes (`git status`: `Cargo.lock`, `crates/ai`, `crates/anim`, more), so slices run free-first. SL3 still waits on GX2-GX5/GX7-GX10; PF9 still needs 2,000-frame captures and RA8 coverage; Q1/PQ3/PQ4 still open with zero reference PNGs; `--perf` still blocked on operator daemons; GM3 BLOCKED (gap 3 below).

### Quality gaps (2026-10-10 review)

1. **fps, measure.** The packaged game measured 3.29 fps on the RTX 3050 (GM3 probe 337, BLOCKERS 2026-10-03), and a coder is profiling it today. Next: three quiet runs of the windowed arena (8 fighters, 600 ticks, loop paused) write per-pass p50/p99 to `target/reports/perf-<date>.csv` and name the top 3 passes; noise under 10%, no code change (seed P1/CQ7).
2. **fps, fix one pass.** Halve the cost of the top pass from packet 1 (one file, a half-res or skip switch). Accept: arena with 8 fighters at 1080p reaches 20 fps, then 60; frame_check and the Q1 manifest show no visible loss.
3. **GM3 hang.** Split the 18,000-frame proof into 3 legs of `--frames 6000` with a resume token; ask the owner to exempt `arena.exe --frames` from the reaper smoke class (maxMin 30). Accept: legs at most 30 min each, joined log shows 18,000 frames, exit 0. BLOCKERS 2026-10-02: reaper killed runs at 35 and 63.7 min. Moot if gap 2 reaches 60 fps.
4. **Proof that does not pass: MP10.** Re-land MP10 with `interp.rs` in Fixed math, or an allowlist for presentation-only interp. Accept: workspace-check PASS, `cargo test -p engine-net --lib interp`, golden x2. Landed 65e355f0, reverted c1945cbd (float x4 trips workspace-check).
5. **Proof that does not pass: wan_soak.** Recheck "wan_soak convergence fails on this box" (BLOCKERS 2026-09-23); log the first divergent tick with desync-diff; fix or mark box-specific. Accept: converge in 300 ticks or less, or a dated reason. Feeds ON30.
6. **Unwired code.** Workspace-check row lint: a new row must name a production caller or a consumer row (2 new cases in workspace-check.test.mjs). AM13 AM7 FIX5 FIX3C PX36 MP8 MP6 RT19 each get a caller row or a LATER tag. Eight repeat fails in BLOCKERS; seats rot scan: 130 stubs, 549 unwired.
7. **Flaky timing tests.** Mark wall-clock gates (pair_discovery_bench_prints_lower, cloth couple perf, chaos heals) perf-only and run them in `proof --perf` on a quiet slot. Accept: shared `cargo test -p engine-physics --lib` green 3 times under parallel builds. BLOCKERS: red x3; shared runner kill at 300 s.
8. **Visual verification gate.** After every release build run q1-check.mjs and frame-triage.mjs and open every PNG; a frame equal to the cube or noise SHA fails the wave. Accept: one report per run lists each PNG path and verdict. RL5 RL18 RL20 RL27 RL28 stay landed-but-OPEN for the GPU leg.
   - Status 2026-10-22: the first dated receipt for the GPU leg exists: `docs/research/gpu-window-2026-10-22/gpu-window-rtx3050.md`, measured at commit `b8b2fd542` on this PC's RTX 3050 in a clean worktree of that commit (the shared tree carried another lane's in-flight render code, so `engine-render` did not compile there). `node tools/product-check.mjs --no-build` -> GREEN, fps 60.0 (LOADED, so reported and not gated), framing empty 0/300, tinted 0.12, pops 5, sunk 0, blank 0, frozen 0, validation errors 0, over 300 frames and 2400 bodies. `node tools/arena-shot.mjs --dir target/vs9 --no-build` -> PASS, 3 frames, 3 masks, magenta_max 0.000%, on the pinned arena600 orbit. Every PNG was opened and looked at (8 images, in the receipt's captures/ and product-run/ folders): a real arena with a scutum-and-spear gladiator under a sunset sky, the same fight from another point of the orbit, a wide shot, and an 8-fighter brawl whose HUD bars drain with a spear in flight and blood on the sand. q1-check and frame-triage still read pinned dated reference sets (`target/q1/2026-10-05/orbit`, `target/showcase/2026-10-08/orbit`) that are git-ignored and not on this box, so they report 0 frames; that is a missing reference set, not a refused capture. The receipt ticks none of RL5 RL18 RL20 RL27 RL28: it proves the window opens, the GPU runs, the passes execute and the captures are real, and each of those five features still needs its own dated receipt against its own bar.
9. **Code debt, four packets.** (a) nav: fix the out-of-bounds panic at `crates/nav/src/lib.rs:31-41` with a test that fails first (re-verify the line; rating from 10-08); (b) replace prod expect/unwrap at dialogue.rs:55, lipsync.rs:79, profiler.rs:285 with typed errors; (c) render: convert 100 of 1,009 as-casts per packet to engine-core helpers, rust-quality render above 66; (d) tools: rust-quality `--clippy-count` writes per-crate warning counts (baseline only).
10. **Loop landing gate.** sprint-loop refuses to commit when workspace-check exits non-zero; test in sprint-loop.test.mjs. Board commit f336c06b claimed PASS after a FAIL.
11. **Doc truth.** FOCUS Now drops ticked BX28 BX37 BX38 PX6 (owner edit); plan rows are regenerated from the board, not hand-copied.

- Status 2026-10-17: disk shows the "sprint-loop refuses to commit when workspace-check exits non-zero; test in sprint-loop.test.mjs" half of item 10 above is false. `tools/sprint-loop.mjs` (Length 10378) has 0 `workspace-check` hits (`rg -c "workspace-check" tools/sprint-loop.mjs` exits 1) and calls no git commit at all — each wave only spawns `opencode run --agent orchestrator` (`:183-187`) — and `tools/sprint-loop.test.mjs` (Length 6264) has 0 `workspace` hits over its 8 tests (dry-run oracle, ready-id list, the two usage errors, three prune-queue cases, the QUEUE-PRUNE rename counter). The audit gate that does refuse now lives in `tools/land.mjs`: the audit comment at `:128-133`, `workspaceVerdict` at `:156-171`, and it is the tool that commits (`git add -- <paths>` `:444`, `git commit -F <msg> -- <paths>` `:448`, the close-row pair `:373-374`, push `:458`). The other half of item 10 still holds: board commit `f336c06b` exists ("Board: UI3/UI4 replan plus MP10 lightyear reword plus LADDER P3 lines (workspace-check PASS)"). So the `:38` cite in `docs/plan/BOOK-GAPS-2026-10-12.md` row S14 has drifted to `:39` and the loop/test half of that row needs a fix.
- Status 2026-10-21: the loop half of item 10 is gone from the tree, not just false - `tools/sprint-loop.mjs`, `tools/sprint-loop.test.mjs` and `tools/land.mjs` were all removed on 2026-10-17 (commit `6bdf32bf`, "cut the center link, the keeper, the queue tools and 13 of 14 agents"; none of the three files exists now). The refusal shape they carried is now the by-path land rule in the root `AGENTS.md` "Proof" section plus `node tools/workspace-check.mjs`. The second half of item 10 (board commit `f336c06b`) still holds.

### build and runtime speed (OPT)

- Release profile: fat LTO, one codegen unit; no `panic = "abort"`, no PGO (PF3 note: "pending game-exe evidence").
- Linker: `rust-lld.exe` on `x86_64-pc-windows-msvc`; cold `engine-game` test build 121 s with link.exe, 79 s with lld.
- Fast dev loop (PF6, partial): rust-lld is in the config; no cranelift profile, no hot patching (OPT4, OPT13 open).
- Test runner: `cargo test`; proof legs with 20-30 min timeouts; recurring `-j4` flake fingerprint. Lane builds use shared pools `target/pool-<k>`.
- CPU parallelism: deterministic fork-join; parallel islands, frame arenas and SIMD still open (PF5 in section 7).
- ECS spine: not started; the arena runs outside the ECS (ARC1-ARC7 open).
- Status 2026-10-12: ECS spine row above says not started, but `crates/ecs/src/spine.rs:1` exists (SoA columns) and the arena spine step uses it (`crates/arena/src/spine_step.rs:1`, `crates/arena/src/lib.rs:72`), so that cell is false.
- GPU: wgpu 30.0.1 with `Backends::all()`; no measured DX12 vs Vulkan choice (OPT33). Frame latency fixed at 2.
- Texture streaming: missing (no `crates/render/src/stream.rs`; FIFO whole-texture uploads only). A 2026-09-30 node spike cut uploads 74.6% with zero misses (MIT); the Rust implementation is owed (OPT17).
- GPU-driven culling and meshlets: open (VG1, VG2, VG3, VG8). Release frame time on the reference GPU: never measured (Q1 open).
- Provider: capability negotiation, sessions, jobs, scripted fallback, streaming, 4 vendor adapters. Record and replay only inside the brief chain; no general cassette (OPT24). Model routing, VRAM arbitration and prompt versioning are missing (OPT25-OPT28).
- Test strength: fuzz, chaos, museum; no mutation or property tests yet (OPT21, OPT22).
- Status 2026-10-17: the mutation half of the line above is stale: `tools/mutants.mjs:1` is on disk (Test-Path True, Length 16562) with `tools/mutants.test.mjs` (Length 16655) beside it, so a mutation tool ships; the property half holds (`Select-String -Path Cargo.toml,crates/*/Cargo.toml -Pattern proptest` = 0 hits). Tracked as row S13 in `docs/plan/BOOK-GAPS-2026-10-12.md` (its plan cite `:53` has drifted to this line `:55`).
- MCP door: self-describing, declarative tools, EXPLAIN, audit receipts (MCP7, MCP9, MCP10, EA3).

## 2. Finish line

- **Tier A alpha (SL6 closed, SL18-SL20 ticked):** `node tools/proof.mjs` green with golden `d57b5c8ae3096614` x2; PF1 exact budgets enforced in the default suite on both arena tracks; supervisor catalog green in chaos; SL21 window-alloc audit green; AH6 chaos covers every shipped recovery (zero `Unsupported` rows for landed code). Measurable: gate green, 0 per-tick heap allocs on the arena hot path after warmup, every red receipted, no weakened test.
- **Tier B v1:** Q1 reference-GPU captures plus a perf report; PF2 GPU timestamps resolved against real queries; per-machine baseline table with ratio comparison (PF10); PF3/PF4/PF5 kept only with measured wins; PQ1 latency numbers in the perf report (P2 p95 ≤ 50 ms). Measurable: numbers on named hardware, budgets per machine on file, killed-with-numbers rows for misses.
- **Tier C:** 1080p60 p50/p95/p99 frame evidence on the reference GPU with named quality tiers; 2,000-frame steady-state traces covering every GPU pass; PF9 rerun debugger with tier, VRAM and draw counts in every RA8 manifest; published perf report per release.
- **CQ finish bar (this plan is done when):** Tier A rows CQ5 CQ7 CQ10 CQ14 CQ15 green plus SL21 SL35 SL44 audits green; Tier B row CQ9 closed with CQ24-CQ36 facades under the bar; `node tools/rust-quality.mjs` PASS with render C or better and the baseline tightened to the improved lows; golden `d57b5c8ae3096614` unchanged x2 through every CQ row.

CUT (never): perf autopilot (PA1, parked), balance assistant (BAL1, parked), performance-doctor UI (PD1, parked), vendor GPU-capture pipelines in-repo, a second benchmark harness while PF1 has gaps, kernel anticheat.

### build and runtime speed (OPT)

- **Tier A:** build baselines recorded; a dev build that plays the arena at ≥ 30 fps; link time and proof wall time down by the measured amounts OPT3-OPT7 keep; lane acceptance runs only affected packages; disk use reported and bounded.
- **Tier B:** backend chosen by numbers; bindless and multi-draw paths with fallbacks; texture streaming under a 6 GB budget; frame pacing and dynamic resolution holding 60 fps; provider cassettes, structured-output ladder, scorecard router, VRAM arbiter and versioned prompts; mutation and property tests ratcheted.
- **Tier C:** CPU frame pipelining over the ECS spine; code hot-patching for gameplay; crate fan-out cut; API-break checks on every public crate.

Measurable bars (reference box, release unless named): dev arena ≥ 30 fps; touch-`engine-combat` rebuild and relink ≤ 50% of the OPT1 baseline; proof wall time ≤ 60% of baseline; Base tier 1080p ≥ 60 fps with GPU p95 ≤ 14 ms (P6); input-to-photon p95 −20% at equal fps; zero pipeline-creation hitches on a material walk; an outside agent's first successful edit under a set token budget per vendor.

### dependable v1 receipts (FB)

Tier B v1, seven receipts, each with an evidence line:

- **Transport (FB1):** N = 16 dedicated-server matrix (loss, jitter, reorder, reconnect) converges with full checkpoint equality; every mismatch carries an authority/replay diagnosis naming the culprit tick. Bar: 4 fault cells x2 runs green, 0 undiagnosed.
- **Versions (FB2):** prefab/scene/asset versions stable, migration fixtures green, failed imports refuse typed, incremental cook byte-identical x2, reimport and project move lose no reference. Bar: 0 unlisted versions, 0 lost refs, cook x2.
- **Ship (FB3):** templates boot headless, packaging instructions run verbatim, compat policy enforced by test, provenance manifest on every AI-made object. Bar: newcomer template boots with zero coaching. Remap (UIX10) and accessibility basics (UIX4) stay parked per parked.md and do not gate FB3.
- **AI pillars (FB4):** distilled integer `.e2nn` policy replays bit-exactly, every AI-made object chains provenance, museum loads, migrates and replays alpha tapes and scenes. Bar: digest x2 identical, chain complete, museum green.
- **Soak (FB5):** repeat load/unload/play cycles plus the PQ3 hour: memory within 5%, no desync, no unresolved crash or data loss, first bad minute named on failure. Bar: P5 numbers held in one report.
- **Matrix (FB6):** Windows/Linux release artifacts on the declared matrix, versioned perf baseline, issue/repro process. Bar: every cell has a receipt line or a filed blocker.
- **Workflow (FB7):** an unfamiliar developer runs create, edit, ship; blockers and time recorded; fixes land in the flow. Bar: report posted, every blocker fixed or filed with a row.

Tier A cover (already ticked, FB rows build on it): MP1-MP3, AS1/AS2/AS7, RC4, CT6, TPL1, ED4/ED5, UIX1/UIX5, PF1, LW6, AB11, IN1, TU1, DX4, WIN1, ON2.

CUT (never): consoles, mobile, AAA-scale worlds, engine-wide parity with other engines, every research-archive item, WASM skill packs and code mods (LATER per owner 2026-09-23: data mods and the MCP door cover it). No CUT-genre content, no UE-only kits, no second engine fork.

## 3. Target design

Workloads measure, baselines remember, the gate compares, the supervisor heals, chaos checks the healer, and the slim law deletes whatever lied. Data flow: workloads (`ArenaTick` 32 slots seed 1, `PhysicsStep`, `AiDecide`, `DestructionStep`) run canonical seeds that own all state locally → `WorkloadReport` (ticks, hash, alloc bytes and count) → `check_report` against `BASELINES` (one row per track, selected by `SIM_TRACK`) → `BudgetError` on any miss. Millisecond budgets live separately (release p50 with headroom) and run only under `--perf`.

Ownership: `perf` owns workloads, baselines, budgets, profiler, latency and server-budget harnesses; `observe` owns events, spans, digest, registry and supervisor catalog; `proof` owns gate invocation, fixtures, museum, soak and the GPU-liveness gate. No mirrors: tick-tuning constants live in net only (SL17); FNV-1a converges on `engine_core::Fnv64` (SL5); balance constants converge on single owners (SL16).

Determinism (tier 3): hashes, alloc counts and event sequences are exact; wall-clock ms, GPU timings and provider latencies are presentation quantities, recorded, bounded, reported, never hashed. Sim heals apply at tick boundaries and land in the tape.

Budgets (lane machine, release p50): arena 50 ms, physics 120 ms, ai 5 ms, destruction 25 ms per canonical run; server tick p99 ≤ 4 ms at 64 + 500 AI; event-to-present p95 ≤ 50 ms; zero steady-state allocs for physics and AI slices; exact byte/count rows per arena track.

### build and runtime speed (OPT)

Plain version: measure, change one thing, measure again, keep or kill. Four profiles, one job each: `dev` (workspace opt-level 1, dependencies opt-level 3, so the window runs at playable speed), `fast-test`, `profiling` (release plus line tables and thin LTO, for Tracy and PF2 captures), `ship` (fat LTO, one codegen unit, `panic = "abort"` after an unwind audit, PGO from a recorded tape). Linker `rust-lld.exe` on Windows if the A/B wins. Lanes test only the packages a diff touches plus their reverse dependencies; the full proof runs once per wave.

Runtime CPU: the ECS spine (ARC) holds the performance bars: SoA columns, index order, no per-tick allocation. The sim of tick N+1 overlaps render extraction of tick N; the extraction view is the only sync point, so determinism is untouched.

Runtime GPU: a startup feature ladder (bindless texture arrays, multi-draw indirect count, ray query) picks the fast path per adapter and keeps a tested fallback. Textures stream by mip from GPU feedback under a VRAM budget. The frame loop owns pacing (present mode, frame latency) and dynamic resolution driven by measured GPU time.

AI-agnostic, on top of the C2 contract: request → versioned prompt asset (digest) → router (scorecard, budget) → cassette lookup (hit: recorded result, offline) or adapter → structured-output ladder (native schema, JSON mode, repair, scripted) → arbiter (VRAM, CPU, remote) → record (cassette and provenance). The tick never waits: every rung has a deadline and the scripted fallback is always last (AH5).

Quality for AI-written code: mutation testing shows which tests would still pass if the code were wrong; property tests pin invariants (round-trips, replay equality, spatial checks) across random inputs with fixed seeds; API-break checks guard the compatibility policy (FB3).

### dependable v1 receipts (FB)

Receipts, not systems: each FB row adds one module under `crates/proof/src/fb_*.rs` (beside `museum`, `online_soak`, `tape`) that runs existing harnesses and writes one evidence report under `target/evidence/b/`. Flow: harness → report (ticks, hashes, alloc or resource numbers, refusal lists) → gate check against the section 2 bar → one evidence line. No FB row changes sim behavior.

Ownership: `net` owns transport (`wan_soak::run_matrix` stays the fault engine); `assets`/`scene` own versions, cook and migrations; `editor`/`ui` own templates and onboarding; `game` plus urban own the second game; `nn` owns `.e2nn` bytes; `proof` owns the receipt modules. No mirrors: the fault matrix lives in net only; version numbers in scene/asset registries only; budgets in PF1 baselines only.

Threads: receipts run headless through the same tick, providers off (recorded actions replay, LW6 pattern). Hashes, digests and alloc counts are exact and compared x2; wall-clock ms, GPU and network timings are bounded and reported, never hashed. Receipt modules add at most 5 s wall each in the default suite; the hour soak and matrix captures run under pilot scheduling, with first-bad-minute naming proven on short fixtures first.

Shape: `pub fn run() -> FbReport` (pure over seeded fixtures) and `pub fn check(r: &FbReport) -> Result<String, FbReport>`, which returns the evidence line.

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). GPU time comes from one profiler ring (wgpu-profiler shape): query pools reused, read back frames later, never a wait; every production pass is scoped by name (PF25). Pipeline creation is counted by cause; after load the late count is 0 (OPT34). Cook key = blake3(source, settings, cook version, dependency keys in order) with a begin and end log and one rename per entry (FB8). Replays export to MCAP for Foxglove or Rerun (PF26). A crash report carries the last 120 frame reports. The dev build's critical path is engine-render (294 s of 347 s); a split lands only with --timings before and after (OPT11).

## 4. Hard problems

- Flake vs signal → bounded retry-while-unsettled plus timing-ignore quarantine: setup races under saturation produce errors the mapping never asserts on, so retrying only those preserves every proof (PF1 50x gate, 26-socket settle).
- Budgets vs machine variance → record-once p50 with headroom, release-only enforcement, per-machine rows with ratio comparison (PF10). The default suite enforces exact quantities; `--perf` enforces ceilings per named machine. Needed by PF19 and PF18.
- Soak-onset detection → slope with confidence interval plus change point naming the first bad minute (PQ3 design; Georges 2007 and Mytkowicz 2009 method, reimplemented std-only). Needed by FB5.
- Dirty-tree collisions → free-first order: scene/endpoint/core slices (CQ31-CQ36, CQ14) and packet-filed cuts land before hot-lane render code (post split CQ24-CQ30, submit/live). Why: every code lane is dirty, so a render-monolith edit collides with any running seat; cold leaves merge clean. Reference: claims in `target/queue/claims.txt`.
- Passthrough hot block → cut packet-filed passthrough wrappers first (they own no behavior), then fold the hot-path remainder per call site with before/after counts in the receipt. Why: passthrough FAIL sits on the render hot path where a blind fold risks frame cost; cuts are collision-free, folds stay small and measured. Needed by CQ7.
- Post split size → pure-move slices with a `pub use` facade, one leaf per row (CQ24-CQ30 chain), behavior-verbatim with CPU pixels bit-identical x2. Why: `post.rs` (~3.3k lines) trips the ratchet-baseline gate on any LF4 touch, so only verbatim moves pass review; each row stays S/M and LF4 waits for CQ27 + CQ30.

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4.

## 6. Slim

Delete: ponytail and script crates, legacy demo, world water (done, SL1); per-frame `Vec` allocations on the arena hot path (SL6 scaffold done, residuals in SL18-SL20 and SL21); mirrored tick-tuning constants (done, SL17); mirrored hit-react constants (done, SL8); mirrored hash buffers (done, SL9, SL13-SL15). Merge: FNV-1a sites into `engine_core::Fnv64` when each crate is touched anyway (SL5); balance constants into single owners (SL16). Freeze: baseline rows append-only per track (re-record on behavior change with deltas, hashes and the golden check in the same packet); the golden changes only with an explicit bump receipt; `deny.toml` authoritative (orchestrator edits only). We will not build: a perf autopilot, a balance assistant, a performance-doctor UI, vendor GPU-capture pipelines in-repo, a second benchmark harness, or window-path equivalents of headless gates without explicit N/A. Test count is not a quality metric: every new test pins a behavior, a budget or a refusal.

Execution order (free-first, slim law): (1) packets-filed cuts first — CQ36 close leaf and any filed cut rows, collision-free; (2) typed-error slices second — CQ14 leaf by leaf, one owner per error type; (3) cast slices third — CQ10 in small per-file packets with engine-core helper reuse; (4) splits last with size warnings — CQ24-CQ30 post chain then CQ31-CQ34, each a pure move capped at Size M, facade kept, LF4 held until CQ27 + CQ30. CQ7 alloc cuts and CQ15 Miri ride beside (2)-(3) on cold files first. Render post/submit/live code goes last or not at all this round.

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): PF25 GPU pass timing, PF26 MCAP replay export, CQ23 CPU naga shader gate, OPT34 pipeline counter, OPT35 vsync pacing, FB8 cook key.

Rows from the books (2026-10-12, board shard `../SPRINT-20-game.md`; `BOOK-GAPS-2-2026-10-12.md` 2.10, 2.12): SL46 the fight tick joins the float gate, CQ17 a loom model of the job pool (Miri finds no races), CQ18 release keeps overflow checks, CQ19 GPU uniform size asserts and `.to_bytes()` in the ALLOC gate, CQ20 private newtype fields and one owner for caches, CQ21 an acyclic-graph test and an instability alarm, CQ22 decision records in `docs/adr/`.

Ticked rows are dropped. `needs` is the open prerequisite (`waits`), `ready` means none is open. Full Accept lines: `grep -h "^- \[ \] <ID> " docs/SPRINT*.md`.

- [ ] PF19 Quiesced reference-GPU fill of the PF13 GPU/VRAM rows, pilot-only, no lane, no code. Accept: `node tools/perf-baseline.mjs --check` PASS with all 11 rows `status: baseline` carrying ordered p50/p95/p99 quantiles + golden `d57b5c8ae3096614` x2 unchanged; a loaded-box run still lands observed-only and never overwrites a baseline row.
- [ ] PF18 Mimalloc as the bench-harness allocator on PF1 release workloads, hashes and alloc counts pinned; ms stay uncertified until the HARDEN follow-up (21 samples, quiesce rule) lands. Accept: wall ms delta at identical hash/alloc rows; >= 10% faster or dropped.
- [ ] PF5 SIMD pass on the hot loops (culling, CPU skinning paths, spatial queries, audio mix) with scalar fallbacks; integer simulation math stays. Accept: each change A/B-measured; identical outputs.
PF15 is not on the board (no row, not in FOCUS Now); it is dropped here and stays in git.

Post monolith split (plan-only, unblocks LF4; refines CQ9, which stays open).
`crates/render/src/post.rs` carries 3346 production lines over the 1500 file
bar (`docs/BLOCKERS.md` 2026-10-14T03:46Z; bar at `tools/rust-quality.mjs:33`),
so any LF4 touch to it trips the ratchet-baseline gate. Each slice below is a
pure move into `crates/render/src/post/<name>.rs` with a `pub use` facade kept
in `post.rs`, so every `post::X` path callers use stays valid and CPU pixels
stay bit-identical x2. The facade keeps only the shared vocabulary
(`PostError`, receipts, bounds); behaviour never changes in a split row. The
orchestrator boards these after `node tools/workspace-check.mjs` PASS; LF4
lands after CQ27 (chain isolated) and CQ30 (facade under the bar).

- [ ] CQ24 quality: split post jitter+motion leaf out of the monolith. Tier B.
  Size S. Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/jitter_motion.rs`. Accept: `halton`, `jitter_*`,
  `MotionField`, `check_dims`/`check_image` move verbatim
  (`crates/render/src/post.rs:176`, `:243`, `:305`); facade re-exports them;
  `cargo test -p engine-render --lib post` green; golden unchanged. F2P:
  `grep -c "pub fn halton" crates/render/src/post/jitter_motion.rs` fails
  before (file missing), passes after. P2P: `cargo test -p engine-render
  --lib post` exit 0. needs: none.
- [ ] CQ25 quality: split post TAA resolve + ghost scene out. Tier B. Size S.
  Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/taa.rs`. Accept: `TaaParams`, resolve helpers,
  `taa_resolve`, `taa_resolve_jittered`, `GhostingScene` move verbatim
  (`crates/render/src/post.rs:348`, `:453`, `:2065`, `:2410`); clamped trail
  still under `GHOST_MAX_TRAIL`; lib green; golden unchanged. F2P: `grep -c
  "pub fn taa_resolve" crates/render/src/post/taa.rs` fails before, passes
  after. P2P: `cargo test -p engine-render --lib post` exit 0. needs: CQ24.
- [ ] CQ26 quality: split post upscale + FSR + shimmer scene out. Tier B.
  Size S. Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/upscale.rs`. Accept: `taa_upsample`, `fsr_*`,
  `ShimmerScene` move verbatim (`crates/render/src/post.rs:507`, `:616`,
  `:856`, `:2481`); MTF50 lift and shimmer gates still post numbers; lib
  green; golden unchanged. F2P: `grep -c "pub fn fsr_upscale"
  crates/render/src/post/upscale.rs` fails before, passes after. P2P: `cargo
  test -p engine-render --lib post` exit 0. needs: CQ24.
- [ ] CQ27 quality: split post bloom chain (LF4 dependency) out. Tier B.
  Size M. Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/chain.rs`. Accept: bloom/exposure/ACES/grade
  configs, `apply_bloom`, `run_chain`, `run_frame` move verbatim
  (`crates/render/src/post.rs:967`, `:1165`, `:1365`, `:1860`); torch halo
  lifts the neighbour while a sub-knee field stays identical; lib green;
  golden unchanged. F2P: `grep -c "pub fn apply_bloom"
  crates/render/src/post/chain.rs` fails before, passes after. P2P: `cargo
  test -p engine-render --lib post` exit 0. needs: CQ25, CQ26.
- [ ] CQ28 quality: split post auto-exposure + exposure scene out. Tier B.
  Size S. Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/auto_exposure.rs`. Accept: `AUTO_*`,
  `AutoExposure`, `run_chain_auto`, `run_frame_auto`, `ExposureScene`,
  `clipped_share` move verbatim (`crates/render/src/post.rs:1457`, `:1731`,
  `:1808`, `:2584`); day-sweep tracking and torch-trim behaviour unchanged;
  lib green; golden unchanged. F2P: `grep -c "pub struct AutoExposure"
  crates/render/src/post/auto_exposure.rs` fails before, passes after. P2P:
  `cargo test -p engine-render --lib post` exit 0. needs: CQ27.
- [ ] CQ29 quality: split post RP1 graph + scale seam out. Tier B. Size S.
  Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/graph.rs`. Accept: RP1 pass names, transient
  bytes, `build_rp1_graph`, `compile_rp1_for_present`, the `aa_scale` seam
  move verbatim (`crates/render/src/post.rs:2135`, `:2204`, `:2346`);
  compiled order stays the six RP1 names; lib green; golden unchanged. F2P:
  `grep -c "pub fn build_rp1_graph" crates/render/src/post/graph.rs` fails
  before, passes after. P2P: `cargo test -p engine-render --lib post` exit 0.
  needs: CQ27.
- [ ] CQ30 quality: split post GPU submitter out, facade under the bar.
  Tier B. Size M. Owns `crates/render/src/post.rs` facade hunk + new
  `crates/render/src/post/gpu.rs`. Accept: the window-only `mod gpu` body
  (`Rp1Submitter`, resolve, frame) moves verbatim
  (`crates/render/src/post.rs:2650`, `:2806`, `:3133`); `post.rs` production
  lines read under 1500 and `big_files` no longer lists it; lib green;
  golden unchanged; LF4 is unblocked. F2P: `grep -c "pub struct
  Rp1Submitter" crates/render/src/post/gpu.rs` fails before, passes after.
  P2P: `cargo test -p engine-render --lib post` exit 0. needs: CQ27, CQ28,
  CQ29.
  Status 2026-10-21: this row and CQ24-CQ29 are LANDED on the board
  (`docs/SPRINT-05-code-quality.md:41-70`, all ticked; CQ27/CQ28/CQ30 landed
  781ecd67, CQ29 landed 34fae3ba) and the files are on disk
  (`crates/render/src/post/{taa,upscale,chain,auto_exposure,graph,gpu}.rs`,
  plus `crates/render/src/post_jitter_motion.rs` for CQ24). The `post.rs`
  line cites above are stale - `post.rs` is 2,376 lines, so `:2650`, `:2806`
  and `:3133` no longer exist; the moved symbols now read
  `crates/render/src/post/gpu.rs:163` (`pub struct Rp1Submitter`) and `:363`
  (`pub fn resolve`), `crates/render/src/post/chain.rs:923` (`pub fn
  run_frame`). CQ24's own path is also wrong: the leaf is the flat file
  `crates/render/src/post_jitter_motion.rs` (`pub fn halton`), not
  `crates/render/src/post/jitter_motion.rs`; the other new leaves read
  `post/taa.rs` (`pub fn taa_resolve`), `post/upscale.rs` (`pub fn
  fsr_upscale`) and `post/chain.rs` (`pub fn apply_bloom`). The plan's own
  open list (line 9 order, line 11 open ids) and the CQ24-CQ30 rows above
  still show them as open.

Next chain (plan-only, refines CQ9; orchestrator boards after
`node tools/workspace-check.mjs` PASS). `crates/render/src/present.rs` keeps
the facade plus `DepthFeed`, draw-list/bins and scene-variant leaves beside
its already-split `aa`, `cache`, `look`, `surface`, `wait_stats`,
`gpu_present` modules (`crates/render/src/present.rs:118-124`); each slice
below is a pure move with a `pub use` facade kept, every caller path valid,
behaviour unchanged. `crates/game/src/camera.rs` splits director from
fixture (`crates/game/src/camera.rs:787`, `:1031`); the fixture leaf carries
its own errors and tables. `crates/endpoint/src/assistant.rs` splits
transcript/proposal from fork/apply close
(`crates/endpoint/src/assistant.rs:141`, `:720`, `:893`); the loop shell
stays. Method reference (no bytes ported):
`bevyengine/bevy@c8d199b192a1b173746f7a160d259c4db8ab7789:crates/bevy_render/src/batching`
(API listing 2026-10-14, one concern per module with a facade re-export).

- [ ] CQ31 quality: split present draw-list + bins leaf out. Tier B. Size S.
  Owns `crates/render/src/present.rs` facade hunk + new
  `crates/render/src/present/draw_list.rs`. Accept: `DrawItem`, `DrawList`,
  `BinKey`, `InstanceRecord`, `group_into_bins` move verbatim
  (`crates/render/src/present.rs:459`, `:530`, `:581`, `:618`, `:632`);
  facade re-exports them; consumer WIN1 window loop unchanged; lib green;
  golden unchanged. F2P: `grep -c "pub struct DrawList"
  crates/render/src/present/draw_list.rs` fails before, passes after. P2P:
  `cargo test -p engine-render --lib present` exit 0. needs: none.
- [ ] CQ32 quality: split present depth feed + scene variants leaf out.
  Tier B. Size S. Owns `crates/render/src/present.rs` facade hunk + new
  `crates/render/src/present/feed.rs`. Accept: `DepthFeed`,
  `decode_depth_feed`, `scene_variant_keys_for`, `scene_variant_keys`,
  `WalkReport`, `walk_pipelines` move verbatim
  (`crates/render/src/present.rs:156`, `:194`, `:368`, `:401`, `:410`,
  `:430`); probe-only decode stays off the live frame; lib green; golden
  unchanged. F2P: `grep -c "pub struct DepthFeed"
  crates/render/src/present/feed.rs` fails before, passes after. P2P: `cargo
  test -p engine-render --lib present` exit 0. needs: CQ31.
- [ ] CQ33 quality: split game camera director leaf out. Tier B. Size S.
  Owns `crates/game/src/camera.rs` facade hunk + new
  `crates/game/src/camera/director.rs`. Accept: `DirectorParams`,
  `CameraPose`, `CameraFrame`, `select_lock_target`, `frame_pair`,
  `frame_group`, `shake_offset`, `Director` move verbatim
  (`crates/game/src/camera.rs:138`, `:366`, `:376`, `:433`, `:498`, `:556`,
  `:761`, `:787`); consumer windowed leg reads the same bytes x2; lib
  green; golden unchanged. F2P: `grep -c "pub struct Director"
  crates/game/src/camera/director.rs` fails before, passes after. P2P:
  `cargo test -p engine-game --lib camera` exit 0. needs: none.
- [ ] CQ34 quality: split game camera fixture leaf out. Tier B. Size S.
  Owns `crates/game/src/camera.rs` facade hunk + new
  `crates/game/src/camera/fixture.rs`. Accept: `FixtureError`,
  `FixtureCamera`, `FixtureInput`, `FixtureRules`, `FixtureCheckpoint`,
  `FixtureTape`, `FixtureSession`, `FixtureMode` move verbatim
  (`crates/game/src/camera.rs:1031`, `:1128`, `:1191`, `:1206`, `:1472`,
  `:1481`, `:1556`, `:1783`); facade re-exports them; `camera.rs` keeps only
  shared vocabulary; lib green; golden unchanged. F2P: `grep -c "pub struct
  FixtureMode" crates/game/src/camera/fixture.rs` fails before, passes
  after. P2P: `cargo test -p engine-game --lib camera` exit 0. needs: CQ33.
- [ ] CQ35 quality: split assistant transcript + proposal leaf out. Tier B.
  Size S. Owns `crates/endpoint/src/assistant.rs` facade hunk + new
  `crates/endpoint/src/assistant/transcript.rs`. Accept:
  `TranscriptEntry`, `Proposal`, `PreviewReceipt`, `ProposalConflict`,
  `count_tokens` move verbatim
  (`crates/endpoint/src/assistant.rs:81`, `:141`, `:152`, `:268`, `:285`);
  consumer EA1e loop unchanged; lib green; golden unchanged. F2P: `grep -c
  "pub struct Proposal" crates/endpoint/src/assistant/transcript.rs` fails
  before, passes after. P2P: `cargo test -p engine-endpoint --lib
  assistant` exit 0. needs: none.
- [ ] CQ36 quality: split assistant fork + apply close leaf out. Tier B.
  Size S. Owns `crates/endpoint/src/assistant.rs` facade hunk + new
  `crates/endpoint/src/assistant/close.rs`. Accept: `ForkProof`,
  `ForkBreach`, `ForkTestError`, `ApplyReceipt`, `UndoReceipt`, `ApplyError`
  move verbatim (`crates/endpoint/src/assistant.rs:720`, `:752`, `:772`,
  `:893`, `:916`, `:931`); `AssistantLoop` shell stays in the facade; lib
  green; golden unchanged; CQ9 stays open (facades still over the bar).
  F2P: `grep -c "pub struct ApplyReceipt"
  crates/endpoint/src/assistant/close.rs` fails before, passes after. P2P:
  `cargo test -p engine-endpoint --lib assistant` exit 0. needs: CQ35.

Row changes: CQ9 stays open until CQ36 lands (was CQ30); CQ24-CQ30 keep
their board text. LF4 keeps its board text (`docs/SPRINT-18-gaps.md:1427`).

Proposals for the lead only (planner proposes, orchestrator boards):
add CQ37 typed-error slice packet 2 (post.rs `PostError` String variants at `crates/render/src/post.rs:78`); merge SL37 into the CQ10 cast packets (same files, same helper); merge SL42/SL32 samples into CQ10 packet 1; kill-or-LATER OPT6 (parked: nextest not installed) and OPT21 (no mutation harness this round); kill PF15 (not on the board, not in FOCUS Now). No new board rows from this packet; at most 8 rows per packet when boarded.

### PF, SL, CQ rows

| ID | title | Tier | needs |
|---|---|---|---|
| CQ5 | tick time | A | ready |
| CQ7 | allocation-free frame | A | ready |
| CQ9 | split the monoliths | B | ready |
| CQ24 | split post jitter+motion leaf | B | ready |
| CQ25 | split post TAA resolve + ghost scene | B | CQ24 |
| CQ26 | split post upscale + FSR + shimmer | B | CQ24 |
| CQ27 | split post bloom chain (LF4 dep) | B | CQ25 CQ26 |
| CQ28 | split post auto-exposure + scene | B | CQ27 |
| CQ29 | split post RP1 graph + scale seam | B | CQ27 |
| CQ30 | split post GPU submitter, facade under bar | B | CQ27 CQ28 CQ29 |
| CQ31 | split present draw-list + bins leaf | B | ready |
| CQ32 | split present depth feed + scene variants | B | CQ31 |
| CQ33 | split game camera director leaf | B | ready |
| CQ34 | split game camera fixture leaf | B | CQ33 |
| CQ35 | split assistant transcript + proposal leaf | B | ready |
| CQ36 | split assistant fork + apply close leaf | B | CQ35 |
| CQ10 | checked numerics | A | ready |
| CQ14 | typed errors | B | ready |
| CQ15 | Miri UB gate on non-forbid unsafe sites | B | ready |
| PF9 | replay visual debugger | B | ready |
| PF9a | headless paired-rerun harness (Tracy offline-capture shape) | B | ready |
| SL3 | CPU-only render references | A | ready |
| SL5 | one FNV | B | ready |
| SL21 | window-path per-frame allocation audit | A | ready |
| SL35 | zero-alloc render frame | A | ready |
| SL44 | window-submit per-frame allocation audit | B | ready |
| PF16 | tick-timeline rerun sink | B | waits PF9 |
| CQ6 | history out of comments | A | LATER |
| CQ8 | one owner per behavior | A | LATER |
| CQ16 | std-only file-snapshot and redaction helper for goldens | A | LATER |
| SL16 | shared balance constants | A | LATER |
| SL23 | single-owner texture bounds | A | LATER |
| SL24 | one FNV in game | B | LATER |
| SL28 | one FNV owner in physics | A | LATER |
| SL33 | shared spawn throttle in render | B | LATER |
| SL34 | one FNV owner in endpoint | A | LATER |
| SL36 | timer out of spatial production | B | LATER |
| SL37 | checked conversions sample in render tick | B | LATER |
| SL38 | sim-clippy test-only expects in scene/spatial | B | LATER |
| SL42 | checked conversions in the recorder tick | B | LATER |
| SL43 | board-ID/history comment scrub sample | C | LATER |
| SL45 | engine-core owner for the leftover folds | B | LATER |
| SL29 | one FNV owner in net | A | LATER |
| SL30 | BTree hierarchy in scene | A | LATER |
| SL31 | BTree maps + timer waiver in net | A | LATER |
| SL32 | checked conversions in physics tick | B | LATER |

### OPT rows: build and runtime speed

| ID | title | Tier | needs |
|---|---|---|---|
| OPT4 | linker A/B and PF6 re-verify | A | ready |
| OPT5 | profiles with one job each | A | ready |
| OPT6 | nextest spike | A | parked: nextest not installed |
| OPT7 | affected-package gate | A | ready |
| OPT8 | feature unification spike | B | ready |
| OPT9 | compile cache spike | B | ready |
| OPT13 | gameplay code hot-patch spike | C | ready |
| OPT14 | spine performance bars | B | ready |
| OPT17 | texture streaming | B | ready |
| OPT18 | frame pacing and latency | B | ready |
| OPT19 | zero first-use stutter | A | ready |
| OPT20 | dynamic resolution | B | ready |
| OPT22 | property tests | B | ready |
| OPT24 | response cassettes | B | ready |
| OPT25 | structured-output ladder | B | ready |
| OPT27 | inference arbiter | B | ready |
| OPT29 | agent onboarding budget | B | ready |
| OPT31 | dev window arena at 30 fps on the reference GPU | A | ready |
| OPT32 | build-baseline promotion on the quiet box | A | ready |
| OPT33 | DX12 vs Vulkan A/B on the reference GPU with the OPT2 harness | B | ready |
| OPT11 | crate fan-out cut | C | waits OPT7 |
| OPT15 | CPU frame pipelining | C | waits ARC6 |
| OPT16 | GPU feature ladder | B | waits VG1 |
| OPT23 | API-break check | C | the baseline is re-blessed (44ebebe18, 2026-10-22): 0 breaks over 38 crates at 70ffe43d6. The row stays open: `api-break` is still not a CI gate (`ci.yml` runs `workspace-check` and `rust-quality` only), so a break is only found when someone runs the tool. Wiring it into CI is the rest of this row. |
| OPT26 | scorecard router | B | waits OPT24 |
| OPT28 | prompts as versioned assets | B | waits OPT24 |
| OPT21 | mutation testing ratchet | A | LATER |
| OPT30 | crate cards | B | LATER |

### FB rows: dependable v1 receipts

| ID | title | Tier | needs |
|---|---|---|---|
| FB2 | version/cook receipt | B | ready |
| FB3 | ship-policy receipt | B | ready |
| FB1 | dedicated-server fault-matrix receipt | B | ready |
| FB4 | AI-pillars-in-sample receipt | B | ready |
| FB5 | cycle + hour soak receipt | B | waits PQ3 |
| FB6 | hardware-matrix + baseline receipt | B | waits Q1 GM3 |
| FB7 | stranger-dev workflow receipt | B | waits ED18 |

## 8. Quality gates

- `node tools/proof.mjs` green with golden `d57b5c8ae3096614` x2, kept green forever; `--perf` green with operator daemons.
- `node tools/rust-quality.mjs` PASS with no crate grade drop (render C or better) and baseline at the tightened lows; every CQ row carries golden-unchanged x2 plus its lib-test green line.
- PF1/PF10 budgets fail on a seeded regression and a seeded allocation; behavior-changing perf packets re-record baselines same-wave with deltas, hashes and the golden check.
- Every supervisor catalog fault heals or quarantines within budget in chaos; every new subsystem ships its chaos rows; no `Unsupported` row survives its recovery landing (AH6).
- Fuzz suites save failing tapes; the museum loads every released version; bug bundles replay to the first divergent tick.
- Same fingerprint twice → `../BLOCKERS.md`; timing-only → `#[ignore]` + `--perf`; weakened tests or edited goldens rejected at review; per-frame allocation claims proved by mutation.
- Window-path gates get headless-equivalent coverage or an explicit N/A with numbers; lib.rs and Cargo.toml edits get an immediate diff check.

### build and runtime speed (OPT)

- Every kept OPT change has before/after numbers in its receipt from the same machine and profile; misses are reverted the same wave.
- Golden `d57b5c8ae3096614` unchanged by every profile, linker and runner change (they must not touch integer simulation), asserted x2.
- Tile residency (OPT35) keeps the upload cut at 30% or more with 0 misses, with tile and bulk digests x2 on the 64-texture sweep; any miss, digest drift or feedback stall on the frame path fails the gate.
- The OPT1 build table and the OPT2 frame report rerun every 5 waves with BENCH; a regression past 10% is a red gate.
- OPT21 mutation ratchet and OPT22 properties run in proof once landed.
- Provider rows keep "unknown cost is never zero" and "the tick never waits"; cassettes never answer a request whose digest differs.
- OPT6 nextest spike (parked: `cargo-nextest` is not installed): the writer is unblocked when `cargo nextest --version` prints a version, then one before/after wall-time run keeps flake counts visible. Source: `docs/archive/2026-10-10/plans/deepdive-OPT6-2026-10-08.md`.
- OPT9 compile cache spike (sccache, measurement only, no Cargo or lanes change): cold check 300 s should drop to warm 70 s or better, warm hit rate above 80%, two warm runs within 10%; never commit from the spike. Source: `docs/archive/2026-10-10/plans/compile-cache-spike.md`.

### dependable v1 receipts (FB)

- Every row: golden `d57b5c8ae3096614` identical twice; `node tools/workspace-check.mjs` PASS on the landed tree.
- Transport gate: 4 fault cells x2 runs converge with full checkpoint equality; a seeded cheat or mismatch gets a diagnosis naming the tick.
- Version gate: zero unlisted versions; a seeded bad version and a seeded lost reference both fail loudly; cook bytes x2.
- Ship gate: template boots headless from instructions verbatim with zero coaching; a compat violation fails the test.
- Pillars gate: `.e2nn` digest identical x2; provenance chain complete per AI-made object; museum fixtures green.
- Soak gate: P5 numbers (no crashes, memory within 5%, no desyncs, frame p99 within budget); a seeded leak names the first bad minute via slope CI and changepoint.
- Matrix gate: every hardware cell receipted or blocker-filed; the baseline carries a version that bumps with a reason.
- Workflow gate: time posted, blockers recorded, fixes in-flow; coached-around blockers fail review.
- No-render rule: no FB row submits GPU work; pilot-measured pixels arrive as receipt lines, never lane-synthesized.

## 9. Session estimate

Session estimate: not kept here; see `git show d96091cd:docs/plan/quality.md`, section 9. The open rows in section 7 are the current scope.
