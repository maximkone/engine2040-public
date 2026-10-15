# CODELAW — idiomatic Rust, measured complexity

## The slim law (operator, 2026-09-23: strong, slim, fast, no junk)

Every packet and review checks these. A packet that breaks one is rejected.

1. **One owner per type.** Never mirror another crate's type or constants to
   dodge a dependency edge (`game/src/practice.rs`, `render/src/live.rs` did);
   add the documented edge or move the type one layer down.
2. **Consumer first.** Code lands with a caller outside its own tests, in the
   same task or the one named next on the board. Unconsumed code gets a
   deletion task, not more features.
3. **Depth over breadth.** Deepen an existing module before adding one; a new
   crate needs a real boundary (application, platform adapter, heavy optional
   dependency). `FOCUS.md` CUT items get no code.
4. **Budgets are part of the API.** Every per-tick or per-frame system states
   its ms/MB/bytes budget and gets a release-mode benchmark in the perf gate.
5. **No steady-state allocation** in per-tick simulation or per-frame
   submission, proven by a counting-allocator test, never by claim.
6. **Deterministic authority.** Authoritative code uses `+ - * /`, `sqrt`,
   integer or fixed math, stable iteration (no `HashMap` order) and no wall
   clock. Presentation may use anything.
7. **Data-oriented hot paths.** Structure-of-arrays for per-entity data; no
   `dyn` call, lock or string formatting per entity per tick; nothing that
   blocks in the tick or the audio callback.
8. **One crate per role.** One physics stack, one ECS, one renderer, one
   allocator. A dependency row says what it replaces, its license is on the
   allow-list, and it stays off the headless default graph.
9. **Reuse.** Port or adapt code from any source (owner 2026-10-10); no license check or `EVIDENCE.md` row. An optional `PORT(owner/repo@rev:path)` comment helps later fixes. `deny.toml` governs Cargo dependencies only. Never hand-roll crypto.
10. **Honest evidence.** A CPU reference is not a GPU feature, a stub is not
    a feature, a data path is not a window. Receipts say what ran, where.
11. **Cross-platform by default.** Windows ships, Linux runs CI: no behavior
    that holds on one OS only (`TimedOut` vs `WouldBlock`, CRLF fixtures).
12. **Delete, don't archive in code.** No commented-out code, no TODO without
    a board ID, no task IDs or history in comments; module headers of at most
    about 15 lines: purpose, owner, units, determinism, errors.
13. **Tests catch bugs.** Each test fails on a plausible defect: negative
    cases, goldens with bump receipts, no restated constants; wall-clock tests
    are `#[ignore = "timing: ..."]` and run under `--perf`.
14. **Small surface.** `pub` only what a consumer uses; `pub(crate)` otherwise;
    features only for real optional dependencies.
15. **Import, never re-derive.** Shared primitives have one owner in
    `engine-core` (`Fnv64`/`hash_bytes`, `splitmix64`, `Rng`, `Fixed`,
    `alloc_probe`) and every crate may depend on it. Frozen means frozen
    behavior pinned by a golden test, never a reason to copy code.
16. **Measured, not claimed.** `node tools/rust-quality.mjs` scores every
    crate (`docs/RUST-QUALITY.md`); no packet raises a crate's debt count,
    new crates enter at grade A, and new lines pass the `rust-*` rules of
    `tools/packet-gate.mjs`. The baseline only goes down.
17. **Exceptions carry a reason.** `#[expect(lint, reason = "...")]`, never
    `#[allow]`; `#[ignore = "<why>"]`; waivers `// det-ok: <why>` and
    `// alloc-ok: <why>` are counted and reviewed like code.
18. **Short and flat.** A new function stays under 100 lines and a file
    under 1,500 production lines; grow a module, not a monolith. Pass
    structs over long argument lists; no free function that only forwards.

## Before writing

Identify the consumer, existing equivalent, ownership boundary and failure mode.
Reuse a suitable implementation; choose a maintained allowed dependency for
complex formats/crypto/platform work rather than hand-rolling to meet a line cap.
Dependencies need license/platform/MSRV/maintenance/feature-cost review.

## Rust rules

- Prefer explicit ownership and borrowing; generational handles for independently
  lived scene/ECS objects. Use `Arc` at actual thread/shared-resource boundaries.
  Avoid cyclic `Rc<RefCell<_>>` engine graphs, not every use of interior mutability.
- Structs/enums express domain state; newtypes distinguish IDs/units. Borrow
  `&str`/slices for read-only input. Use traits for real substitution boundaries
  (providers/devices/plugins), not one trait per struct.
- Organize systems over data; select SoA, AoS or archetypes from access patterns
  and measurements. Dynamic dispatch in a cold adapter is fine. Avoid per-object
  allocations/locks/virtual calls in measured hot loops where they cost time.
- Keep public APIs small but useful to their first real integration consumer.
  Simulation often uses construct/step/snapshot/restore; importers and editors
  need different contracts. Do not force every API into construct/step/hash.
- Typed errors in libraries, contextual reporting at application boundaries.
  Existing std-only crates can implement `Error` directly; `thiserror`/`anyhow`
  are choices subject to dependency policy, not mandatory uninstalled imports.
- Invalid external input returns errors; `unwrap`/`expect` are appropriate only
  for proven local invariants/tests with a clear reason, not a substitute for IO
  or stale-handle handling. Define overflow/NaN behavior where relevant.
- Unsafe is localized to a named invariant about validity, aliasing, alignment,
  lifetime and thread access. Document the caller/implementation obligations;
  use targeted tests and Miri/sanitizers when applicable. Never add unsafe for
  an unmeasured speed claim. Rust traits/struct layout are not a stable FFI ABI.
- Serialization is explicitly versioned and validated; never persist pointers,
  raw struct padding or compiler type IDs. Asset identity differs from content hash.

## Scheduling and performance

Fixed tick + seeded streams + deterministic order for authoritative simulation.
Presentation/services may use wall time, threads and async. Bounded jobs have
cancellation, queue limits and teardown ownership. Do not embed HTTP waits or
a general async executor into gameplay ticks.

Start simple. Profile with named hardware, build profile, scene/seed, warmup,
sample count and p50/p95/p99. Optimization claims require A/B measurements.
Avoid global “no allocations” claims without allocation instrumentation.

## Maintainability

Prefer cohesive functions/files and readable invariants over arbitrary limits on
lines, files or instructions. A two-file maximum must not split an atomic API
change or hide necessary tests. Extract shared code after ownership is clear;
textual similarity alone does not imply equivalent domain semantics.

Comments are short English, explain why, and keep history in docs/git. Public
contracts document units, errors, ownership and determinism where applicable.
Test behavior/invariants, not copies of the implementation or constants alone.

## Gates

Use `CONTRIBUTING.md` for runnable commands. Require relevant build/test/lint,
dependency checks and simulation hash ×2 before a requested commit. Backend
compilation, real backend execution, visual review and perf evidence are distinct.
Never silently update a golden or weaken a check to make a receipt green.

Quality has five layers (`docs/RUST-QUALITY.md`): workspace lints and the
simulation `clippy.toml`, the packet gate's `rust-*` rules, the per-crate
ratchet, review (judge Rust score under 3 fails) and the loop's `CQ` rows.
Never weaken a layer to land a packet: fix the code, or record a reasoned
`#[expect]` or waiver that the next review can see.
