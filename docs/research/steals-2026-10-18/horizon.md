# Steal card: ludovic111/horizon (2026-10-18)

## What it is

A from-scratch Rust rewrite of **Minecraft: Java Edition 26.3**, aiming for one-to-one vanilla parity and checked against the real game with **281 differential harnesses**. Native client (Linux/macOS/Windows), server, launcher, relay, bots. 179 stars, 95 MB tree, HEAD `b0b441439771` (2026-10-18T16:37Z), language Rust. Read live 2026-10-18 (docs, tools, crates; read-only). Nothing built or run.

## Licence

**MIT** — `LICENSE` first line "MIT License" (read live today); GitHub API `license.spdx_id` = `MIT`.

## Steals (ranked by value per hour)

1. **A dependency-tracked test runner with content-keyed shared passes and a replayed oracle.** `docs/parity-workflow.md:7` (`tools/parity.py` runs the `test_*_parity.py` harnesses in parallel and reruns only what a change can affect), `:23` (a tracer records every file a harness depended on — Python modules, the Java oracle and its sources, other repo files, and the Rust sources of the probes from Cargo `.d` files; a pass stands until one changes), `:25` (`tools/parity_shim.py` stands in for `java`/`javac`, records each oracle run and replays it while inputs are unchanged — `block_update` 53 s → 5 s after a Rust-only change), `:29` (a pass is written to `parity-shared/` keyed on **content of dependencies, not location**, so a worktree whose files match counts a pass without running it).
   - Maps onto engine2040: `tools/affected.mjs:1-31` already maps a diff to a reverse-dependency closure with a green-skip, and `:28-32` closes over `cargo metadata`. The delta is a per-test pass cache keyed by recorded dependency content shared across worktrees, plus a replayed stable oracle. Lands in `tools/affected.mjs` and the `gh-check` flow, never in a shipped crate.
   - **Kill:** if the recorded dependency set cannot be shown closed, keep the coarse `cargo metadata` closure — a false pass costs far more than a rerun.

2. **A declared closed Rust scope with validation.** `docs/parity-workflow.md:44` — a harness declares `PARITY_RUST_SCOPE = [...]`, the path prefixes its probes can reach; the runner keeps only those files **if the scope is closed**: nothing in it names `crate::`, `super::`, another workspace crate, declares an uncovered module, or includes source, and nothing outside implements anything for a type it defines. Payoff is named: `datafix` depends on 41 files, not 354.
   - Maps onto engine2040: `tools/affected.mjs:27-32` closes over the whole declared graph, so `crates/core/src/lib.rs` maps to 36 of 36 packages. A per-gate scope that is *proven closed* before it prunes would narrow a lane gate.
   - **Kill:** a scope that cannot be proven closed keeps every file — never prune on a guess.

3. **Platform-accurate float math (`jmath`).** `docs/parity-workflow.md:37`: vanilla's `Math.sin`, `pow`, `log` do not round the same on Apple silicon (HotSpot uses fdlibm) as on x86-64 (Intel stubs), and Rust's float functions round like neither (they call the platform libm), so mirrored code uses `horizon_core::jmath`, which computes what HotSpot computes on the host architecture.
   - Maps onto engine2040: the hash law assumes identical bits across machines. Integer trig already exists (`crates/core/src/fixed.rs`), but any `f32`/`f64` transcendental that can reach a hashed path is a cross-platform hazard. Lands in `crates/core` as a `jmath`-style shim plus one line in `docs/HASH-LAW.md`.
   - **Kill:** if the value cannot reach the golden, do not port it.

4. **A generated boundary corpus checked against a checksum-pinned oracle.** `tools/test_rust_parity.py:20-70` builds a stable-seed (`0x484F52495A4F4E`) corpus of `i32`/`i64` edges, float/double bit patterns, packing widths and ±2^31 intervals; `tools/test_tick_parity.py:20-80` drives wraparound collision clusters in a deadline index, equal-key heads, map growth/shrink and time overflow; `tools/test_rust_parity.py:73-87` authenticates against the lock and bundled checksums, not a generated report.
   - Maps onto engine2040: one golden (seed 2040) plus tests. A deterministic generated corpus would widen integer tick/hash coverage before it feeds the proof crate (`crates/proof`).
   - **Kill:** the corpus must be fixed-seed deterministic, or it is a flake, not a proof.

## Also seen, not stolen

`docs/benchmark.md:15-33` — a statistical bar worth copying into the perf-bench skill: same machine, one `options.txt` for both games, same world/path, warm-up excluded, the same `stats()` code for both, repeats reported as mean with standard deviation and a 95 % CI (Student's t), runs on a machine doing other work marked and not published. `docs/parity-workflow.md:50` — a new or changed harness must pass three runs in a row or it is `FLAKY` and fails the gate.

## Not read

`docs/rust-parity.md` (106 KB), `docs/collision-geometry.md` (133 KB), the 280 other `tools/test_*_parity.py` bodies, `crates/horizon-world`.

DONE 4 steals, donor files read live 2026-10-18 (`docs/parity-workflow.md`, `docs/benchmark.md`, `docs/tick-state.md`, `docs/profiling.md`, `tools/test_rust_parity.py`, `tools/test_tick_parity.py`). Nothing built or run.
