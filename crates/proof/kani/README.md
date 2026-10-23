# HS23 Kani harnesses

Twenty proof harnesses over four integer kernels, five each:

| Kernel | File | What it proves |
| --- | --- | --- |
| Q16 math (`engine_core::fixed`) | `q16.rs` | checked multiply/divide, floor square root, lerp endpoints, scalar compare |
| Cut-plane quantization (`engine_combat::cut`) | `cut.rs` | envelope reject, travel bound, self contact, effective energy bound, odds range |
| Bond arithmetic (`engine_destruction::bonded`) | `bond.rs` | load covers mass, weakest link, capacity off switch, strict threshold, capacity replacement |
| Tape decode (`engine_proof::tape`) | `tape.rs` | size/magic/version/count gates, checkpoint roundtrip, one-tick encode/decode |

Every harness has a `Kernel:` and a `Bound:` line in its doc comment.

## Run

```text
cargo kani -p engine-proof --lib --default-unwind 64
```

`--lib` verifies only the proof library; `engine-destruction` is a normal
core-only dependency, so no test target or dev-dependency is needed.

## Tool status

Kani was not installed on the shared Windows PC on 2026-10-23
(`cargo kani --version` prints `no such command: 'kani'`). The harnesses are
authored and their properties are mirrored by the `hs23_*` tests in
`crates/proof/src/kani_kernels.rs`. `.github/workflows/kani.yml` installs
Kani 0.68.0 on `ubuntu-latest`, runs each kernel set by harness name, and
prints the passed count. The Kani accept stays open until that job passes.

The cut harnesses target the integer contact kernel, not the float CSG
`CutPlane`/`knife` path, which is outside the integer-kernel scope.
