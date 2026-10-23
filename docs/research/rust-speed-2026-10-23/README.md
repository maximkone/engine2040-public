# Rust speed, wave 2026-10-23

Scope: one question, in two halves — how do we make this workspace **build** faster,
and how do we make the **tick and the shipped binary** faster, in 2026, as one solo
dev on one Windows PC. Nothing here changes code: every item ends in a command, a
diff shape, or a rejection with the number that kills it.

Method: every repo number below was read from a file at HEAD while this card was
written (`3925f1fe5`, re-checked at `98a3bd079` at commit time — several sessions
share this tree) or from a tool on this PC that printed it
(`llvm-profdata --version`, `rustc -V`, `Get-Command`). Every outside
number was fetched this session at the URL next to it. The toolchain on this PC is
stable **1.97.1** (2026-07-14); no nightly is installed, and no item below asks for one.

Number labels used everywhere: **measured** = read from a file, a run, or a tool on
this PC; **donor claim** = the donor's own sentence or table, no number of ours;
**estimate** = ours, with the reasoning named; **arithmetic** = plain multiplication
or division of measured numbers, not a benchmark.

## The one-line answer

The build lever that is left is structural, not a flag: `engine-render` is **294 s of
the 347 s** dev build (**measured**) and every flag lever in the list below is
already on, already measured to lose, or blocked on a nightly toolchain we do not
run. The runtime lever that is left is **PGO on the `ship` profile**, and it is
cheaper than the folklore says: `llvm-profdata` is already on this PC inside the
installed `llvm-tools` component (**measured**). The web lever that is left is
**SIMD128**, whose only real risk — "does the sim still produce the same tape?" —
is already green in CI at `wasm32-wasip1-simd128` (`docs/HASH-LAW.md:162`, MATCH).
All three are cheap to test and none is cheap to guess, so the first thing to install
is a sampler: item 1 in the ranked table is `samply`, because every other item is only
as good as the profile behind it.

## What we already hold (measured)

| number | value | where |
|---|---|---|
| dev build, warm, `-j 4` | 347 s wall; `engine-render` 294 s (frontend 99.7 s, codegen 194.3 s) | `docs/SMOOTH-BUILD.md:151` |
| cold link, `link.exe` → `rust-lld.exe` | 121 s → 79 s | `.cargo/config.toml:52-58` |
| runner build, fat LTO/1 unit vs thin/16 | 58.84 s vs 38.08 s, hash unchanged | `docs/research/build-cpu-2026-10-21/CARD.md` |
| sccache, one target dir, 100 % hits | 20.2 s → 1.1 s | `.cargo/config.toml:67-70` |
| sccache, fresh target dir | 0 of 2 cacheable units served | same card |
| `target/debug/incremental` | 701 MB of a 943.6 MB `target/` | `Cargo.toml:276-278` |
| workspace tests in the default gate | 3,216 | `Cargo.toml:250-252` |
| per-package `opt-level = 2` lines | 13, on top of deps at 3 | `Cargo.toml:288-316` |
| arena frame, 8 fighters, thin release | `interval_ms` p50 16.688 ms = 59.9 fps | build-cpu card, RTX 3050 |
| arena per-tick heap allocs | 0 after warmup | `docs/plan/quality.md:67` (PF1) |
| `dist/web` wasm | 4,545,212 B of a 4,700,000 B budget (3.3 % headroom) | `dist/web/build.json`, `tools/web-budget.json` |
| `dist/web` total | 7,264,802 B of an 8,388,608 B limit | same |
| page frame CPU, `opt-level` 3 vs `z` | 2.64 ms vs 5.95 ms (sim 2.31 vs 5.56) | `Cargo.toml:347-356` |
| tools present on this PC | `sccache` only; no `samply`, `nextest`, `wasm-opt`, `flamegraph`, `llvm-bolt`, `mold` | `Get-Command` |
| `llvm-profdata` | **present**: `…\1.97.1-…\lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-profdata.exe`, LLVM 22.1.6 | ran `--version` |

## 1. Build time

### 1.1 PGO on the `ship` profile — the one feedback lever Windows has left

- **What.** rustc's own PGO: instrument, run the arena tape, merge with
  `llvm-profdata`, rebuild with `-Cprofile-use`. `ship` is our fat-LTO/1-unit shape,
  so the profile reaches every cross-crate inlining decision, which is where a
  layout hint is worth the most.
- **URL.** https://doc.rust-lang.org/rustc/profile-guided-optimization.html — the
  four-step workflow and the `--target` trick that keeps `RUSTFLAGS` off build scripts.
  https://github.com/Kobzol/cargo-pgo for the wrapper (`cargo pgo instrument` /
  `optimize`), if the raw flags get tedious.
- **Gain.** **No trustworthy number exists.** The rustc book and `cargo-pgo`'s README
  both give the workflow and neither gives a percentage. Our **estimate** is 5-15 %
  frame CPU on a hot integer tick, and the honest status is "unmeasured until BSP1
  runs". This is exactly why it is a row and not a decision.
- **Risk.** Two costs, both real: (a) it is a *second* full fat-LTO build per cycle,
  which on this PC is minutes, not seconds; (b) a profile gathered from one arena run
  is a narrow workload — `cargo-pgo` warns that a different workload can get *slower*.
  Mitigation: profile the 600-frame seeded tape (the same tape the golden proof uses)
  and never ship a PGO build that fails the golden.
- **Do** (PowerShell 7; note `--target`, which is what keeps `.profraw` out of build
  scripts and moves the output under `target\x86_64-pc-windows-msvc\`):

```powershell
$prof = "$env:USERPROFILE\.rustup\toolchains\1.97.1-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin\llvm-profdata.exe"
$pgo  = Join-Path $env:TEMP 'e2040-pgo'
Remove-Item -Recurse -Force $pgo -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $pgo | Out-Null
$env:RUSTFLAGS = "-Cprofile-generate=$pgo"
cargo build --profile ship --target x86_64-pc-windows-msvc -p engine-game --bin arena
.\target\x86_64-pc-windows-msvc\ship\arena.exe --scene arena --fighters 8 --frames 600 --exit
& $prof merge -o "$pgo\merged.profdata" $pgo
$env:RUSTFLAGS = "-Cprofile-use=$pgo\merged.profdata -Cllvm-args=-pgo-warn-missing-function"
cargo build --profile ship --target x86_64-pc-windows-msvc -p engine-game --bin arena
```

Accept: `interval_ms` p50 improves on the same tape, three runs each arm, and the
golden still prints `d57b5c8ae3096614`.

### 1.2 BOLT — rejected for the shipped binary

- **What.** Post-link basic-block and function layout from a sampling profile.
- **URL.** https://github.com/llvm/llvm-project/blob/main/bolt/README.md — "BOLT
  operates on X86-64 and AArch64 **ELF** binaries."
- **Why rejected.** Our shipped artifact is PE/COFF on Windows. There is no path from
  a `.exe` to `llvm-bolt`, so this is not a tuning question. It would only apply to
  the Linux legs of the golden matrix, where build time is not a problem.
- **Reopens if** we ever ship a Linux build as a product.

### 1.3 Cranelift backend — rejected for now

- **What.** `-Zcodegen-backend=cranelift`, much faster codegen, slower generated code.
- **URL.** https://github.com/rust-lang/rustc_codegen_cranelift
- **Why rejected.** Three measured-in-docs blockers, in order of severity:
  "Unwinding on panics ([experimental and not supported on Windows and macOS])",
  `std::arch` is "partially supported" (our `wide` dependency compiles its lanes
  through `std::arch` intrinsics, so this is a correctness risk, not a speed one),
  and it ships in **nightly** only. A dev-loop backend that cannot unwind on our own
  OS will turn one panic into a confusing abort.
- **Risk if we did it.** A second toolchain in the loop, and the codegen difference
  means the dev binary is no longer the shape we ship.
- **Reopens if** the Windows unwinding issue closes and the component lands on stable.
  No compile-time number is quoted here on purpose: the only honest one is a timed
  clean build on our tree, which needs nightly installed first.

### 1.4 `-Zbuild-std` / `build-std-features` — rejected for now

- **What.** Rebuild `core`/`std` from source so they get our flags (e.g. `+simd128`
  on wasm, or `panic_immediate_abort`).
- **URL.** https://doc.rust-lang.org/cargo/reference/unstable.html#build-std
- **Why rejected.** Nightly only, and it adds a std build to every cold cache. Our
  std is the prebuilt target std from rustup (`rustup component list --installed`
  shows `rust-std-x86_64-pc-windows-msvc` and both wasm targets, **measured**), so
  today we buy no std-level speedup at all — and need none: the page's cost is in our
  sim, not in `core`.
- **Reopens if** the wasm module becomes budget-bound and a std rebuilt with
  `+simd128` (or `-Zbuild-std-features=panic_immediate_abort`) is the only way under.

### 1.5 Linkers — already decided on both platforms

- **Windows (measured, ours):** `rust-lld.exe` cold link 121 s → 79 s
  (`.cargo/config.toml:52-58`). Already in place; no further lever.
- **Linux (donor claim):** mold "links **4.9x** faster than LLVM lld … at the median"
  in the August 2026 benchmark — https://github.com/rui314/mold (README, nine large
  programs, Threadripper 7980X and M1 Ultra). Our CI already passes
  `-C link-arg=-fuse-ld=mold` on ubuntu, as `.cargo/config.toml:62-65` records; the
  flag itself lives in `.github/workflows/ci.yml`.
- **Why nothing to do.** mold has no Windows support (`docs/plan/build-cpu.md:143`),
  and the remaining link cost is a *count* of links, not their speed — which is the
  codegen-units knob below, not a linker knob.

### 1.6 sccache — keep it opt-in, do not make it a wrapper

- **What.** Compiler cache. We already have it, and already measured both answers.
- **URL.** https://github.com/mozilla/sccache/blob/main/docs/Local.md
- **Measured (ours).** 20.2 s → 1.1 s in one target dir at 100 % hits; **0 of 2**
  cacheable units across fresh target dirs (build-cpu card). The doc explains the
  second number: `hash_working_directory` defaults to `true`, so a different directory
  is a different key, and the local store is a single server (default 10 GiB,
  `SCCACHE_DIR`, `SCCACHE_CACHE_SIZE`).
- **Do.** Nothing. CI already uses the GHA backend with a 5 GB cache
  (`.github/workflows/check.yml`). Keep the per-command opt-in
  (`RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0`) and never a global wrapper.

### 1.7 `CARGO_INCREMENTAL` — leave it on for dev, off for release

- **Measured.** Dev `/ incremental` is 701 MB of a 943.6 MB `target/`
  (`Cargo.toml:276-278`). Release has it off by default and CI sets
  `CARGO_INCREMENTAL=0` for cache hits.
- **Do.** Nothing. A 701 MB cache that keeps a sim-core edit rebuild at 15 s
  (`Cargo.toml:250-252`, **measured**) is a fair trade on one PC; the disk is the
  price, and stale-tree cleanup is a different problem.

### 1.8 Workspace splitting — the biggest lever, and it is already owned

- **Measured.** `engine-render` is 294 s of a 347 s dev build, with 99.7 s frontend
  and 194.3 s codegen (`docs/SMOOTH-BUILD.md:151`). No compiler flag moves 85 % of a
  build that sits in one crate.
- **Why not a new row.** `docs/plan/build-cpu.md:151` and the build-cpu card already
  hand this to **OPT11** ("Split `engine-render`"). Proposing `BSP1` for it would open
  a second row for the same work. What is missing is a measured before/after, not a
  decision.
- **First experiment** (an hour, no code): `cargo build -p engine-game --timings`
  and read the `engine-render` bar against the 294 s figure; if the frontend is still
  ~100 s, the split boundary should follow the module boundary that carries the
  derive-heavy frontend, not the file count.

### 1.9 nextest vs libtest — a real speedup with a real caveat

- **What.** Process-per-test runner.
- **URL.** https://nexte.st/docs/benchmarks/ — **donor claim**: 1.37x (penumbra) to
  3.38x (crucible) faster test runs, on an AMD 7950X / Linux, **excluding build time**,
  and with doctests left out because nextest does not run them.
- **Our arithmetic.** Our default gate is 3,216 tests plus doctests
  (`Cargo.toml:250-252`). So a nextest leg would be faster on the 3,216 and would
  need a **second** `cargo test --doc` leg to keep doctests covered — which is exactly
  the shape that makes "just use nextest" a half-answer.
- **Risk.** `OPT6` is parked for one reason only: the tool is not installed
  (**measured**: `cargo-nextest` missing). A missing tool is not a technical blocker.
- **Do.** `cargo install --locked cargo-nextest`, then time
  `cargo nextest run --workspace` against `cargo test --workspace` on the same warm
  tree, three runs each, and report both the run time and the doctest leg.

### 1.10 `#[optimize]` and per-package `opt-level` — one spent, one new

- **Spent (measured).** The 13 `opt-level = 2` overrides plus deps at 3 were measured
  2026-10-03: `-p engine-combat` 0.94 → 0.35 s, touch-to-game 5.22 → 2.48 s
  (`Cargo.toml:255-264`). Nothing left to win there.
- **New.** `#[optimize(size | speed | none)]` was **stabilized** by rust PR #157273,
  "Stabilize `optimize` attribute", merged **2026-10-08** —
  https://github.com/rust-lang/rust/pull/157273. That is *newer than our 1.97.1*
  (2026-07-14, **measured**), so today it does not compile here.
- **Gain.** Two different wins, and they must not be confused: `#[optimize(size)]` on
  cold paths (formatting, error text, `serde` shims) buys code size and I-cache, not
  wall clock; `#[optimize(speed)]` on a hot item buys codegen that the crate-level
  `opt-level` already gives. The honest expectation is a **small** win on the first
  and a **wash** on the second.
- **Risk.** It is an attribute on *items*, so a wrong placement is invisible until
  measured; and a toolchain bump is a tree-wide event here (`rust-version = "1.97.1"`).
- **Do.** When the workspace toolchain moves past 1.97.1, put `#[optimize(size)]` on
  one cold module, measure the arena `interval_ms` p50 and the binary size, and keep
  it only if neither regresses.

### 1.11 Codegen-unit sweep on the shipped shape — the cheap unknown

- **Measured.** `release` is thin LTO + 16 units, `ship` is fat + 1 (`Cargo.toml:242-246`,
  `342-345`). The 16 was chosen when `release` replaced the fat local build; 8, 32 and
  64 were never timed on this tree.
- **Gain.** **Estimate**: link and codegen wall time, no runtime change — codegen units
  only change how much cross-unit inlining the backend can do, and thin LTO already
  sees across them.
- **Risk.** Lower CGU counts buy runtime and cost build time (the opposite of what we
  want locally), and a *shipped* shape must keep the 60 fps floor
  (`tools/product-bar.json`) — the floor is not lowered.
- **Do.** Three `cargo build --release -p engine-game --bin arena` runs at
  `CARGO_PROFILE_RELEASE_CODEGEN_UNITS` 8/16/32, same warm tree, plus one 300-frame
  `interval_ms` p50 each. Keep 16 unless a number says otherwise.

## 2. Runtime

### 2.1 Allocators (mimalloc / jemalloc / snmalloc) — rejected for the tick

- **What.** Swap the global allocator. We already carry `mimalloc 0.1.52` in the lock
  for the bench harness only (`Cargo.toml:209-212`, PF18).
- **URL.** https://remadewithrust.com/learn/game-memory-allocator-for-developers/ —
  **donor claim**, a recorded Endless Sky allocation stream replayed through four
  allocators: start-up allocator time **171 → 104 ms**, worst-1 % battle frames as
  allocator cost **24 % and 31 % lower**, but *combined* battle cost (allocator time
  plus the page faults of first writes) **0.90x and 1.07x — the interval includes
  1.0**. mimalloc's battle allocator time read **twice** glibc's with **6-11x** more
  calls over 100 µs; jemalloc's combined 99th percentile was **69 % worse**.
- **Our reason to refuse.** PF1 already holds **0 per-tick heap allocations** after
  warmup (`docs/plan/quality.md:67`, **measured**). An allocator swap optimises a call
  site we do not make. The donor also states Windows clients cannot be recorded yet,
  so their Windows story is untested.
- **Reopens if** PF1 regresses on a real path, or a frame profile shows allocator
  frames above noise.

### 2.2 `std::simd` vs `wide` vs `pulp`

- **`std::simd` is still nightly.** https://doc.rust-lang.org/std/simd/ carries the
  `portable_simd` feature gate; the tracking issue is rust-lang/rust#86656. Nothing
  in this workspace can use it on 1.97.1.
- **`wide` is already here** (`wide 1.7.1` in `Cargo.lock`, **measured**) and is used
  for the four-joint SoA lerps in `crates/anim` (`Cargo.toml:218-220`). Its lanes lower
  to `std::arch` intrinsics where the target has them.
- **Verdict.** Extending `wide` to one more hot loop is **PF5's** work, already open
  (`docs/plan/quality.md:161`). No new row. The rule that makes it safe is already in
  the repo: every SIMD change is A/B-measured with identical outputs, and the scalar
  path stays.

### 2.3 `repr(C)` / SoA — already the shape where it was measured

- **What.** Data layout: struct-of-arrays for per-entity hot data, `repr(C)` only at
  FFI/GPU boundaries.
- **Measured.** `crates/anim` already samples SoA (`Cargo.toml:218-220`); the tick
  carries integer state in fixed-point, not float structs.
- **Verdict.** No experiment proposed on layout alone. A layout change without a
  profile is a guess, and section 3 is the thing that turns it into a measurement.

### 2.4 `likely` / `unlikely` — unstable; two stable cousins are usable

- **URL.** https://doc.rust-lang.org/std/hint/ (docs for 1.99.1, read today):
  `likely` and `unlikely` are marked **Experimental**; `cold_path` and
  `select_unpredictable` are stable.
- **What that means for us.** The branch hint on the tick's hot `if` is not available
  on stable. What *is* available is `cold_path()` — put it in the arm that panics or
  bails, not in the arm that runs — and `select_unpredictable` for the one
  data-dependent choice per tick that a predictor cannot learn.
- **Gain.** **Estimate**: small and local (layout of a few blocks). It is worth one
  measured hunk precisely because it is nearly free and needs no dependency.
- **Risk.** A hint in the wrong place is a pessimisation the compiler will happily
  keep.
- **Do.** One hunk, one A/B on the 600-frame tape, keep only on a p50 win at an
  identical hash.

### 2.5 `-C target-cpu=native` — not for the binary we ship

- **What.** Let LLVM use this CPU's full feature set (AVX2/FMA and friends).
- **URL.** https://doc.rust-lang.org/rustc/codegen-options/index.html (`target-cpu`).
- **Why refused for ship.** Our hash law spans x86-64, aarch64 and two wasm legs
  (`docs/HASH-LAW.md:162`, MATCH). FMA availability changes rounding — the workspace
  already encodes this: `suboptimal_flops` is deliberately `allow` with the reason
  "`mul_add` changes rounding with FMA availability, which breaks cross-platform
  simulation hashes" (`Cargo.toml:146-148`). A native build can move the golden.
- **What it is still good for.** A **ceiling measurement**: one local build with
  `-C target-cpu=native` on the arena tells us how much the generic x86-64 baseline
  costs. If the answer is under ~3 % on `interval_ms` p50, we stop thinking about
  per-CPU shipping entirely.
- **Do.** One throwaway build, one A/B, delete the flags, write the number down.

### 2.6 Devirtualisation — the stable trap, and the unstable one

- **Stable traps.** `dyn` dispatch in the tick costs an indirect call the predictor
  may miss; the fix is a `match` on an enum or a generic monomorphisation at the call
  site, not a compiler flag.
- **The unstable flag is a trap of its own.** `-Zvirtual-function-elimination` is
  unstable, and the unstable book documents that it can remove vtable entries that
  are still reachable — a miscompile, not a tune-up.
  https://doc.rust-lang.org/unstable-book/compiler-flags/virtual-function-elimination.html
- **Do.** Count the `dyn` sites on the tick path (one grep), switch at most the
  hottest one to an enum, and A/B it. If the count is zero, this section is closed
  with a number instead of an opinion.

### 2.7 Integer math and `overflow-checks` — on, and the cost is unmeasured

- **Measured.** `overflow-checks = true` in `[profile.release]` (`Cargo.toml:246`) and
  inherited by `web`, so a wasm overflow panics where the native build panics.
- **Why it stays.** The sim is integer fixed-point; an overflow that silently wraps is
  a hash divergence that shows up 300 ticks later. The check is the receipt.
- **What we do not know.** What the checks cost per tick. Nobody has A/B'd it, and the
  answer would only be used to *argue about* turning them off — which we would not do.
  Not a row; named in "Still unverified".

## 3. Profiling a wgpu + Rapier tick, solo, on Windows

The order below is by "time to first answer", not by power.

### 3.1 samply — the fit (one install, 10 seconds of recording)

- **URL.** https://github.com/mstange/samply — "samply works on macOS, Linux, and
  Windows", sampling "at some sampling interval (the default **1000Hz**, i.e. 1ms)",
  and "On macOS and Windows, both on- and off-cpu samples are collected". UI is
  profiler.firefox.com, and the Microsoft Symbol Server is preconfigured on Windows.
- **Why it fits our tick.** Our frame is `wgpu` submit + present (blocking) wrapping a
  60 Hz integer tick. An on-CPU-only sampler shows the tick and hides the wait; samply
  shows the off-CPU stack too, so "we are at 16.7 ms because present blocks" and "we
  are at 16.7 ms because the tick is slow" become two different pictures.
- **Cost.** `cargo install --locked samply` (not installed, **measured**), and the
  binary must be release-shaped with debug info — we already have that profile:
  `[profile.profiling]` inherits release, `lto = "thin"`, `codegen-units = 1`,
  `debug = "line-tables-only"` (`Cargo.toml:322-326`).
- **Do:**

```powershell
cargo install --locked samply
cargo build --profile profiling -p engine-game --bin arena
samply record -- .\target\profiling\arena.exe --scene arena --fighters 8 --frames 600 --exit
```

  At 60 Hz, 600 frames is ~10 s, so ~10,000 samples at 1 kHz — enough to rank
  functions, not enough to trust a 1 % difference. Read the frame track first, then
  the call tree.

### 3.2 puffin and `perf --trace` — zero install, already in the tree

- **URL.** https://github.com/EmbarkStudios/puffin (scope macros), already a dependency
  (`puffin 0.20.0` in `Cargo.lock`, **measured**).
- **What we have.** `crates/perf` writes one Chrome Trace Event JSON with the tick
  spans, the GPU passes and the frame marks — `cargo run --release -p engine-perf --
  --trace arena600.json 600` (`crates/perf/src/main.rs:11-13`) — and `--capture`
  prints the frame-capture report. It is presentation-only and never hashed.
- **Verdict.** This is the first thing to run after any change, because it needs no
  install and no code change. It is **instrumented**, not sampled, so it answers "which
  scope is wide" but not "where inside this scope".

### 3.3 Tracy — only when a GPU pass must be correlated with the tick

- **URL.** https://github.com/wolfpld/tracy — "GPU (All major graphics/compute APIs:
  OpenGL, Vulkan, Direct3D 11/12, Metal, OpenCL, CUDA, WebGPU)", plus memory and locks;
  Rust bindings via https://github.com/nagisa/rust_tracy_client.
- **Verdict.** It is the only tool here that puts a GPU zone on the same timeline as a
  CPU scope. That is worth its cost **only** when we are chasing a specific pass
  (shadow cascade, GTAO) against the tick; PF15 was dropped for this reason
  (`docs/plan/quality.md:162`), and this card does not reopen it.

### 3.4 cargo-flamegraph — works on Windows, weaker answer

- **URL.** https://github.com/flamegraph-rs/flamegraph — "Windows: native support with
  the blondie library", sampling frequency default **997 Hz** (`-F`).
- **Verdict.** One static SVG, no off-CPU, no timeline. It is the right tool for a
  batch job, not for a frame loop, and its own README points at samply for an
  interactive UI.

### 3.5 Nsight Systems — GPU-bound frames only

- **URL.** https://docs.nvidia.com/nsight-systems/ — system-wide GPU/CPU timeline for
  Vulkan and DX11/DX12 among others.
- **Verdict.** Use it when the frame is GPU-bound and we need per-pass GPU time; it
  says nothing about the integer tick. Our `perf` crate already resolves GPU timestamps
  against real queries (PF2), so this is a second opinion, not the first.

## 4. wasm32 and `dist/web`

### 4.1 SIMD128 — the one landable web change

- **What.** Build `engine-web` with `-C target-feature=+simd128` and tell `wasm-opt`
  about it.
- **Proof that it is safe already exists.** `golden-matrix.yml` runs a
  `wasm32-wasip1-simd128` leg built with `-C target-feature=+simd128`, and
  `docs/HASH-LAW.md:162` records all four targets (incl. both wasm legs) as **MATCH**.
  `wasm-probe.yml` already builds the same tape with and without SIMD128 and prints the
  wall time.
- **Diff shape, two files.** `tools/web-build.mjs`: add `"--enable-simd"` to
  `WASM_OPT_FEATURES` (line 38) — the file already says wasm-opt "rejects a module
  that uses a feature it was not told about" (line 36) — and pass
  `-C target-feature=+simd128` in the cargo step. That is it.
- **Gain.** **Unmeasured on our page.** The tape proves the digests, not the speed. The
  place a win would show is the page's own split line: at `opt-level` 3 the frame is
  2.64 ms CPU with **2.31 ms of it in sim** (`Cargo.toml:347-356`, **measured**), and
  the sim is where `wide` lanes live.
- **Risk.** (a) Size: the budget has 3.3 % headroom (154,788 B), and SIMD128 code is
  usually *smaller* than the scalarised form but this is not measured; (b) the
  browser leg needs a SIMD-capable engine — SIMD128 is no longer in the active
  proposal list (`WebAssembly/proposals`), so every current browser has it; (c)
  `relaxed-simd` stays **off** (Rapier's determinism note in
  `docs/SPRINT-22-web-realism.md`).
- **Do:** `node tools/web-build.mjs`, then read `dist/web/build.json` for
  `engine_bg.wasm.bytes` against 4,700,000 and the page's frame split against 2.64 ms.

### 4.2 `wasm-opt` level — pick it by CPU, not by size

- **Measured.** `WASM_OPT_FLAGS` is `-Oz` plus six feature flags
  (`tools/web-build.mjs:38-39`), `wasm-opt` **117**, and the shipped module is
  `rawWasmBytes` 5,873,319 → 4,545,212 (**measured**, `dist/web/build.json`).
  `--opt-ab` exists and compares **sizes only**, keeping the smallest module; the last
  recorded build has `wasmOptAb: null`, i.e. it never ran.
- **Why this is a gap.** The page already proved that the *compiler's* size setting
  (`z`) is a 2.25x CPU loss against `-O3` (`Cargo.toml:347-356`). Nobody has asked the
  same question of wasm-opt's level, and `-Oz` is the most aggressive size setting in
  the pipeline.
- **Do.** `node tools/web-build.mjs --opt-ab`, keep the three modules, serve each, and
  read the page's split line. Record the CPU column next to the size table.

### 4.3 memory64 — rejected, we do not use the memory we have

- **URL.** https://github.com/WebAssembly/proposals — `memory64` is not in the active
  proposal list, so the spec is not the moving part here; our own budget is.
- **Measured.** The 10-minute soak peak is **15,990,784 B** (244 pages) against a
  **19,988,480 B** budget (`tools/web-budget.json` note, QW14). Peak is ~80 % of the
  budget, i.e. the module fits in 32-bit memory with room.
- **Reopens if** a soak crosses the `mem` budget; at that point the fix is pages, not
  a 64-bit memory.

### 4.4 Threads — rejected, still Phase 4

- **URL.** https://github.com/WebAssembly/proposals — **Threads** sits under
  "Phase 4 - Standardize the Feature". It is not merged into the spec.
- **Cost if we did it.** rustc needs nightly + `build-std` for atomics on wasm32, the
  host needs COOP/COEP headers (we already ship a `coi-serviceworker.js`, so the
  hosting half is partly paid), and the tick would need a deterministic split — which is
  PX15's problem, not a web flag.
- **Reopens if** threads reach the spec **and** the sim has a proven deterministic
  island split with an identical hash.

### 4.5 `opt-level = z` for the web profile — already refuted by measurement

- **Measured.** The web profile moved from `z` to `opt-level = 3`: frame CPU 5.95 →
  2.64 ms, wasm 1,067,194 → 1,319,755 B, gzip 462,762 → 528,682 B (`Cargo.toml:347-356`).
  The size went up and the page got 2.25x cheaper per frame; that trade is still inside
  today's budget. No change.

## 5. Ecosystem 2026: what we do not use

Checked against `Cargo.lock` rather than memory (**measured**): `wide 1.7.1`,
`puffin 0.20.0`, `mimalloc 0.1.52`, `hashbrown 0.17.1`, `smallvec 1.16.2`,
`glam 0.33.9`.

| candidate | status | verdict |
|---|---|---|
| `std::simd` | nightly (tracking #86656) | blocked by toolchain; `wide` covers us |
| `pulp` | not in the lock | same job as `wide`; adding a second SIMD abstraction buys nothing |
| `snmalloc` | not in the lock | same argument as 2.1; PF1 holds 0 allocs |
| `hashbrown` / `smallvec` | already in the lock (0.17.1 / 1.16.2) | nothing to add; no measured claim to chase |
| `glam` | 0.33.9 in the lock, workspace pins `"0.33"` (`Cargo.toml:189`) | its own release notes carry no perf number we could verify, so a bump is a maintenance decision, not a speed one |
| `cargo-pgo` | not installed | only a convenience wrapper over 1.1's flags |
| `samply` / `nextest` | not installed | tools, not crates; see 1.9 and 3.1 |
| `#[optimize]` | stable since 2026-10-08 (newer than 1.97.1) | see 1.10 |

So the answer to "what does 2026 add that we are missing" is: **one attribute and one
profiler**, both of which are scheduling questions here, not architecture.

## Ranked: what I would do, in order

| # | change | gain | effort | first experiment |
|---|---|---|---|---|
| 1 | `samply` on the 600-frame arena | turns "16.7 ms" into a ranked function list; the only item that makes the others measurable | S (one `cargo install`) | 3.1's three lines |
| 2 | Split `engine-render` (OPT11, not new) | 294 s of a 347 s dev build (**measured**) | L, structural | `cargo build -p engine-game --timings` |
| 3 | PGO on `ship` (**BSP1**) | **no donor number**; ours to measure, estimate 5-15 % frame CPU | S (tool already present) | 1.1's block, golden-gated |
| 4 | SIMD128 on `dist/web` (**WEB18**) | unmeasured; sim is 2.31 of 2.64 ms frame CPU | S (two lines in one file) | `node tools/web-build.mjs` |
| 5 | `wide` in one more hot loop (PF5, not new) | depends on 1's answer | S-M per loop | pick the loop from the profile |
| 6 | nextest gate wall (**BSP3** / OPT6) | 1.37-3.38x run time (**donor**), minus a doctest leg | S | install + one timed pair |
| 7 | codegen-units 8/16/32 (**BSP2**) | link wall, no runtime change | S | three builds, one bench each |
| 8 | `#[optimize(size)]` on cold paths (**BSP4**) | code size, not wall clock; blocked on toolchain | S after a toolchain bump | one cold module |
| 9 | `cold_path` in the tick's bail arms (**PF49**) | **estimate** small | S | one hunk, one A/B |
| 10 | `-C target-cpu=native` as a ceiling (**PF50**) | answers a question, ships nothing | S | one throwaway build |
| 11 | wasm-opt level by CPU (**WEB19**) | may repeat the `z`→`3` finding | S | `--opt-ab` + serve each |
| 12 | devirtualisation count on the tick (**PF51**) | **estimate** small | S | one grep, then at most one site |

## Proposed rows

`BSP` because the build-speed queue is full: `OPT` runs to **OPT58** and `PF` to
**PF48**; `docs/plan/build-cpu.md` closes OPT4/OPT39/OPT45/OPT46 by number, so a new
build-speed id is clearer than OPT59 in a grab-bag queue. Maxima verified at commit
time: OPT58, PF48, WEB17 (**measured** across `docs/`).

- **BSP1** — PGO on the `ship` profile, proven on the seeded 600-frame tape.
  Accept: three runs per arm, `interval_ms` p50 improves, golden
  `d57b5c8ae3096614` under the PGO ship build. Closes the open note in
  `Cargo.toml:339-341` ("no PGO until … a recorded-tape PGO A/B per PF3 land").
- **BSP2** — codegen-units sweep 8/16/32 on `release`. Accept: three build walls and
  three `interval_ms` p50s; 16 stays unless a number moves; 60 fps floor untouched.
- **BSP3** — nextest wall on the default gate, with a doctest leg beside it. Accept:
  both legs green, run times for both runners, doctest count unchanged. This is OPT6's
  missing number; close OPT6 with it instead of opening both.
- **BSP4** — `#[optimize(size)]` on one cold module after the toolchain passes 1.97.1.
  Accept: binary size down, `interval_ms` p50 and golden unchanged.
- **PF49** — `cold_path()` in the tick's bail arms, one hunk. Accept: p50 win at an
  identical hash, else revert.
- **PF50** — `-C target-cpu=native` ceiling on the arena. Accept: one number written
  down and the flags deleted; no shipped change.
- **PF51** — devirtualisation audit: count `dyn` on the tick path, convert at most the
  hottest site to an enum. Accept: the count in the report, plus an A/B.
- **WEB18** — SIMD128 in `dist/web` (`--enable-simd` + `-C target-feature=+simd128`).
  Accept: wasm bytes under 4,700,000, frame split under 2.64 ms, wasip1 SIMD128 tape
  still MATCH.
- **WEB19** — wasm-opt level chosen by measured frame CPU. Accept: `wasmOptAb` filled
  with both columns (bytes and ms) in `dist/web/build.json`.

## Rejected, and what reopens them

| idea | why not, with the number |
|---|---|
| BOLT | LLVM BOLT reads only x86-64/AArch64 **ELF**; we ship PE/COFF |
| Cranelift backend | nightly; no panic unwinding on Windows; `std::arch` partial (hits `wide`) |
| `-Zbuild-std` | nightly, and the page's cost is our sim, not `core` |
| mold on Windows | no Windows support; rust-lld already gave 121 → 79 s |
| sccache as a global wrapper | 0 of 2 units across fresh target dirs; keep it opt-in |
| allocator swap in the tick | PF1 holds 0 per-tick allocs; donor combined battle cost 0.90x-1.07x incl. 1.0 |
| `std::simd` | nightly only (#86656); `wide` already in the lock |
| `-C target-cpu=native` for ship | FMA rounding breaks the cross-platform hash (HASH-LAW) |
| `-Zvirtual-function-elimination` | unstable; documented to remove reachable vtable entries |
| `memory64` | soak peak 15,990,784 B of a 19,988,480 B budget |
| wasm threads | still Phase 4; needs nightly + build-std + COOP/COEP |
| `opt-level = z` for the web profile | measured: 2.25x worse frame CPU for 252,561 B saved |

## Still unverified

- **No A/B of PGO, `#[optimize]`, `target-cpu=native`, codegen-units, `cold_path` or
  the overflow-check cost on this tree.** Every gain above marked **estimate** is ours,
  not a number. The commands to get them are in the items.
- **No cranelift compile-time number for our tree** is quoted, because the only honest
  one is a timed clean build and nightly is not installed here.
- **SIMD128's size and CPU effect on `dist/web` is unmeasured.** The wasip1 tape proves
  digests, not time; `wasmOptAb` is `null` in the last recorded build.
- **`samply`, `nextest`, `wasm-opt`, `flamegraph`, `llvm-bolt` are not installed** on
  this PC (only `sccache` is), so every command in sections 1.9 and 3.1 needs its
  install step first. `llvm-profdata` is the exception and was run.
- **No LADDER line was added** for this card: the wave's scope was the card folder
  only, and `docs/research/LADDER.md` is read-only in that scope.
