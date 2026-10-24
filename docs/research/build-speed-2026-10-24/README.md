# Build speed, wave 2026-10-24

Read with `../rust-speed-2026-10-23/README.md` (every lever already measured or
rejected, build time and runtime) and `../build-cpu-2026-10-21/CARD.md` (the
profile change that landed, OPT45). This page adds only what those two do not
hold: three levers that changed in 2026, one dead end measured today, and the
honest ceiling on this PC.

## The one-line answer

Nothing left in configuration gives more than about 20 % here. The ceiling is
structural: `engine-render` is 294 s of a 347 s dev build (99.7 s frontend,
194.3 s codegen), and no compiler flag moves a build that sits in one crate.
The only 100x in reach is the edit loop, not the build: hot-reload the game
crate (OPT13, still unmeasured) and a gameplay edit costs seconds, not minutes.

## 1. What changed in the Rust project in 2026

### 1.1 headstart - up to 54 % on a clean build, and not installable

- **What.** Six rustc patches and three cargo patches that let a dependent crate
  start compiling against a dependency's *interface* while rustc is still
  checking that dependency's function bodies. Today a workspace builds as a
  staircase, one crate after another; with this the steps overlap.
- **Measured by its authors** on 13 real projects (rust-analyzer, Zed, Bevy,
  lemmy, polars and others), 16 cores: up to **54 % faster `cargo check`** and
  up to **42 % faster `cargo build`** on clean builds, and none of the 13 got
  slower.
- **URL.** https://github.com/PowderworksCode/headstart (Sep 2026),
  results in `docs/results.md` there.
- **Why it is not used today.** It is a patch series to rustc and cargo, not a
  release, so there is nothing to install on 1.97.1 and no nightly carries it.
- **Reopens when** it lands in a rustc nightly. It is the largest single clean
  build lever on the horizon for this tree, and the only one that attacks the
  347 s figure without a refactor.

### 1.2 Relink-Don't-Rebuild - the fix for exactly our shape

- **What.** When a change only touches function bodies, downstream crates do not
  need recompiling at all; the linker can do the work. The Rust 2026 Fast Builds
  goal quotes **5-10x** on those rebuilds.
- **Why it matters here.** Our workspace has 44 members; a body-only edit in a
  leaf crate still walks the chain to `engine-game` and `arena`.
- **URL.** https://goals.rust-lang.org/2026/roadmap-fast-builds.html
- **Status.** Not shipped. Nothing to apply; watch it, do not plan on it.

### 1.3 TPDE - the 10-20x headline, blocked by COFF

- **What.** A baseline compiler back-end that produces code **8-26x faster than
  LLVM `-O0`** at about the same run-time quality (CGO 2026 paper, arXiv
  2505.22610; SPECint 2017, geomean 12.15x on x86-64). Rust has a 2026 goal to
  ship it as an experimental codegen option.
- **The blocker.** It targets **x86-64 ELF and AArch64 ELF**. Our artifacts are
  PE/COFF, so this is not a tuning question - the same reason BOLT was rejected
  in the 2026-10-23 card.
- **Reopens if** a COFF path appears.

### 1.4 Parallel frontend - still nightly, and the stable flag will be `-j`

- **Status (2026).** Still `-Zthreads` on nightly only. A stabilization plan
  closed as `rust-lang/compiler-team#1005`; the stable interface will be
  `-j` / `--jobs`. Nightly now defaults to 2 frontend threads
  (`rust-lang/rust#162848`). Tracking issue `rust-lang/rust#113349`.
- **Why it is worth a number.** `engine-render` frontend is 99.7 s of its 294 s,
  the second-largest slice after codegen. This is the one unmeasured lever with
  a plausible 20-40 % on that crate, and it needs no code change.
- **Cost.** A nightly toolchain plus a separate target dir: one cold build of the
  tree. It cannot be measured inside a normal iteration.

## 2. Measured today: dependency dedupe is a dead end

Both earlier cards point at dependency unification as a lever, and
`SMOOTH-BUILD.md` names "the unification target is the 80 multi-version names".
That figure comes from parsing `Cargo.lock`, which counts every platform copy.
Measured on the graph we actually compile:

- `Cargo.lock`: 1,052 packages, 80 multi-version names, 101 extra copies.
- `cargo tree -d --workspace` (host target, default features): **5 duplicated
  names, 2 versions each** - `cpufeatures`, `getrandom`, `miniz_oxide`,
  `once_cell`, `syn`.
- The same command with `--target all` prints 62 packages instead of 10: those
  are platform copies (`windows-sys` for other ABIs, `redox_syscall`, `wasi`)
  that never compile on Windows. That gap is the difference between the
  lockfile count and the work we actually do.
- The `syn` pair is v2.0.119 (only from `ssimulacra2` through `equator-macro`)
  and v3.0.6 (from `bytemuck_derive`). Both are small and neither is removable
  without dropping a dependency we use.

So there is nothing to win here, and OPT36 (`engine-workspace-hack`) should not
be wired for speed. The crate is externals-only on purpose; adding it to members
would pull `image` into crates such as `engine-anim` that never touch it, which
makes those `-p` builds slower, not faster. Feature unification already happens
inside a single cargo invocation; the hack only helps when you alternate `-p`
targets, and this workspace has 5 minor duplicates to unify.

## 3. WSL2 - owner said no, numbers kept for the record

Building the same tree inside WSL2 with the source in ext4, from same-machine
benchmarks: `cargo build` 52.9 s -> 30.8 s and `cargo build --release`
136.4 s -> 76.9 s, about **1.72x**. The penalty for building across `/mnt/c` is
large enough to erase it. Owner decision 2026-10-24: no WSL. Recorded so the
next session does not re-propose it without new evidence.

## 4. Headroom, ranked

| Lever | Gain on this tree | Door |
|---|---|---|
| Hot-reload the game crate (OPT13) | 347 s -> seconds, per gameplay edit | open, unmeasured, no toolchain change |
| Split `engine-render` (OPT11) | 1.4-1.8x on the default-feature loop; **0** on an `--all-features` loop | open; three cycle-breaks first, section 6 |
| `-Zthreads` on nights | 20-40 % of a 99.7 s frontend | nightly toolchain + one cold build |
| headstart | up to 42 % clean build | not shipped |
| RDR | 5-10x on body-only rebuilds | not shipped |
| TPDE | 8-26x codegen | ELF only, blocked |
| Cranelift | 5-12 % (Zed, line tables) | blocked, see 2026-10-23 card 1.3 |
| PGO | 5-15 % frame CPU, unmeasured | two fat-LTO builds, BSP1 |
| Dependency dedupe / workspace-hack | ~0 | measured dead end, section 2 |
| sccache across lanes, wild, mold, BOLT | 0 | already rejected, both cards |

## 5. Order I would take

1. OPT13 hot-reload spike on one gameplay crate. Only lever with 100x-shaped
   payoff, and it costs no toolchain change.
2. OPT11 split of `engine-render`, with a `--timings` before/after.
3. `-Zthreads=8` measured on `engine-render` alone, once a nightly is installed
   for that spike only.

## 6. OPT11 baseline and the split plan (measured 2026-10-24)

Three runs on this PC, each after `touch crates/render/src/lib.rs`, `cargo` in the
slot pool, dev profile, `--timings`. The pool had 0 free slots at the start of
the first run, so every number below is a contended-box number: per-unit times
are real waits, not pure compile cost.

| Run | Command | Wall | Units | Peak / mean concurrency |
|---|---|---|---|---|
| A | `cargo build -p engine-render --all-features` | **101.3 s** | 14 | 7 / 3.58 |
| B | `cargo build -p engine-game --all-features` | **287.1 s** | 60 | 10 / 8.03 |
| C | `cargo build -p engine-game` (default features) | **102.7 s** | 15 | 9 / 5.97 |

### The premise in PW44 does not survive measurement

PW44 says `crates/render` "owns the dev build's critical path (294 s of a 347 s
build)". Run B reproduces the 287 s figure, and in it `engine-render` is **113.1 s
of 287.1 s, and not the critical path**:

| Unit | Wall span | frontend | codegen |
|---|---|---|---|
| rapier3d | 230.5 s | 17.6 | 212.9 |
| naga | 195.7 s | 24.6 | 171.1 |
| wgpu-core | 193.8 s | 6.6 | 187.2 |
| parry3d | 147.6 s | 8.5 | 139.1 |
| wgpu-hal | 144.9 s | 5.8 | 139.1 |
| engine-assets | 139.7 s | 6.2 | 133.5 |
| **engine-render** | **113.1 s** | 18.3 | 94.8 |

The `--all-features` wall is set by third-party dependency codegen at opt-level 3
(`[profile.dev.package."*"]`). **No split of `engine-render` can move run B**, and
that is the run a full-feature gate does. Splitting render only pays on the
default-feature loop (run C), where it is the largest single unit:

| Unit | Wall span | codegen |
|---|---|---|
| engine-spatial | 100.2 s | 94.6 |
| **engine-render** | **92.0 s** | 78.7 |
| engine-assets | 67.0 s | 61.1 |
| engine-net | 66.1 s | 60.0 |
| engine-game | 59.0 s | 51.1 |
| engine-arena | 42.3 s | 39.6 |

Run C overstates its own breadth: it ran straight after run B, so the feature
flip rebuilt units outside render's closure (spatial, net, nav, world, ai,
audio). A clean warm loop after a render edit is render plus its dependents.

Render on its own, warm deps, near-solo: **48.6 s, frontend 18.2 s + codegen 30.4 s**
(run A's unit). Under run C's contention the same crate reports 92.0 s. So the
intrinsic span to beat is about 50 s, and the split's ceiling on the warm loop is
"two units of ~25 s that compile at the same time".

### The three cycles that block the split

The module graph (181 files, 5.2 MB of source, 131k lines) has three hubs and
three back-edges. A split needs these broken first, and they are small:

1. `graph` -> `gtao`: `crate::gtao::LookFrame` at `graph.rs:1046`, `:1199`,
   `:1619-1620`. Move `LookFrame` into a shared `types` module.
2. `graph` -> `executor`: `executor::none_gpu_timings`, `timestamp_setup`,
   `query_pair`, `read_pass_ms` at `graph.rs:1233`, `:1292`, `:1307`, `:1420`.
   `executor` is 51 KB and only needs `graph` for types, so it moves down with it.
3. `backend` -> `present` and `shadow_post`: `present::Presenter::device_descriptor`
   at `backend.rs:1170`, `shadow_post::fnv1a64` at `:515`, `:527`, `:944`. Both are
   single functions that belong in `gpu` and a util module.
4. `test_gpu` -> `present`: 29 modules use it, and it is public API for other
   crates' tests (`engine_render::test_gpu::require_or_skip` in `app`, `editor`,
   `assets`), so it stays a public module but must stop reaching upward.

### Layering the split lands on

| Crate | Holds | Why |
|---|---|---|
| `engine-render-base` | `backend`, `graph`, `gpu`, `executor`, `buffers`, `uploads`, `frame`, `frame_alloc`, `adapter`, `shader_gate`, `variants`, `manifest`, `webgpu_limits`, `cache`, `test_gpu` | the frame graph and the device; edited rarely |
| `engine-render-passes` | `ibl`, `gtao`, `fog`, `sky`, `post`, `aa_scale`, `tonemap`, `bloom_dual`, `lens`, `upscale_easu`, `shadow_cluster`, `shadow_cascade`, `shadow_post`, `rt_shadows`, `contact_shadow`, `torch_light`, `spot`, `decals`, `accel`, `baked_gi`, `gi`, `gi_probe`, `particles_gpu`, `wound_map` | the hot edit surface; one crate per layer keeps the pass edit inside one unit |
| `engine-render` | `present` (1.35 MB, the top hub), `v1*`, `kam2*`, `window`, `capture_path`, `bin` | the public facade; re-exports the two below |

Accept for the row, measured the same way: `touch crates/render/src/lib.rs`,
`cargo build -p engine-game`, wall under 72 s (30 % off run C), `cargo llvm-lines`
per-crate table before and after, golden `d57b5c8ae3096614` twice.

### The cycles, re-measured after steps 1-2 (2026-10-24)

The first inventory counted every `crate::` mention. Most of them are inside
`#[cfg(test)]` blocks, so the production graph is far cleaner than the file
graph. Checked against the `#[cfg(test)]` line of each file:

| File | lines | `#[cfg(test)]` at | upward refs below it |
|---|---|---|---|
| `graph.rs` | 2505 | 1628 | 4 (`executor::none_gpu_timings`, `timestamp_setup`, `query_pair`, `read_pass_ms`) |
| `backend.rs` | 1274 | 769 | 0 |
| `executor.rs` | 1350 | 696 | `crate::graph` at 627-654, window-gated |

So the remaining production back-edges are exactly three, not the seven the
file graph suggested:

1. `graph` -> `executor` timing helpers: `graph.rs:1233`, `:1292`, `:1307`,
   `:1420`. They are `pub(crate)` and gated `#[cfg(any(feature = "gpu", test))]`
   in `executor.rs:458-620`. Moving them to a `timing` module removes the last
   cycle that touches `graph`, and it is a contiguous 165-line move.
2. `test_gpu` -> `present`: `test_gpu.rs:224` calls `Presenter::init_offscreen()`
   and matches `present::PresentError::NoAdapter`. This is the only
   base-to-`present` edge in production code; 29 modules call
   `test_gpu::require_or_skip`, and three other crates use it, so the probe has
   to be inverted (take the probe as a parameter) rather than deleted.
3. `shader_gate` -> seven passes: `shader_gate.rs:142-171` names each pass's
   `*_WGSL` constant in a registry. Nothing else in it reaches upward. The
   registry belongs with the passes, so `shader_gate` is a passes-crate module.

Everything else that looked like a cycle (`graph` -> `present`, `graph` ->
`baked_gi`, `backend` -> `present`, `backend` -> `v1`) is test-only and moves
with the tests.

Landed so far, each a green `cargo check -p engine-render --all-features`:

- `b35c26bdb` - `LookFrame`, `LookError` and `MAX_LOOK_DIM` move to `types.rs`;
  `gtao` re-exports them, so every `gtao::LookFrame` path still resolves. The
  graph no longer names `gtao`; `half_size()` stays in `gtao` because it uses
  `GTAO_HALF_DIV`.
- `ef52c335a` - `fnv1a64` moves to `types.rs`; `shadow_post` re-exports it.
  `backend` no longer names `shadow_post`.

- `c4f0374e4` - the per-pass timestamp helpers move out of `executor` into
  `timing.rs` (165 lines, pure math plus one wgpu query-set builder).
  `graph` now calls `crate::timing`, not `crate::executor`.

After those three commits the **production module graph is acyclic**. Every
remaining upward reference in `graph`, `backend`, `test_gpu` and `shader_gate`
sits inside `#[cfg(test)]`:

- `shader_gate.rs` puts the WGSL registry at 142-171, inside `mod tests` from
  109. Not a production edge.
- `test_gpu.rs:224` calls `Presenter::init_offscreen()`, inside a
  `#[cfg(test)]` fn. Not a production edge.
- `backend.rs` reaches `present` and `v1` only past its `#[cfg(test)]` at 769.
- `graph.rs` reaches `present` and `baked_gi` only past its `#[cfg(test)]` at
  1628, except the `gx1_test_frame` helper at 1619-1620, which is
  `pub(crate)` and used by `executor`'s tests.

So the split no longer needs a design change, only a mechanical move plus one
decision: the base crate's tests need the passes crate, and cargo allows that
as a `[dev-dependencies]` edge (a dev edge may close a cycle with a normal
one). Test code that says `crate::present::…` or `crate::baked_gi::…` becomes
`engine_render_passes::…`, and nothing in production changes.

## Sources read live 2026-10-24

- https://goals.rust-lang.org/2026/roadmap-fast-builds.html
- https://github.com/PowderworksCode/headstart
- https://github.com/rust-lang/compiler-team/issues/1005
- https://github.com/rust-lang/rust/pull/162848
- https://github.com/rust-lang/rust/issues/113349
- https://github.com/tpde2/tpde and https://arxiv.org/abs/2505.22610
- https://github.com/rust-lang/rustc_codegen_cranelift
- https://github.com/wild-linker/wild and https://github.com/rui314/mold
- https://github.com/rksm/hot-lib-reloader-rs
- https://users.rust-lang.org/t/slow-compile-times-on-windows/119144
