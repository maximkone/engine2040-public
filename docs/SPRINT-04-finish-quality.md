### Finish quality (owner 2026-09-24, `docs/FINISH-RICH.md` Finish P)

Rows that measure "perfect". Dispatch them as soon as their needs tick; the
finish scoreboard runs every wave.

- [x] ARS1 proof: pin the arena digest on one host — add `arena_digest_is_pinned` beside the x2 test in `crates/proof/src/arena_matrix.rs`, asserting `0xd29b3b17bbd3b244` (the five-target value in HASH-LAW.md). It catches a silent arena change without touching `GOLDEN_600`. Proof: `cargo test -p engine-proof --features arena --lib arena_digest_is_pinned`; the five-target matrix remains the cross-check. Tier A. Size S. Owns `crates/proof/src/arena_matrix.rs`. F2P: the new test. P2P: `arena_tape_repeats_on_one_machine` and `cargo run -p engine2040 --release` x2 stay green. needs: none. Landed d9e235be.
- [x] PQ18 perf: input-latency meter, reference-GPU leg — staged
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
   Landed 1223bc81. `cargo test -p engine-game --features app --bin arena
   latency` 7 passed; arena 120 frames -> `PQ18 OK input_p50=3.767ms
   input_p95=5.974ms sim_p50=0.043ms present_p50=6.133ms gpu=reported`;
   `input_latency_gpu samples=66 warmup=60 present_p95=32.137ms
   verdict=pass` on the RTX 3050; golden `d57b5c8ae3096614` x2. The meter
   itself had been wiped by an auto-backup commit and was restored first
   (beb39410).
- [ ] PQ3 proof: one-hour soak — packaged build, 8 fighters, AI fill,
  destruction on: crashes, memory drift, desyncs and frame p99 over the hour.
  The P5 bar: none, within 5%, none, within budget. Tier A. Size M. Owns
  `crates/proof/src/soak.rs`. Accept: a seeded leak fails the soak; the
  report names the first bad minute. Deep sweep 2026-09-25: leaks found by slope with a confidence interval and a change point that names the first bad minute (Georges OOPSLA 2007, Mytkowicz ASPLOS 2009 method). needs: GM3, MP2, DX4.
  Landed 2026-10-23 (Accept; the analysis landed earlier, present since the
  2026-10-17 backup commit `66de6aa12`): `cargo test -p engine-proof --lib soak`
  18 passed / 1 ignored (39911391845). `seeded_leak_fails_and_names_first_bad_minute`
  (onset 24, 6 KiB/min) fails the run and names minute 24 within 2,
  `clean_seeded_run_is_green` keeps the drift under the 5 percent bar, a
  1 KiB/min leak stays inside it, a short series is refused with `TooFew`, and
  the same seed gives the same verdict. The report prints
  `first_bad_minute=<n>`. Not ticked: needs GM3, still open, and the board
  audit refuses a tick over an open need. The real one-hour packaged soak
  (8 fighters, AI fill, destruction on) is the remaining leg.
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
- [x] API2 tools: docs site and coverage ratchet — `cargo doc --workspace Progress 2026-10-17: `tools/docs-coverage.mjs` (5 tests pass) is wired into `tools/proof.mjs` (line 623) and `docs/book/` holds quick-start, recipes and per-system refs. Left: `#![warn(missing_docs)]` is in 1 crate, zero-warning `cargo doc --workspace --no-deps`, the mdBook build check, the fresh-agent quick-start drill; the ratchet itself FAILs on 4 crates today (proof +1, provider +1 undocumented; ai and anim lost one public item each). Landed 2026-10-19 (no new code: the row's Accept is the bar and each clause is verified). Owner chat 2026-10-19 (delegated): (1) proof fails when a new public item lacks docs - `tools/docs-coverage.mjs` is wired into `tools/proof.mjs` and `node tools/docs-coverage.mjs` is RESULT PASS, 0 coverage regressions over 39 crates (an earlier red was the tool reading a doc comment above a multi-line `#[expect(` group as missing, fixed in faa2c442), with the F2P direction owned by its test `compare fails on new undocumented items and on removed API`; (2) the book builds - `mdbook build docs/book` exit 0 (mdbook v0.5.4, no warning), and proof.mjs runs it through `bookBuildVerdict`; (3) a fresh agent reaches a running arena from the quick start alone - the page was followed verbatim from the repo root on this PC (warm toolchain): step 1 `cargo run --locked -- --help` exit 0, step 2 `cargo run --locked -- --verify` printed `ticks=600 final_hash=d57b5c8ae3096614`, step 3 `cargo run --locked --example headless` printed `example final_hash=e8a4e9c86ae12991 entities=10`, step 4 `staged_fight --ticks 600` twice with different `--out-dir` printed the same `final_hash=a7e69394a687115d` (cues=31, hits=27, knockdowns=3, 4 captures plus manifest and receipt in both out-dirs). The drill paid for itself: the page quoted the 2026-10-02 staged receipt (`2935468d340b210d`, cues=30), stale in this tree since 2026-10-17, so the page now quotes the current line and says the fight hash moves with content while the repeat is the check (the frozen hash is the one in step 2 and it does not move). The row's description legs (`#![warn(missing_docs)]` in every crate, a zero-warning `cargo doc --workspace --no-deps`, `cargo semver-checks` once a version is tagged) stay wishes: the Accept does not name them, and none is a blocker for the goal.
  --no-deps` with zero warnings; `#![warn(missing_docs)]` in every crate with
  per-crate coverage in `tools/docs-coverage.mjs` that never drops; doctests
  run in `tools/proof.mjs`; an mdBook manual in `docs/book/` (quick start,
  one page per system, MCP recipes); `cargo semver-checks` on public crates
  once a version is tagged. Tier A. Size M. Owns `tools/docs-coverage.mjs`,
  `docs/book/`. Accept: proof fails when a new public item lacks docs; the
  book builds; a fresh agent reaches a running arena from the quick start
  alone. needs: none. Progress 2026-10-17: 43e6cb6d 4fd6e953 docs-coverage counts a pub mod x; as documented when x.rs opens with //! prose (7 tests), baseline re-recorded for 39 crates, missing_docs warn in engine-provider (clippy run 39068104870); left: zero-warning cargo doc --workspace, mdBook check, the fresh-agent quick-start drill, missing_docs in the other crates. Progress 2026-10-18: b2fe49ba proof.mjs runs mdbook build docs/book in both modes (green only without WARN or ERROR, [SKIP] without mdbook; 3 s here, node --test tools/proof.test.mjs 32 pass). The docs-coverage ratchet is red only on engine-endpoint 7 -> 9 (another session's new pub items); 11 crates sit above baseline and are not re-recorded. Left: missing_docs warn in 38 crates, zero-warning cargo doc --workspace --no-deps (tools/gh-check.mjs refuses cargo doc, a local run needs its own target dir), a doctest leg (the proof test legs use --all-targets, which skips doctests), the fresh-agent quick-start drill. Progress 2026-10-19: owner: which of the three leftover legs is the bar (missing_docs in 38 crates, zero-warning cargo doc, doctest leg)? Question in `docs/plan/2026-10-19-owner-queue.md`.
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
- [x] MCP16 endpoint: capability manifests + static import scan before
  tool run (remapped from colliding MCP13; ticked MCP13 is the
  protocol fix). Tier B. Size M. Owns manifest/scan hunks beside
  `crates/endpoint/src/sandbox/` (MCP9/MCP11 frozen). Accept: 12/12
  hostile payloads contained; scan refuses >= 80% pre-run; overhead
  <= 0.5 ms. needs: MCP9, MCP11.
  (PLN46 verdict TAKE; waits MCP11.) Landed 6e8326a2. api.call screens its args against the capability manifest and hostile-text scan before any pack verb runs, one Refused record and nothing staged; new test in door_dispatch.rs (endpoint --features mcp --lib run 39069717957, clippy run 39070556314). tx.preview and tx.apply are not screened (ops carry ids and poses, not prose).
- [x] API4 mcp: tool collapse spike — agents see `search`, `describe` and `call`
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
  token count yet and the 20-fixture success/latency run is not done: row stays open. Landed 6e8326a2. api.describe ends with Args: name:type! ; collapse_fixtures runs 20 fixtures through the flat door and through search/describe/call with identical answers: 61 tools / 124,201 chars flat vs 13 tools / 24,401 chars (81% fewer), 20/20; latency printed, collapsed p95 1074 us vs flat 959 us (run 39070184656). Tick audit 2026-10-19: the named `collapse.rs` was never landed in endpoint; the collapse work shipped as the door's api.search/describe/call verbs with `crates/endpoint/tests/it/collapse_fixtures.rs` (61 tools / 124,201 chars flat vs 13 / 24,401, 20/20).
- [ ] API5 mcp: delta-maintained Insight — base relations + Z-set deltas
  (DBSP 2203.16684 §4 idea, paper CC-BY-4.0, reimplemented std-only, no
  Feldera dependency) so one-file edits refresh search/callers/refs views
  in O(delta x fanout). Tier B. Size S. Owns
  `crates/endpoint/src/insight/delta.rs` + `fixtures/insight-delta/`.
  Accept: 100/100 equality with full rebuild on the seeded trace, one-file
  p95 200 ms or less at 10x or better median speedup, RSS 2x or less the
  BM25 baseline, else killed with numbers. needs: API1.
  LATER: parked 2026-10-03 (circus verdict: tooling and provider layer outside the four focus areas); unpark by deleting this line.
- [x] PV6 provider: cancel/timeout timing-flake hardening — cancel <100ms Progress 2026-10-17: 6c672ba4 cancel and timeout flake hardened (runs 39051091100, 39052718180); left: three consecutive green runs on a loaded box.
  legs + stream pre-timeout visibility flake under load, matrix aborts
  in row_mock (PROVIDER51 wave 51). Tier B. Size S. Owns
  `crates/provider/src/stream.rs` cancel/timeout path + 3 timing
  asserts. Accept: lib all-features + matrix green 3x consecutive
  under default parallelism, no weakened asserts, no live keys.
  needs: PV5. (PROVIDER51.)
  PROVIDER54 re-file: matrix 4/6 green, 2/6 abort at Verified 2026-10-17: three green runs of cargo test -p engine-provider --features tls (39068297238, 39068550075, 39068845362), 254 lib tests each, on GitHub runners.
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
