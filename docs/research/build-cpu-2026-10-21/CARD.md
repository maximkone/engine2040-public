# Build CPU: fewer cores burned per iteration

Owner ask 2026-10-21: "optimize our cpu usage af and all rust and cargo too".
Status: **CARD**, spike not run. The box sat at 99-100% CPU with 17-23 foreign
`cargo`/`rustc` processes through every window, and the one probe (a new thin-LTO
profile's cold `cargo check`) did not finish in 30 minutes, so it was reverted.
No speedup is claimed here; the numbers below are measured baselines, not wins.

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
- Local product loop: `tools/bins.mjs` builds arena with `--release` = **fat LTO,
  1 codegen unit**, every time the keeper finds it stale (every 4 h), and
  `product-check` pays that relink on every arena source change.
- Artifacts: **1.19 GB** of PDBs in `target/debug`; largest 46.8 MB
  (`kam2play`), 41.1 MB (`kam1c`), 27.1 MB (`wasm_bindgen_macro`).
- Toolchain: stable 1.97.1 only (no nightly installed); sccache 0.18.0 present;
  cargo-nextest absent (OPT6 parked).

## Ideas, ranked by measured cost x fit

1. **Thin-LTO local product profile** (`[profile.product]`, inherits release,
   `lto = "thin"`, `codegen-units = 16`) for the arena the keeper and frame tools
   run. Removes the fat-LTO single-unit relink from the recurring loop while
   `ship` keeps fat LTO for the packaged artifact. The fps floor (60) has 5-8x
   measured headroom (300-480 fps on the RTX 3050). One-time cost: the whole
   dependency tree is rebuilt for the new profile - the exact reason the probe
   could not finish on a loaded box. Source: Cargo profiles book.
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

## Spike (quiet window; one experiment)

- Fixture: `cargo build -p engine-game --features app --bin arena` under
  `--release` (fat LTO, 1 unit) vs `--profile product` (thin, 16 units), each
  after `cargo clean -p engine-game`.
- Metric: wall and rustc user CPU of the relink; accept if the product relink is
  >= 30% cheaper, and a 300-frame arena run on the same box keeps every fps
  floor (60) and the golden hash unchanged.
- Kill: cold product build costs more than it saves over 10 rebuilds, or the run
  drops under 120 fps on the RTX 3050.
- Patch to apply when it runs (reverted 2026-10-21, box too loaded):
  `[profile.product]` in Cargo.toml; `dir: "product"` + `--profile product` on
  the arena row of `tools/bins.mjs`; `target/product/arena.exe` in
  `tools/product-check.mjs`, `tools/arena-shot.mjs` (use the bin row's build
  line) and `tools/arena-net.mjs` (default exe + message + test pin).

## Sources (read live 2026-10-21)

- https://doc.rust-lang.org/cargo/reference/profiles.html
- https://rustc-dev-guide.rust-lang.org/parallel-rustc.html
- https://github.com/rust-lang/rustc_codegen_cranelift
- https://github.com/mozilla/sccache
- https://nnethercote.github.io/perf-book/build-configuration.html
- Mokhov, Mitchell, Peyton Jones, *Build Systems a la Carte*, ICFP 2018 (Microsoft Research PDF)
- arXiv 2002.06183v1 (Smits, Konat, Visser 2020)
- arXiv 2405.00796v1 (Bazel CI study, 2024)
