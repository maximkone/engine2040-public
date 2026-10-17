---
name: golden-matrix
description: "Use when you prove or debug that engine2040 gives the same hash on every machine: the five golden-matrix.yml targets, the tape, corpus and ragdoll rows, and how to read a MATCH, DIVERGE or SKIP line from tools/golden-matrix.mjs."
---

# Golden matrix

Finish-line gate G6 "same hash" (rows HS2, WEB1, WEB3). The authority simulation must give the same digests on every machine. Golden: seed 2040, 600 ticks, `d57b5c8ae3096614` (`docs/HASH-LAW.md`). Compare tool: `tools/golden-matrix.mjs`, workflow `.github/workflows/golden-matrix.yml` (public mirror only).

## The five targets
| label | machine |
|---|---|
| `linux-x86_64` | ubuntu-latest, native. It is the base every other row is compared with |
| `windows-x86_64` | windows-latest, native (the players' PCs) |
| `linux-aarch64` | ubuntu-24.04-arm, native |
| `wasm32-wasip1` | the same sources built for WASI and run under wasmtime (32-bit usize, no threads, software libm) |
| `wasm32-wasip1-simd128` | the same with `RUSTFLAGS=-C target-feature=+simd128` (the `wide` crate under Rapier takes its SIMD paths; relaxed SIMD stays off) |

Each target records three things, and the `compare` job checks all of them, plus the `engine-physics --features rapier` lib tests native against wasm (same test names must pass):
- **tape**: `engine2040 --record out/golden.e2040 --verify` (release). Per tick two u64s: `legacy` (the frozen golden lane) and `state` (the full state hash).
- **corpus rows**: `cargo run -p engine-proof --bin matrix_line --release`. First token is the host label; a runner that is not what its slot says fails.
- **ragdoll rows**: `cargo run -p engine-proof --features ragdoll --bin ragdoll_line --release`, a digest of every Rapier body transform for a fixed 300-tick fall (WEB3). This is the claim that Rapier `enhanced-determinism` matches across x86-64, aarch64 and wasm. The authority layer does not use Rapier (skill `two-physics-layers`).

## Run it
- Start: push a throwaway branch `golden-matrix-*` of the public mirror, or workflow_dispatch (the file must be on the mirror's main). Getting the tree there: skill `web-build-and-test`, section "Get your tree to the mirror".
- Read the result with `node tools/public-export.mjs --ci`, or open the `compare` job log. Read logs after the run ends.
- By hand on two files you have: `node tools/golden-matrix.mjs a.e2040 b.e2040 [c.e2040 ...] [--rows a.txt b.txt ...] [--ragdoll a.txt b.txt ...]`. Each group needs two or more files and is compared with its own first file. Exit 0 all agree, 1 a divergence or a skipped pair, 2 usage or unreadable file. Tests: `node --test tools/golden-matrix.test.mjs`.
- Locally you can record a tape: `cargo run -p engine2040 --release -- --record out/golden.e2040 --verify` (never in a loop that holds the cargo lock for long; prefer the runner).

## Read the output
```
golden-matrix: 5 tapes
<name> seed=2040 ticks=600 inputs=N last_state=2aca9338670aa0cb
MATCH <name> vs <first>
DIVERGE <name> vs <first>: first bad tick T (<lane> lane) cause=<c> - <note>
SKIP <name>: <why>
```
- MATCH on every line, golden `d57b5c8ae3096614`: gate passes.
- `SKIP`: the two tapes are not comparable (format or fixture version, seed, tick count or input list differ). It exits 1: fix the recorder, not the compare.
- `DIVERGE` names the first bad tick and the lane: `legacy` (the golden lane), `state`, or `both`. Tick 0 means the start state differs: seed, init order or struct layout. Tick T means T-1 matched: bisect that tick's systems.
- `cause=` is a guess from the xor of the two hashes: `fma` (difference of 1-2, one or two bits: a fused multiply-add on one target only), `libm` (small difference: a platform `sin`, `exp`, `powf`), `sort-tie` (large: equal keys sorted in a different order, or a `HashMap` walk). All three are banned in authority code (`docs/HASH-LAW.md`).
- Corpus rows name the first bad block and case range. Ragdoll rows name the first tick and body, and how many bodies differ on that tick. A ragdoll-only divergence does not touch the golden: physfx stays cosmetic, but Rapier cannot become an authority engine on the web.
- A missing artifact (`MISSING matrix/golden-<host>/<file>`) means that recorder failed; the matrix is incomplete, not green.

## Pitfalls
- Record every target in release (that is what ships and what `ci.yml` proves); a debug tape is not comparable evidence.
- Do not "re-pin" the golden to make a row match. A hash move needs a receipt (old and new hash, reason, both outputs) or a revert.
- Aarch64 or wasm only: look for fused multiply-add and libm first (`cause=`), then `HashMap` order.
- Nothing here proves pixels. Pixels have their own toleranced gates (`docs/VISUAL-TARGET.md`).
