---
name: perf-bench
description: "Use when you measure or optimize speed: flamegraph, samply, Gungraun, bench-compare, alloc_probe, paired runs, p50/p95/p99, perf CSV, frame or build time, or any speedup claim: profile first, one variable per change, noise stays open, no golden or threshold changes."
---

# Perf and bench

Measurement comes before optimization. Every speed claim starts with a profile, not a patch. Plans: `docs/plan/quality.md` (the OPT rows). Hash moved or a run looks wrong: `determinism-debug`. Loop and budget questions: `tick-and-runtime`.

## 1. Profile first

1. Pick one target: build time, frame or tick time, or memory and code size.
2. Use one machine, exclusively. A parallel build or a GPU test beside the run invalidates it.
3. Build profiles in the root `Cargo.toml`: `profiling` (release, thin LTO, line tables), `perf-test` (release, thin LTO, 16 codegen units), `ship` (release alias, fat LTO, one CGU), `release`. Dev stays honest: do not time a dev build.
4. CPU hotspots: `cargo flamegraph -p <crate>` or `samply` on the fixture (external tools; install if absent, no crate depends on them). Name the top 3 frames with sample counts, not guesses.
5. Tests and clippy go through `node tools/gh-check.mjs` (the machine is not the point). Timings run on the PC, idle.

## 2. Measure

Report these fields every time:

- hardware and driver, build profile, seed and input, warmup, sample count;
- distribution: p50, p95, p99 (and mean, max);
- allocations and peak memory;
- paths to the raw output.

How:

- **Paired runs.** Baseline and change on the same idle box, same fixture, same seed. Report the noise band beside the delta.
- **Allocation.** `engine_core::alloc_probe::measure` (calls and bytes per tick after warmup). Dev-dependency feature `test-util`. Never write a second copy of a core helper.
- **CPU regression.** Gungraun (iai-callgrind, instruction counts) beside wall-clock p50/p95/p99. Benches live in `crates/perf/benches/` (`instructions.rs`, `off_path.rs`). Callgrind is Linux-only, so `gungraun` is a Linux dev-dependency of `crates/perf`. The runner must match: `cargo install --locked --version 0.20.0 gungraun-runner`. Wall-clock alone never lands a row.
- **GPU.** A GPU claim needs the real GPU path and per-pass timestamps. No hardware or baseline: say unverified.
- **Perf harness.** `cargo run --release -p engine-perf --bin perf > target/perf-run.csv`. Header: `scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,cpu_workload_rate,final_hash`. Run lines are in `door-and-headless-runbook`. `node tools/bench-compare.mjs` times named workloads or shell commands (`--warmup N --runs N`, warmups discarded). Baselines live in `tools/perf-baseline.json`.
- **Box clock.** Record the box clock against a reference clock with each timing bench. On drift, re-pin or quarantine the number. Never gate a row on another box's threshold.

## 3. Change

- One variable per change. Re-profile on the same fixture. Report before and after pairs within noise.
- A borderline delta (inside run-to-run noise, or overlapping distributions) stays open. Never tick it green or red.
- Never weaken a threshold. Never delete a timing test. Never repin the golden.
- Determinism is never traded for speed. The same fixture hashes the same before and after (`GOLDEN_600=d57b5c8ae3096614`, see `docs/HASH-LAW.md` for the receipt rule).
- `engine-core` stays std-only. IO and GPU work stay outside the fixed tick.
- A regression claim from a loaded box, or a speedup with no baseline, is not a finding.
- A new binary, new flags and a busy GPU in one rerun is not one result. Change one thing per rerun.

## 4. Report

Bottleneck. Evidence paths. Before and after numbers with the noise band. Smallest next improvement. No source writes on a measurement-only row. Prove with `cargo check -p <crate>`, then `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>`, and `node tools/rust-quality.mjs` PASS.
