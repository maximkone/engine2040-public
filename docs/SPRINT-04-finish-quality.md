### Finish quality (owner 2026-09-24, `docs/FINISH-RICH.md` Finish P)

Rows that measure "perfect". Dispatch them as soon as their needs tick; the
finish scoreboard runs every wave.

- [ ] PQ18 perf: input-latency meter, reference-GPU leg — staged
  input-to-present timestamps in the arena window loop plus one
  quiesced operator run on the reference GPU (RTX 3050) posting
  sim/present p50/p95 against the P2 p95 ≤ 50 ms bar. Tier B.
  Size S. Owns window-loop timestamp hunks beside
  `crates/perf/src/latency.rs` (no tick change) + the GPU report
  rows. Accept: `GpuConditions::report` prints `input_latency_gpu`
  with adapter line, measured (non-PENDING) sim/present p50/p95 and
  a pass/FAIL verdict on the reference GPU; headless
  `headless_report` unchanged; golden unchanged. needs: PQ1.
   (GPU-pilot only: headless writers skip; closes the PENDING column
   PQ1 leaves behind.)
   (DO NOT DISPATCH 2026-10-05: staged leg via PQ18a; headless writers skip parent. Pilot PQ18b only, per 510 deep-dive.)
- [ ] PQ3 proof: one-hour soak — packaged build, 8 fighters, AI fill,
  destruction on: crashes, memory drift, desyncs and frame p99 over the hour.
  The P5 bar: none, within 5%, none, within budget. Tier A. Size M. Owns
  `crates/proof/src/soak.rs`. Accept: a seeded leak fails the soak; the
  report names the first bad minute. Deep sweep 2026-09-25: leaks found by slope with a confidence interval and a change point that names the first bad minute (Georges OOPSLA 2007, Mytkowicz ASPLOS 2009 method). needs: GM3, MP2, DX4.
- [ ] PQ11 proof: slope-with-CI plus changepoint soak-onset (READER-SPEED53
  card 1, quality §4 hardest-first). Tier B. Size S. Owns onset hunks in
  `crates/proof/src/online_soak.rs` (integer-only). Accept: seeded-leak
  caught, 0 false alarms on clean x2, minute error ≤1 window, golden
   unchanged. needs: PQ3. (SPEED53.)
  Wave 54 SPIKE-PQ11 ADOPT: slope+CI+CUSUM clean-None x2, leak w13 err 1
   vs threshold FP, integer-only, golden unchanged → onset hunks may land
   on this row (needs PQ3 still open — writer waits).
- [ ] PQ19 proof: C1 evidence re-run — the `gpu_live` truth gate plus
  `node tools/proof.mjs` on today's tree, creating
  `target/evidence/c1/`. Tier A. Size S. Owns
  `crates/proof/src/gpu_live.rs` (rerun only, no `live` flag flips
  without fresh device evidence) plus the `target/evidence/c1/`
  report. Accept: `cargo test -p engine-proof --lib gpu_live` green
  (seeded stub fails naming it, all 9 ticked passes live) + `node
  tools/proof.mjs` PASS golden `d57b5c8ae3096614` x2 with the report
  reproducing byte-identical x2; golden unchanged. needs: GX6.
  (planner-rows-r2.)
  LATER: parked 2026-10-03 (circus verdict 2: evidence re-run or status-line row, it moves no pixel, hash or timing and finish.mjs stayed 7/16/3 over 10 batches; unpark when a gate needs the receipt); unpark by deleting this line.
- [ ] PQ20 proof: land the C1 evidence line — one `Met:` status-table
  row for C1 above the lumped `B, C, P` row in `docs/FINISH-RICH.md`,
  quoting the PQ19 report (9/9 passes live, seeded-stub refusal names
  the row, proof PASS golden x2), so the gate stops printing partial
  with no open row. Tier A. Size S. Owns `docs/FINISH-RICH.md` (one
  C1 status-table row only, nothing else). Accept: `node
  tools/finish.mjs` prints `C1: met`; `node
  tools/workspace-check.mjs` PASS. needs: PQ19. (planner-rows-r2.)
  LATER: parked 2026-10-03 (circus verdict 2: evidence re-run or status-line row, it moves no pixel, hash or timing and finish.mjs stayed 7/16/3 over 10 batches; unpark when a gate needs the receipt); unpark by deleting this line.
- [ ] PQ4 playtest: reference comparison — every capture in the V1–V5 suite
  scored against the RT7 path-traced reference and the owner's reference
  shots with a perceptual metric (FLIP, NVIDIA HPG 2020, BSD-3, reimplemented;
  SSIM secondary; LPIPS/VMAF only as offline notes, never the gate); scores tracked per build, a regression
  fails the gate. Tier C. Size M. Owns `crates/playtest/src/lookcheck.rs`.
  Accept: a seeded exposure regression is caught; scores repeat within noise.
  needs: Q1, RT7.
- [ ] API2 tools: docs site and coverage ratchet — `cargo doc --workspace Progress 2026-10-17: `tools/docs-coverage.mjs` (5 tests pass) is wired into `tools/proof.mjs` (line 623) and `docs/book/` holds quick-start, recipes and per-system refs. Left: `#![warn(missing_docs)]` is in 1 crate, zero-warning `cargo doc --workspace --no-deps`, the mdBook build check, the fresh-agent quick-start drill; the ratchet itself FAILs on 4 crates today (proof +1, provider +1 undocumented; ai and anim lost one public item each).
  --no-deps` with zero warnings; `#![warn(missing_docs)]` in every crate with
  per-crate coverage in `tools/docs-coverage.mjs` that never drops; doctests
  run in `tools/proof.mjs`; an mdBook manual in `docs/book/` (quick start,
  one page per system, MCP recipes); `cargo semver-checks` on public crates
  once a version is tagged. Tier A. Size M. Owns `tools/docs-coverage.mjs`,
  `docs/book/`. Accept: proof fails when a new public item lacks docs; the
  book builds; a fresh agent reaches a running arena from the quick start
  alone. needs: none.
- [ ] API3 mcp: incremental Insight — a revision memo (salsa's pattern,
  Apache-2.0 OR MIT, no dependency: it pulls rayon and hashbrown) so API1
  rebuilds only what changed. Tier B. Size S. Owns
  `crates/endpoint/src/insight/memo.rs`. Accept: incremental equals full
  rebuild 20 of 20; one-file rebuild under 2 s, else killed. needs: API1.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] MCP12 mcp: WASM sandbox for tool execution — boxagnts-style
  sandbox for agent tools (MIT, PORT cite + EVIDENCE row). Tier B.
  Size S. Owns `crates/endpoint/src/sandbox/wasm_tool.rs`. Accept:
  20 malicious tool calls contained; p95 ≤ +0.5ms. Kill: float math
  in tick. needs: MCP9, MCP11. (RES-STEAL wave 36 Dive 12, reviewed
  2026-09-26; waits MCP11.)
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] MCP16 endpoint: capability manifests + static import scan before
  tool run (remapped from colliding MCP13; ticked MCP13 is the
  protocol fix). Tier B. Size M. Owns manifest/scan hunks beside
  `crates/endpoint/src/sandbox/` (MCP9/MCP11 frozen). Accept: 12/12
  hostile payloads contained; scan refuses >= 80% pre-run; overhead
  <= 0.5 ms. needs: MCP9, MCP11.
  (PLN46 verdict TAKE; waits MCP11.)
- [ ] API4 mcp: tool collapse spike — agents see `search`, `describe` and `call`
  instead of every tool, with summaries they can expand word for word and a
  token-savings report (idea from a 2-star repo whose claims are
  unreproduced). Experiment. Size S. Owns `crates/endpoint/src/collapse.rs`.
  Accept: ≥ 50% fewer tool-definition tokens on 20 fixtures with 0 success
  loss and no extra p95 latency over 10%, else killed with numbers. Hunt 2026-09-25: tools grouped in packs revealed on demand (Unity-MCP skill files and extension packs, Apache-2.0). needs: API1.
  Progress 2026-10-04 (door v2, rows DV1 and DV2 below): the router holds ten core tools
  (`scene.read`, `tx.preview`, `tx.apply`, `tx.undo`, `play.run`, `capture`, `api.search`,
  `api.describe`, `api.call`, `api.load`); the other 13 old tools are verbs in eight packs
  found by `api.search` and called by exact name. Measured advertised list: core 21,326
  chars (47.4% under the 40,514 of the 21-tool door), core plus the default scene pack
  35,193 chars (13.1% under); all 25 tools loaded 53,622. The 50% bar is not met on the
  token count yet and the 20-fixture success/latency run is not done: row stays open.
- [ ] API5 mcp: delta-maintained Insight — base relations + Z-set deltas
  (DBSP 2203.16684 §4 idea, paper CC-BY-4.0, reimplemented std-only, no
  Feldera dependency) so one-file edits refresh search/callers/refs views
  in O(delta x fanout). Tier B. Size S. Owns
  `crates/endpoint/src/insight/delta.rs` + `fixtures/insight-delta/`.
  Accept: 100/100 equality with full rebuild on the seeded trace, one-file
  p95 200 ms or less at 10x or better median speedup, RSS 2x or less the
  BM25 baseline, else killed with numbers. needs: API1.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [ ] PV6 provider: cancel/timeout timing-flake hardening — cancel <100ms Progress 2026-10-17: 6c672ba4 cancel and timeout flake hardened (runs 39051091100, 39052718180); left: three consecutive green runs on a loaded box.
  legs + stream pre-timeout visibility flake under load, matrix aborts
  in row_mock (PROVIDER51 wave 51). Tier B. Size S. Owns
  `crates/provider/src/stream.rs` cancel/timeout path + 3 timing
  asserts. Accept: lib all-features + matrix green 3x consecutive
  under default parallelism, no weakened asserts, no live keys.
  needs: PV5. (PROVIDER51.)
  PROVIDER54 re-file: matrix 4/6 green, 2/6 abort at
   tests/matrix.rs:824 row_mock cancel_leg, observed 111ms vs 100ms
   budget under package-cache lock contention; lib 133/0 green. NOT
   fixed — same fingerprint (timing under load).
- [x] PV7 provider: tool-loop agent harness with budget + transcript — a Landed 6c672ba4. Run 39051091100.
   bounded perceive-plan-act loop over the door (max steps, max tokens,
   stop on done/stuck), ported as std-only Rust from `0xPlaygrounds/rig`
   agent/adapter shape (license MIT, reimplement only, never the crate
   dep). Tier B. Size M. Owns `crates/provider/src/agent_loop.rs`
   (stream frozen). Accept: scripted provider runs a 3-step tool task
   offline; budget overrun stops with a named error; transcript replays
   x2. F2P:
   `cargo test -p engine-provider --lib agent_loop` (fails before: no
   agent loop). P2P:
   `cargo test -p engine-provider --lib offline_adapter_gives_scripted_lines`
   stays green. needs: PV2.
   (STEAL 2026-10-03 rig, https://github.com/0xPlaygrounds/rig.)
