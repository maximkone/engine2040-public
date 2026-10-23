# box3d-rust determinism oracle — results (row PX45)

Is the float Box3D ragdoll bit-identical across hosts, and what does a fully
integer solver cost? Two questions, one scene: the donor's own
`falling_ragdolls` determinism scene, stepped at a fixed 1/60 s with 4 substeps.

Read 2026-10-23. Nothing here is linked by a crate under `crates/`; the spike is
its own workspace.

## Pins

| what | source | pin |
|---|---|---|
| oracle engine | <https://github.com/larsbrubaker/box3d-rust> | `f2ca61e39b653d1b13945d13ebd15478d5fa4661`, MIT, vendored in `vendor/box3d-rust/` |
| cost, float | <https://github.com/erincatto/box3d> | `47d7f7cc7e091142c08d11dc7d2e493c5d34f536` |
| cost, fixed point | <https://github.com/mas-bandwidth/fixed3d> | `071a3f6de64d771ebe05ffccde3564fdb52fcdcd` |

The float baseline is the commit fixed3d's own README measures against, not
box3d HEAD: HEAD trims the human ragdoll from 14 bones to 12, so HEAD and
fixed3d would run different grids.

## Build lines

```
cargo build --release --manifest-path tools/oracle-box3d/Cargo.toml
cargo build --release --target wasm32-wasip1 --manifest-path tools/oracle-box3d/Cargo.toml
node tools/oracle-box3d/wasm-run.mjs tools/oracle-box3d/target/wasm32-wasip1/release/box3d-oracle.wasm --steps 120
pwsh -NoProfile -File tools/oracle-box3d/cost/run.ps1 -Processes 3 -Reps 4 -Steps 500
```

## Determinism: equal on both hosts

| run | tick 120 | tick 600 |
|---|---|---|
| Windows x64, msvc, run 1 | `0x5D3CD3FA` | `0x48DE6D57` |
| Windows x64, msvc, run 2 | `0x5D3CD3FA` | — |
| wasm32-wasip1, Node 24 WASI | `0x5D3CD3FA` | `0x48DE6D57` |

- 100 live bodies, `donor_settled=0xFF4C605B` at `sleep_step=291` on every run.
- Run 2 read run 1's dump with `--compare`: `no field differs: every body state
  matches` - all 1,300 values (100 bodies x 13 fields: position, rotation,
  linear and angular velocity).
- The dump path names the first differing body and field, so the next run that
  breaks reports *where*, not only that.

Verdict: the float engine is host-stable across a native and a wasm build. It is
a usable oracle for our hashed ragdoll.

## Cost: the integer solver is 4x, not the donor's 2.2x

500 steps, 4 reps, 3 processes, scalar (SIMD off), both engines built by the
same zig clang, same target and flags. Full log:
`docs/receipts/2026-10-23-box3d-oracle/cost-runs.txt`.

| engine | runs | cpu median ms | cpu min ms | wall median ms |
|---|---|---|---|---|
| float box3d | 12 | 93.75 | 78.12 | 94.68 |
| fixed3d (Q48.16) | 12 | 382.81 | 343.75 | 379.31 |

`factor_cpu_median = 4.08x`, `factor_cpu_min = 4.40x`,
`factor_wall_median = 4.01x`. fixed3d's README claims about 2.2x in scalar mode.
On this PC, same scene, the measured ratio is **4.08x**.

Scene identity is reported, not assumed: `awake_at_create=112` on both engines,
but `sleep_step` is **308 float vs 404 fixed**. The two solvers start the same
112 bodies and part company inside the first second, so the ratio is a *cost*
ratio between two engines doing the same work at the start, not a same-trajectory
comparison. The fixed-point solver is also not the same simulation, and nobody
should read its output as an equal-hash partner for the float one.

## Decision input

- Float Box3D is host-stable: keep it as the determinism oracle for our ragdoll
  corpora, one hashed reference per scene.
- A fully integer solver costs about 4x the float engine for the same scene on
  this hardware. That is a budget line for any "integer physics" plan, not a
  free port.

## Open

- The cost ratio is measured on one machine while other sessions share it
  (wall time is noisy; CPU time is the number to read).
- `sleep_step` differences mean the two engines cannot be compared tick for tick
  without a fixed-point port of the donor scene itself, which nobody has done.
