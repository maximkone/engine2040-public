---
name: determinism-debug
description: "Use when a golden hash, replay checkpoint, tape or golden-matrix row diverges, or a replay, capture or perf run looks wrong: reproduce twice, bisect to the first divergent tick, read MATCH/DIVERGE/SKIP and cause=fma/libm/sort-tie, never re-pin the golden."
---

# Determinism debug

A hash divergence is evidence, not a verdict. Law: `docs/HASH-LAW.md`. Same complete input fixture gives the same hash.
Golden `GOLDEN_600=d57b5c8ae3096614` (seed 2040, 600 ticks, `crates/proof/src/lib.rs`). It moves only with an explicit receipt (section 5).
Diagnose sim vs extraction vs GPU. Never blame a driver by default.

## 1. Triage order for a wrong-looking run

Three suspects, in this order: replay first, capture path second, perf noise last.
1. Replay before pixels. Does the same fixture hash the same twice before you read any capture? Never give a look verdict on a diverged replay. Never compare pixels from two different trajectories.
2. Find the first divergent tick, not the last frame. A report that says "diverged at frame 600" when the split began at tick 40 is useless.
3. Only then the capture path (skill `test-and-proof`), then perf noise (skill `perf-bench`).
4. Change one variable per rerun (skill `debug-triage`).

## 2. Four phases

1. **Reproduce.** Run the same binary, config and fixture twice. Record the command, seed, build and config, the first divergent tick and the state. Not reproducible: gather more data, never guess.
2. **Scope.** Which tier broke? The legacy v1 golden is frozen. The arena v2 fixture has its own version and a non-aliasing seed expansion.
   - v1 seeds `2k` and `2k+1` alias (`Rng::new(seed | 1)`): the same trajectory, but the receipts differ because the raw seed is folded in. Receipt inequality proves nothing about trajectories. Assert trajectories directly.
3. **Bisect.** Compare checkpoints to find the first divergent tick. Then walk the hash scope IN list (below). Check suspects in the order of section 3.
4. **Fix once.** One root-cause change, a negative test that alters covered state, and two runs with the same hash. The same fingerprint twice: write a `docs/BLOCKERS.md` row with owner and repro and go on. A run never lands green on a failing hash.

Hash scope (HASH-LAW):
- IN: canonical entity and component state that affects future ticks, RNG state, timers, pending authoritative events, AI plan and blackboard state, script state, accepted external actions, asset and rules version references.
- OUT: GPU handles and pixels, render interpolation, UI selection and layout, wall-clock durations, model credentials, transport timing, telemetry.

## 3. Suspects, in order

1. `HashMap` iteration order in hashed state. Pointer addresses. Unspecified Rust hashing.
2. Platform float functions in authority code: `sin`, `cos`, `exp`, `powf` and friends. Only `+ - * /`, `sqrt`, comparisons and integer or `Fixed` math are allowed. `node tools/workspace-check.mjs` reports platform float math in sim code (a WARN today, report-only).
3. Fast-math flags. Thread-count-dependent reductions: merge job results by entity id, never by arrival order.
4. Rapier without `enhanced-determinism`, or with SIMD or parallel features (skill `physics-bodies`).
5. Wall clock or transport timing leaking into state. Job completion order deciding an outcome.
6. Init order or struct layout (tick 0 differs).
Cast and overflow rules: skill `rust-safety`.

## 4. The matrix: the same hash on every machine

Gate G6 "same hash". Compare tool: `tools/golden-matrix.mjs`. Workflow: `.github/workflows/golden-matrix.yml` (public mirror only).

| label | machine |
|---|---|
| `linux-x86_64` | ubuntu-latest, native. The base every other row is compared with |
| `windows-x86_64` | windows-latest, native (the players' PCs) |
| `linux-aarch64` | ubuntu-24.04-arm, native |
| `wasm32-wasip1` | same sources for WASI under wasmtime (32-bit usize, no threads, software libm) |
| `wasm32-wasip1-simd128` | the same with `RUSTFLAGS=-C target-feature=+simd128` (the `wide` crate under Rapier takes its SIMD paths; relaxed SIMD stays off) |

Each target records three things. The `compare` job checks all of them. It also runs the `engine-physics --features rapier` lib tests native against wasm (the same test names must pass).
- **tape**: `engine2040 --record out/golden.e2040 --verify` (release). Per tick two u64s: `legacy` (the frozen golden lane) and `state` (the full state hash).
- **corpus rows**: `cargo run -p engine-proof --bin matrix_line --release`. The first token is the host label. A runner that is not what its slot says fails.
- **ragdoll rows**: `cargo run -p engine-proof --features ragdoll --bin ragdoll_line --release`. A digest of every Rapier body transform for a fixed 300-tick fall. It claims Rapier `enhanced-determinism` matches across x86-64, aarch64 and wasm. The authority layer does not use Rapier.

Run it:
- Push a throwaway branch `golden-matrix-*` of the public mirror, or `workflow_dispatch` (the file must be on the mirror's main). To get the tree there see skill `github-check`, section "The whole suite: the mirror".
- Read the result with `node tools/public-export.mjs --ci`, or open the `compare` job log. Logs exist after the run ends.
- By hand on files you have: `node tools/golden-matrix.mjs a.e2040 b.e2040 [c.e2040 ...] [--rows a.txt b.txt ...] [--ragdoll a.txt b.txt ...]`. Each group needs two or more files and is compared with its own first file. Exit 0 all agree, 1 a divergence or a skipped pair, 2 usage or unreadable file. Tests: `node --test tools/golden-matrix.test.mjs`.
- Locally you can record a tape (`cargo run -p engine2040 --release -- --record out/golden.e2040 --verify`). Do not hold the cargo lock for long: prefer the runner.

Read the output:
```
golden-matrix: 5 tapes
<name> seed=2040 ticks=600 inputs=N last_state=...
MATCH <name> vs <first>
DIVERGE <name> vs <first>: first bad tick T (<lane> lane) cause=<c> - <note>
SKIP <name>: <why>
```
- MATCH on every line with golden `d57b5c8ae3096614`: the gate passes.
- `SKIP`: the two tapes are not comparable (format or fixture version, seed, tick count or input list differ). It exits 1. Fix the recorder, not the compare.
- `DIVERGE` names the first bad tick and the lane: `legacy` (the golden lane), `state` or `both`. Tick 0 means the start state differs: seed, init order or struct layout. Tick T means T-1 matched: bisect that tick's systems.
- `cause=` is a guess from the xor and difference of the two hashes:
  - `fma`: difference of 1 to 2, one or two bits. A fused multiply-add on one target only.
  - `libm`: small difference. A platform `sin`, `exp` or `powf`.
  - `sort-tie`: large difference. Equal keys sorted in a different order, or a `HashMap` walk.
  All three are banned in authority code.
- Corpus rows name the first bad block and case range. Ragdoll rows name the first tick and body, and how many bodies differ. A ragdoll-only divergence does not touch the golden (physfx stays cosmetic), but Rapier cannot become an authority engine on the web.
- `MISSING matrix/golden-<host>/<file>` means that recorder failed. The matrix is incomplete, not green.

Pitfalls:
- Record every target in release. A debug tape is not comparable evidence.
- Aarch64 or wasm only: look for fused multiply-add and libm first (`cause=`), then `HashMap` order.
- Nothing here proves pixels. Pixels have their own toleranced gates (`docs/VISUAL-TARGET.md`).

## 5. Golden receipt rule

- Never re-pin the golden to make a row match. A hash move needs a receipt, or a revert.
- The receipt names: fixture and version, the reason, old and new hash, and both run outputs. If you commit it, put the receipt in the scoped commit.
- Prove it unchanged with `cargo run -p engine2040 --release` twice (same hash) and `node tools/gh-check.mjs cargo test -p engine-proof --lib golden_hash_untouched`.
- New integrations get a new versioned fixture with a negative test that alters covered state. Do not stretch v1 coverage by name. Most workspace crates are not in `GOLDEN_600`.
- A missing GPU adapter is unsupported coverage, not a green parity result.
- Commands to prove a physics change: skill `physics-bodies`. Door runs: skill `door-and-headless-runbook`.
