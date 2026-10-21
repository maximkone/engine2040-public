# Build CPU: fewer cores burned per iteration

Owner ask 2026-10-21: "optimize our cpu usage af and all rust and cargo too".
Status: **SPIKE ran, idea 1 LANDED and the row is ticked** (OPT45). `[profile.release]` is now
thin LTO + 16 codegen units; the fat-LTO one-unit shape moved to `[profile.ship]`
(the packaged artifact and the CI release-proof leg). Measured on the GitHub
runner, same tree overlay, same 22 crates compiled in both runs:

| Shape | Profile | Build wall (runner) | Golden hash |
|---|---|---|---|
| fat LTO, 1 unit | `--profile ship` | **58.84 s** (run 39576660515) | `d57b5c8ae3096614` PASS |
| thin LTO, 16 units | `--release` | **38.08 s** (run 39576029326) | `d57b5c8ae3096614` PASS |

-35% wall for the same work, and the hash does not move: the sim is a
deterministic fixed point and no fast-math came with the profile change.
The pre-change fat baseline on the same line was 1 m 05 s (run 39575327069).
Local numbers (this PC) stay as the noisy reference only: the fat arena release
build took **11 m 41 s** with ~9.7 min of it in the one-unit relink, on a box
that was 100% busy with other sessions' cargo.

Still open: nothing. The 300-frame arena run on the **thin** build keeps the
60 fps floor (`tools/product-bar.json`), measured 2026-10-22 on the reference
GPU (RTX 3050 6GB Laptop, driver `32.0.16.1062`, 1280x720): six runs of
`--scene arena --fighters 8 --frames 300 --exit`, `interval_ms p50` =
16.770 / 16.618 / 16.688 ms in a quiet batch (median 16.688 ms = **59.9 fps**)
and 16.639 / 16.719 / 17.111 ms in a batch on a box at 100 % CPU with 19
cargo/rustc running (median 16.719 ms = **59.8 fps**). `validation_errors=0`
and `fight_draws=40` on every leg. The binary is `target/release/arena.exe`
(20,216,320 bytes), so the `release` profile dir, and `release` is thin LTO
with 16 units; the fat-LTO `ship` profile has its own `target/ship` dir, so
the two shapes never shared a target tree. The floor was never lowered; it is
the same bar the fat build had to pass. OPT45 is ticked.

Ideas 2-8 below are untouched. Idea 4 (dependency debug info off) is the next
cheap lever and stays its own commit.

## Baseline (all measured in this repo)

- Dev build 347 s wall (warm, `-j 4`); `engine-render` is the critical path at
  294 s (frontend 99.7 s, codegen 194.3 s), then `engine-assets`, `engine-nav`,
  `engine-proof`, `engine-audio` at 150-170 s each (`docs/SMOOTH-BUILD.md`,
  2026-09-30).
- `rust-lld.exe` cold link 121 s -> 79 s vs `link.exe` (OPT4).
- `[profile.test] debug = 0`: -17% user CPU, -24% wall on engine-physics
  (`.cargo/config.toml`, 2026-10-11).
- sccache across lane pools: **DROP** 2026-10-20. A fresh target dir served 0 of
  2 cacheable units; only rlib units are cacheable (24 of 30 requests refused).
  It pays in one target dir (clean rebuild, branch switch) and on CI only.
- Local product loop (before the change): `tools/bins.mjs` built arena with
  `--release` = **fat LTO, 1 codegen unit**, every time the keeper found it
  stale (every 4 h), and `product-check` paid that relink on every arena source
  change. It is now thin LTO, 16 units.
- Artifacts: **1.19 GB** of PDBs in `target/debug`; largest 46.8 MB
  (`kam2play`), 41.1 MB (`kam1c`), 27.1 MB (`wasm_bindgen_macro`).
- Toolchain: stable 1.97.1 only (no nightly installed); sccache 0.18.0 present;
  cargo-nextest absent (OPT6 parked).

## Ideas, ranked by measured cost x fit

1. **Thin-LTO local product profile** - **LANDED (OPT45)**. `[profile.release]`
   is now `lto = "thin"`, `codegen-units = 16`; `[profile.ship]` keeps
   `lto = "fat"`, `codegen-units = 1` for the packaged artifact and the CI
   release-proof leg. The arena the keeper and frame tools run no longer pays
   the one-unit relink. The fps floor (60) has 5-8x measured headroom (300-480
   fps on the RTX 3050) and is not lowered. Source: Cargo profiles book.
2. **Parallel front-end** `-Z threads=n` (nightly): type checking, borrow
   checking and MIR optimization run in parallel; parsing, HIR lowering and macro
   expansion stay serial. Our engine-render frontend is 99.7 s of a 294 s crate,
   so this is the second-biggest single lever after splitting the crate.
   Source: rustc-dev-guide "Parallel compilation" (tracking issue 113349).
3. **Cranelift dev backend** (`-Z codegen-backend=cranelift`, nightly component):
   much faster codegen for dev/test builds, slower generated code. Good for the
   inner loop, not for anything measured. Source: rust-lang/rustc_codegen_cranelift.
4. **Dependency debug info off in dev/test** (`[profile.dev.package."*"]
   debug = false`): the 1.19 GB PDB set is mostly dependencies; no runtime
   effect, shorter links. One-time dep rebuild, same quiet-box caveat.
5. **Split `engine-render`** (OPT11): the measured critical path; structural, so
   it beats every flag. Owned by the render session.
6. **cargo-nextest** (OPT6): wall time and per-test overhead, not CPU; needs the
   tool installed.
7. **sccache**: already measured DROP for fresh dirs; keep it opt-in per command
   (`RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0`), never a global wrapper.
8. **Theory**: Mokhov et al., *Build Systems a la Carte* (ICFP 2018) - scheduler
   and rebuilder taxonomy; our actionable parts are critical-path scheduling and
   content-addressed caching (both already present). Smits, Konat & Visser,
   *Constructing Hybrid Incremental Compilers...* (arXiv 2002.06183) and the
   Bazel-CI study (arXiv 2405.00796) support one cached build graph over many
   lane pools; our sccache measurement is the local counterexample (keys differ
   per target dir), so the fix is a stable checkout path or a shared cache, not a
   wrapper.

## Spike: ran 2026-10-21 on the GitHub runner

- Fixture: the same line in both runs, `cargo test -p engine2040 --release
  --test proof_cli default_and_max_seed` (22 crates compiled from scratch in
  each run), the only difference being the profile shape.
- Metric: the runner's own "Finished `release` profile [optimized] target(s) in
  Xs" line, plus the golden hash the test asserts.
- Result: fat LTO + 1 unit = **58.84 s** (`--profile ship`, run 39576660515),
  thin LTO + 16 units = **38.08 s** (`--release`, run 39576029326). Both PASS
  with `"final_hash":"d57b5c8ae3096614"` (`tests/proof_cli.rs:55`).
- The pre-change fat baseline on the same line was 1 m 05 s (run 39575327069).
- Patch landed instead of the reverted `[profile.product]`: `[profile.release]`
  is thin LTO + 16 units, `[profile.ship]` keeps fat LTO + 1 unit,
  `tools/ship.mjs` builds the packaged artifact with `--profile ship`,
  `tools/proof.mjs` builds the CI golden leg in the same shipped shape, and
  the CI release-proof leg builds `--profile ship`.
- Still open: the 300-frame arena run on the thin binary and the 60 fps floor
  (`tools/product-bar.json`). The floor is unchanged and not lowered.

## Idea 1 paid a second bill: one shape, not two (OPT46)

The `perf-test` profile existed only because `release` used to be fat LTO with
one codegen unit. After the change above, `perf-test` (it inherited `release`
and set the same LTO and codegen units) and `release` are the same shape, so
the local golden was compiling the whole tree a second time in a second target
dir. The profile is gone and the local golden builds on `release`; CI keeps
`ship`. One shape per job, one tree per shape.

## Idea 4 measured on this PC: dependency debug info (OPT39)

The dev profile already builds dependencies with line tables only
(`.cargo/config.toml`), and the test profile builds them with no debug info
at all, so idea 4 only touches `cargo build` / `cargo run` in the dev profile.
Cargo's own override rule was measured on this PC (`cargo check -v` in a fresh
target dir): `[profile.dev.package."*"]` does **not** match workspace members,
so `debug = false` there takes the debug info off third-party crates only and
our own crates keep their line tables.

## Sources (read live 2026-10-21)

- https://doc.rust-lang.org/cargo/reference/profiles.html
- https://rustc-dev-guide.rust-lang.org/parallel-rustc.html
- https://github.com/rust-lang/rustc_codegen_cranelift
- https://github.com/mozilla/sccache
- https://nnethercote.github.io/perf-book/build-configuration.html
- Mokhov, Mitchell, Peyton Jones, *Build Systems a la Carte*, ICFP 2018 (Microsoft Research PDF)
- arXiv 2002.06183v1 (Smits, Konat, Visser 2020)
- arXiv 2405.00796v1 (Bazel CI study, 2024)
