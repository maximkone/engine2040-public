---
name: engine-code
description: "Use when you write Rust in an engine2040 crate: a new module or crate, Cargo manifest, feature or dependency, public API, rustdoc, cross-crate refactor, or a donor port. Reuse ladder, typed errors, hot paths, area rules."
---

# Write engine code

Other skills own the details. Load them by name:

- `rust-safety`: unsafe, statics, locks, threads, numeric `as`.
- `tick-and-runtime`: fixed step, frame budget, allocation, jobs, loop.
- `test-and-proof`: how to prove a diff. `review`: the quality bar and the 0-5 score.
- `github-check`: heavy cargo on a runner. `commit-and-board`: commit and tick rows.
- `determinism-debug`: the golden hash moved.

## 1. Before code

1. `git status`. Preserve changes you did not make.
2. `node tools/board-slice.mjs <ROW>`. A ticked row, or a named function that already exists, is a NOOP: say so and stop.
3. Read the crate's public API and tests. Grep its section of `docs/ARCHITECTURE.md` (not the whole file) and `docs/CODELAW.md`.
4. Name the consumer, the data owner, the units, the determinism scope and the error type.

## 2. Workflow

1. Smallest complete change. Ladder: does it need to exist, then reuse, then std, then an allowed installed crate, then new code.
2. Prefer a module. Add a crate only for a real dependency, platform or ownership boundary.
3. Typed ids and errors. Bounded buffers. Validate external input (sizes, ranges, NaN, versions). No `unwrap` on IO or user data.
4. `engine-core` stays std-only. IO, GPU, model and network work stays outside the fixed tick. Job completion order never decides sim state.
5. Test the behavior and one failure case. Then wire the real consumer. A trait that compiles is not integration.
6. `git grep` every new `pub` name. A caller outside tests must exist. Zero callers: wire one (tick, submit, encoder) with a behavior test on the call path, or board its consumer row.
7. Finish the capability in one row. If it cannot land whole, say exactly what remains.
8. Proof: `cargo check -p <crate>` while you work. Then `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` and `... cargo clippy -p <crate> --all-targets -- -D warnings`. `rustfmt --edition 2021 --check <files>` (never `cargo fmt`).
9. If sim code changed: `cargo test -p engine-proof --lib golden_hash_untouched` and `cargo run -p engine2040 --release` twice, same `final_hash=d57b5c8ae3096614`. `-p` is the package name in `crates/<dir>/Cargo.toml`.

"Blocking waiting for file lock" is another cargo build in `target/`. Wait.

## 3. Checklist for every new line

- **Reuse first.** `git grep` the behavior before you write it. Owners in `crates/core/src/lib.rs`:
  - hashing: `engine_core::{Fnv64, hash_bytes, hash_u64s, mix64, splitmix64}`;
  - rng: `Rng` (`fork`, `fork_named`), `CounterRng`;
  - clock: `time::IntAcc`, `tick` (`MAX_STEPS`);
  - math: `Fixed`, `det_math` (`det_sin`, `det_cos`, `det_exp`), `vec3`, `quat`;
  - conversions: `checked` (saturating and decimal parsers);
  - zero-alloc proofs: `alloc_probe::measure` (dev-dependency feature `test-util`).
  Any crate may depend on `engine-core`.
- **Errors.** Typed enums with `Display`. `?` over `unwrap`. `expect` names the proven invariant. Panics only for bugs. Binaries may add context.
- **Numbers.** `From`/`TryFrom` or a named tested conversion, not `as` (see `rust-safety`). `total_cmp` for float order. Units in names or newtypes. Never `mul_add` in sim code.
- **Hot paths.** No allocation, `format!`, lock or `dyn` per entity per tick. Buffers owned by the system and reused. Slices in, slices out. `sort_unstable` for keys. Prove with `alloc_probe::measure`.
- **Shape.** Functions under 100 lines, files under 1,500. A params struct over 7 arguments. No free function that only forwards. `pub(crate)` unless a consumer needs it. Docs on every `pub` item.
- **Exceptions.** `#[expect(lint, reason = "...")]`, never `#[allow]`. `#[ignore = "<why>"]`. `// det-ok:` and `// alloc-ok:` waivers are reviewed.
- **Comments.** Why, not history. No board ids, sprint numbers or "was X". Git carries history.
- **Tests.** Each fails on a plausible defect: oracle, negative case, property over seeds. Never restate a constant.

"Ponytail" from the user: shortest diff that works, one check, mark deferred work `ponytail: <path>`. Never cut validation, determinism or tests.

## 4. Area rules

**Simulation** (`docs/HASH-LAW.md`): fixed dt, explicit seeded streams (`Rng::fork`), stable iteration order, NaN guards. No `HashMap` order, wall clock or pointer value in hashed state. A new fixture gets its own version and a non-aliasing seed expansion. Rollback needs identical results on every PC: sim code uses `+ - * /`, `sqrt`, `Fixed` or `det_math`, never `sin`, `exp`, `powf` and friends.

**Scene, editor, commands** (`docs/EDITOR_AI_CONTROL.md`): persistent ids, generational handles and content digests are three different things. One schema feeds serializer, inspector and AI tools. Every edit (inspector, gizmo, script, AI) goes through the command service: validate, preview without side effects, atomic apply, journal, undo. A stale revision or bad id writes nothing. Play runs in a child process.

**Render** (`docs/VISUAL-TARGET.md`): work stage by stage, V1 to V5. Right-handed, +Y up, meters. Explicit linear vs sRGB. Handle resize, minimize and device loss. Pixels never enter the sim hash. See `wgpu-render`, `render-pipeline-math`.

**Audio and real time:** the mix callback never allocates, locks, logs or blocks. Prove it with `alloc_probe::measure` over the callback body.

**AI** (`docs/EDITOR_AI_CONTROL.md`, `docs/RESEARCH-VERDICT.md`): engine-owned request, result and capability types. Providers are adapters (mock, local, remote) with deadlines, cancellation, bounded queues and offline fallback. Model output is a proposal: validate schema, ranges, references and revision, run `engine-spatial` checks, then apply through the command service. Models give labels and intent. Deterministic code owns geometry. Replay uses recorded accepted actions and never calls a model. See `ai-door`.

## 5. Dependencies

- A new Cargo dependency's license must pass `deny.toml`. Run `cargo deny check`.
- Add its row to `docs/DEPENDENCIES.md`.
- Prefer small crates without C toolchains. Put heavy ones behind a feature.
- Donor source from any repo may be ported directly (owner 2026-10-10). No EVIDENCE row. Never hand-roll crypto.
- Use `workspace = true` and the root `[workspace.dependencies]` table.

## 6. Docs

Readers are a stranger building a game and an AI calling the engine through MCP. Write for both.

- Every sentence is true of the code on disk. Never document a planned or `Unsupported` path as working. Say what it does today and name the board row that finishes it.
- Public items: one-line summary, then units (meters, ticks, permille), ownership, threading, determinism scope, errors and panics. Match the module header style (`crates/arena/src/fork.rs` is the model).
- Every public entry point gets a short runnable example (doctest or `examples/`). `cargo test --doc -p <crate>` and `cargo run -p <crate> --example <name>`.
- The mdBook in `docs/book/` holds guides: quick start, one page per system, recipes. Link to rustdoc. Never copy signatures by hand.
- Coverage: raise `#![warn(missing_docs)]` crate by crate. Report `cargo doc --no-deps` warnings before and after. `broken_intra_doc_links` is a workspace rustdoc lint.

## 7. Cross-crate refactor

Before the first edit, write down:

1. The crates in play, and what is out of scope (generated files, vendor drops, `Cargo.lock`, local cargo config).
2. Batches. Each names its crates and files. No touched crate is missing.
3. One CI line and one rollback line per batch, for example `cargo test -p engine-net` and "revert `quic.rs`".
4. Order: security, breaking bugs, architecture, performance, style last.

Land one batch at a time. Restate the out-of-scope list at each batch.

## 8. Cargo

Facts that bit this repo. More in `references/cargo-cheatsheet.md` and `references/cargo-patterns.md`.

- Features unify: a shared crate builds once from the union of requested features. Every feature must be additive.
- `[profile.*]` is read only from the workspace root manifest. Dependency profiles are ignored. The root has `dev` (opt-level 1, dependencies 3, 13 sim crates 2), `release`, `profiling`, `perf-test`, `ship`, `web`. `.cargo/config.toml` adds `fast-test` and sets `test` to `debug = 0`.
- `release` here sets `overflow-checks = true`. Cargo's default is off. Sim code panics instead of wrapping.
- The nearer `.cargo/config.toml` wins. Member config files in a workspace are not read.
- `Cargo.lock` pins revisions. Never hand-edit it. `cargo update -p <name>` refreshes one entry.
- Targets are auto-discovered (`src/bin/`, `examples/`, `tests/`). `required-features` hides a bin: `staged_fight` and `arena` need `--features app`.
- Root `Cargo.toml` is a package plus workspace with `default-members = ["."]`. A bare `cargo build` builds only the root. Use `-p <crate>` or `--workspace`.
- A virtual manifest (no `[package]`) needs `resolver = "2"` set explicitly. This root sets it.
- Integration tests are modules of one binary in `crates/<c>/tests/it/`. A new top-level `tests/*.rs` fails `tools/workspace-check.mjs`.

Sources: the Cargo book (rust-lang/cargo), section 8.
