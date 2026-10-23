# Receipt: box3d-rust determinism oracle (row PX45)

Date 2026-10-23. The full evidence is `tools/oracle-box3d/RESULTS.md`; this is
the short record of what ran on this PC and what it means.

## Ran

| leg | command | result |
|---|---|---|
| native, twice | `box3d-oracle --steps 120 --dump run1.txt`, then `--compare run1.txt` | `digest=0x5D3CD3FA` both, `no field differs` |
| native, longer | `--steps 600` | `digest=0x48DE6D57` |
| wasm32-wasip1 | `cargo build --release --target wasm32-wasip1`, then `wasm-run.mjs ... --steps 120`, `--steps 600` | `0x5D3CD3FA`, `0x48DE6D57` |
| integer cost | `cost/run.ps1 -Processes 3 -Reps 4 -Steps 500` | float `93.75` ms vs fixed `382.81` ms cpu median, `4.08x` |

The donor scene settles at `0xFF4C605B` on tick 291 in all four runs, 100 bodies.

## Means

- The float ragdoll is bit-identical native and on wasm. It can serve as the
  hashed determinism oracle we lack for bodies.
- A fully integer solver is 4.08x float on this PC, not the donor's 2.2x, and it
  diverges from the float trajectory at `sleep_step` 404 against 308.

## Did not run

- Nothing in `crates/` and no golden: this receipt moves no engine code and no
  hash. `d57b5c8ae3096614` is untouched.
- No fixed-point port of the scene, so no tick-for-tick comparison.
