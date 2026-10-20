# OPT9 (compile cache spike): a warm sccache serves no lane in a fresh target dir — DROP (receipt)

Written 2026-10-20 on the Windows box, observed-only. OPT9 asks for one cold lane
build before/after sccache, kept only if the warm lane is 30% faster or more. The
answer is a DROP with numbers: the cache is healthy (it hits inside one target
dir) but it cannot serve a lane build in a *fresh* target dir, which is exactly
what a lane pool is, so nothing here justifies a global wrapper or a pool wrapper.
The tree's current state (opt-in per run, no `rustc-wrapper`) stays.

Tool: `node tools/compile-cache-spike.mjs --lane engine-physics --jobs 4`
(4 cells, each its own `CARGO_TARGET_DIR`; its own sccache server via
`SCCACHE_SERVER_PORT`, own `SCCACHE_DIR`, `SCCACHE_BASEDIRS` = the repo). Cells:
`cold` (rustc directly, incremental as configured = today's lane), `prime`
(sccache, empty cache), `warm` (sccache, cache filled by `prime`), `warm2`
(warm again in a fresh dir).

## The lane and the box

- Lane = the compile step of the affected gate for one crate:
  `cargo test -j 4 --locked -p engine-physics --all-targets --no-run`
  (`laneCommand` in `tools/affected.mjs` plus `--no-run`: the same line without
  running the tests; `engine-physics` depends on `engine-core` only).
- Box: Windows, 12 CPUs, 16 GB RAM, sccache 0.18.0, rust-lld. Not quiet: three
  python processes and other agents' cargo held 83-94% CPU during the day, so
  every second here is observed-only, not a baseline. An earlier probe of the
  same line read 434.5 s while the box was busiest and 22.6 s twice afterwards:
  that is the spread of this box, which is why the tool times the cells
  back-to-back and reports the warm-cell spread.

## Numbers (run `target/spike-opt9/20261020T105037/report.json`)

| cell | wall s | cacheable units asked | hits | misses | hit rate |
|---|---|---|---|---|---|
| cold (no wrapper) | 21.6 | - | - | - | - |
| prime (empty cache) | 14.4 | 2 | 0 | 2 | 0% |
| warm (filled cache) | 13.6 | 2 | 0 | 2 | 0% |
| warm2 (filled cache) | 16.8 | 2 | 0 | 2 | 0% |

- `warm` is 13.6 s vs `cold` 21.6 s: 37% on the clock, **but 0 of 2 cacheable
  units hit**, so that gap is not the cache. The two warm runs alone spread 23.5%
  (13.6 vs 16.8 s) on this box: anything under about a quarter of the wall time
  is inside the noise here.
- An earlier run with the box's shared cache server (`…T104528`) read the same:
  cold 22.6, prime 15.8, warm 12.6, warm2 11.3, 0 hits in every cache cell.
- Server counters for the run: 30 compile requests, 6 executed (2 per cache
  cell, all misses, 6 writes, 4.5 MB), 24 not cacheable: `crate-type` 15,
  `missing input` 6, `-` 3. So a 10-rustc-call lane exposes only 2 units to the
  cache at all (the rlib compiles); the test binaries are `crate-type` units
  sccache refuses.

## Why 0 hits: the target dir is in the key (observed), and basedirs did not bridge it

- Observed: `cargo build -p engine-core` in two *fresh* dirs, same dedicated
  server, `SCCACHE_BASEDIRS=C:\engine2040` (`target/spike-opt9/dbg2`): 2 misses,
  0 hits. Same command, same compiler, same source, only the target dir differs.
- Observed: a scratch crate in one *fixed* target dir, source touched between
  builds (`…\Temp\opencode\sccache-mech`): 1 hit out of the rebuild. So the cache
  itself works on this box; the fresh dir is what kills it.
- Inferred (not measured inside sccache): the rustc arguments carry the target
  path (`--out-dir`, `-L dependency=…`, dep-info), and the key hashes them;
  `SCCACHE_BASEDIRS` normalizes the repo root but not the per-cell subpath, so
  two pools still get two keys. This matches the repo's own 2026-10-01 number
  (`cargo check -p engine-arena` 20.2 s -> 1.1 s at 100% hits, `.cargo/config.toml`
  line 60), which was one target dir rebuilt, not two pools.

## Decision

- DROP for lane pools: a fresh `CARGO_TARGET_DIR` per lane gets 0% hits, so the
  ≥30% bar is not met by the cache. Keep sccache opt-in per command
  (`RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0`), keep the no-global-wrapper guard
  (`.cargo/config.test.mjs`), and do not bring back a pool wrapper.
- The case where it does pay is a clean rebuild or a branch switch in the *same*
  target dir, and CI (`check.yml` sets sccache plus a stable checkout path and
  rust-cache), which this row does not change.
- Not proven here: a bigger lane (the 0% hit rate is structural, so size cannot
  turn it into a win, but no big lane was timed); Linux runner hit rates; whether
  a single shared target dir across lanes (which the build lock forbids) would
  pay.

## Replay

```
node tools/compile-cache-spike.mjs --dry-run --lane engine-physics
node tools/compile-cache-spike.mjs --lane engine-physics --jobs 4
node --test tools/compile-cache-spike.test.mjs
```

Raw counters of the two mechanism checks and of the run are kept on disk (not in
git) at `target/spike-opt9/2026-10-20-mech.txt`, with the scratch crate copied to
`target/spike-opt9/mech-crate`.
