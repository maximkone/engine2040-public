# RUST-QUALITY — the top-tier bar, measured and enforced

`CODELAW.md` owns the law; this file owns how it is measured and forced.
Run `node tools/rust-quality.mjs` for today's numbers; the baseline in
`tools/rust-quality.baseline.json` only goes down.

## 1. Measurement (2026-10-08, master 9295c6b, 214.9k production lines)

Scorecard: workspace **90/100 (A)**. Best: app 100, chaos 100, geo 100, input 100, physics 100.
Worst: render 66, core 82, scene 86, world 86.

| What | Count | Reading |
|---|---|---|
| Default clippy warnings | 0 | basics are clean |
| `.unwrap()` in production | 7 | agents already use `?`/`expect` |
| Numeric `as` casts | 3,173 (12.6/k lines; `node tools/rust-quality.mjs --json --no-cache` 2026-10-13, 251.6k lines) | silent truncation and wrap; target 5/k |
| Comment lines naming board IDs | 3,068 | CODELAW 12 ignored in 252 files |
| FNV/SplitMix re-derived outside core | 307 lines | one primitive, ~100 copies |
| Duplicated function bodies | 214 in 126 groups | byte readers, sRGB, hashes, alloc probes |
| Functions over 100 lines | 111 | largest new one: 375 lines (wave 46) |
| Files over 1,500 production lines | 12 | `render/src/present.rs` 5,704 lines |
| Allocations in step/tick/render/mix fns | 177 | 147 in render: per-frame heap churn |
| HashMap/Instant in simulation crates | 34 | net 22, scene 10, spatial 2 |
| Hand-rolled `GlobalAlloc` probes | 8+ crates | `unsafe` copied for every zero-alloc test |

Clippy pedantic + nursery + restriction probe (report only): 30,397
diagnostics, led by casts (`cast_possible_truncation` 1,086 production),
`missing_errors_doc` 1,440 and `map_err_ignore` 220 (error context dropped).

Backtest of the new packet gate on the last 23 agent commits (8,166 added
lines): **19 would be refused**: 116 board-ID comments, 8 new FNV copies
(landed after SL5 asked for one owner), 1 new 375-line function.

Verdict: agent Rust is locally tidy (units in names, typed errors, docs on
items, few panics) but globally weak. Agents re-derive primitives instead of
importing them, narrate history in comments, grow giant files and functions,
and allocate in hot paths. Root causes: only a few crates had an allowed edge
to `engine-core`, so copying was the path of least resistance; nothing
measured quality between packets; per-packet review cannot see cross-crate
duplication.

## 2. Five enforcement layers

| Layer | Mechanism | Runs | Refuses |
|---|---|---|---|
| 1 Compiler | `[workspace.lints]` (84 lints, each at zero when enabled); `#[expect(lint, reason)]` only; `tools/sim-clippy.toml` copied into every simulation crate | every clippy run: packet, proof, CI | any new hit; HashMap/HashSet/Instant/SystemTime, libm trig/exp/powf and `mul_add` in simulation crates (type-aware, tests included) |
| 2 Packet gate | `tools/packet-gate.mjs` `rust-*` rules on added lines, reading the lane's post-patch files | every collected lane, before checker and judge | `unwrap`, print in libraries, `allow`, `expect` without reason, `dbg!`, nondeterminism in sim, FNV/probe copies, board IDs in comments, string errors, allocation in hot fns, new fn over 100 lines, new file over 1,500 lines. Warns on new casts and growth |
| 3 Ratchet | `tools/rust-quality.mjs` per-crate counts vs baseline; sim-config drift | `proof.mjs`, CI, every wave | any count rise; a new crate below grade A; a simulation crate missing the sim config |
| 4 Review | checker (mechanical), judge Rust score 0-5 (under 3 fails), api-reviewer (Rust API + Pragmatic Rust Guidelines) | every packet | taste and design the tools cannot see |
| 5 Loop | `/sprint` quality duty: one `CQ` writer every wave; lint ladder; score line in every handoff | every wave | a wave that ends with the score down |

The ladder: debt is ratcheted (layer 3) -> burned down by `CQ` rows -> at
zero the lint joins `[workspace.lints]` (layer 1) and can never return.
Raising a baseline needs `--accept <crate> --reason`, logged in the file.

## 3. Metrics (tools/rust-quality.mjs)

Densities are per 1,000 production code lines. `target` scores full
weight, `zero at` scores nothing; weights sum to 100.

| Metric | Weight | Target | Zero at | Why |
|---|---|---|---|---|
| unwrap | 8 | 0 | 1 | crash on bad input; `?` or `expect("<invariant>")` |
| panic_sites | 6 | 1 | 5 | panics are for bugs only (M-PANIC-ON-BUG) |
| casts | 8 | 5 | 40 | `as` truncates and wraps silently; determinism risk |
| allow_no_reason | 4 | 0 | 1 | exceptions state why |
| dead_code_allows | 3 | 0 | 0.5 | consumer first (slim law 2) |
| print_in_lib | 4 | 0 | 1 | libraries report through errors/observe |
| string_errors | 5 | 0 | 1 | typed errors in libraries |
| board_ids | 7 | 0 | 5 | history belongs in git (slim law 12) |
| hash_reimpl | 6 | 0 | 1 | `engine_core::{Fnv64, hash_bytes, splitmix64}` |
| dup_fns | 8 | 0 | 2 | one owner per behavior (slim law 1) |
| passthrough | 3 | 0.3 | 3 | a wrapper that only forwards is API noise |
| long_fns | 7 | 0.1 | 2 | over 100 lines: split into named steps |
| big_files | 4 | 0 files | 3 files | over 1,500 lines: deepen as modules |
| hot_alloc | 8 | 0 | 3 | no steady-state allocation (slim law 5) |
| nondet | 7 | 0 | 2 | stable iteration, no wall clock (slim law 6) |
| undoc_pub | 4 | 5% | 50% | agents and users read docs (M-DESIGN-FOR-AI) |
| tests | 8 | 10/k | 0 | behavior coverage (higher is better) |

`waivers` (`// det-ok:`, `// alloc-ok:`) and `unsafe` are counted and
ratcheted with zero weight: every one is visible and budgeted.

## 4. The bar by layer

- **Core and foundation.** One owner per primitive: hashing (`Fnv64`),
  streams (`Rng`, `CounterRng`), fixed math, alloc probe
  (`engine_core::alloc_probe`, feature `test-util`). Any crate may depend on
  `engine-core` without an edge row. Frozen means frozen behavior, pinned by
  golden tests, not frozen code: refactor into the owner and keep the golden.
- **Simulation.** Integer or fixed math, `+ - * / sqrt`; `total_cmp` for
  float ordering (NaN never panics); BTreeMap or sorted Vec; `engine_core::Tick`
  for time; inputs, outputs and budgets stated; `alloc_probe::measure` proves
  0 allocations per tick after warmup.
- **ECS and data.** Structure-of-arrays columns with generational handles
  and canonical bytes; systems over slices, no `dyn` or lock per entity.
  The engine's ECS is still a two-component prototype: ARC1/ARC2 (spine
  components and schedule) are the core gap, ready now.
- **Physics.** Sorted broadphase keys, never a hash map; islands and solver
  order fixed; NaN inputs refused at the boundary; budget at 64 bodies and
  1,000 agents in the perf gate.
- **Render and visuals.** A pass counts only when it submits on a device;
  per-pass GPU timestamps with a budget (VISUAL-TARGET); no allocation in
  frame submission (frame arenas, reused buffers, pipeline cache); render
  graph resources declared with lifetimes; captures scored by FLIP against
  the path-traced reference (PQ4) before a look row ticks. CPU twins live
  under `#[cfg(test)]` as oracles (SL3).
- **Animation.** Pose buffers reused per frame; blend and IK over slices;
  integer tick time; no allocation in `update`.
- **Audio.** The mix callback never allocates, locks or logs: prove it with
  `alloc_probe` today and RealtimeSanitizer (`rtsan-standalone`) on Linux.
- **Net.** Wire formats versioned and bounded; decode never panics on
  hostile bytes (fuzzed); no wall clock in rollback state.
- **Assets and IO.** IO off the tick; one byte-reader owner per format;
  typed errors naming the offset; cooked data versioned.
- **Editor, tools, MCP.** Every edit through the command service; typed
  refusals; binaries may report context, libraries return typed errors.
- **AI providers.** Deadlines, cancellation, bounded queues; replay never
  calls a model.
- **Tests.** Each test fails on a plausible defect (M-TAUTOLOGICAL-TESTS);
  oracles over restated constants; `#[ignore = "<why>"]` only.
- **Performance evidence.** Wall-clock tests are ignored timing tests;
  instruction-count benches (Gungraun) are the CI-stable perf gate.

## 5. Commands

```text
node tools/rust-quality.mjs                    # scorecard + ratchet (proof, CI)
node tools/rust-quality.mjs --crate physics    # one crate before reporting
node tools/rust-quality.mjs --list hot_alloc --crate render   # burn-down list
git diff -- <owned path> | node tools/packet-gate.mjs --patch - --owned <path>
node tools/rust-quality.mjs --tighten          # orchestrator, after a green wave
```

## 6. Sources (read 2026-09-26; ideas and lint lists, no code copied)

- Bevy `Cargo.toml` workspace lints: `allow_attributes(_without_reason)`,
  `undocumented_unsafe_blocks`, `redundant_else`, `ptr_as_ptr`, `unsafe_code`.
- Embark Studios `rust-ecosystem` lints: a game studio's standard set
  (`dbg_macro`, `todo`, `mem_forget`, `linkedlist`, `lossy_float_literal`...).
- Microsoft Pragmatic Rust Guidelines: M-STATIC-VERIFICATION, M-LINT-OVERRIDE-EXPECT,
  M-TEST-UTIL, M-PANIC-ON-BUG, M-HOTPATH, M-MEM-REUSE, M-DESIGN-FOR-AI,
  M-TAUTOLOGICAL-TESTS, M-RUST-SHAPED.
- TigerBeetle TIGER_STYLE: assertion density, static allocation after init,
  bounded loops and queues, short functions.
- cargo-mutants `--in-diff`: mutation testing limited to a patch.
- Gungraun (formerly iai-callgrind): instruction counts, stable in CI.
- rtsan-standalone: RealtimeSanitizer for real-time callbacks.
- Ericson, *Real-Time Collision Detection*: sort-based uniform grids.
