---
name: engine-code
description: "Use when you write Rust in an engine2040 crate: a module, crate, manifest, feature, dependency, public API, rustdoc, refactor or donor port. Reuse ladder, rules, areas."
---

# Write engine code

Other skills: `rust-safety`, `tick-and-runtime`,
`test-and-proof`/`review`, `github-check`/`commit-and-board`, `determinism-debug`.

## Reuse first

Exists? Reuse, std, allowed installed crate, new code. `git grep` it first.
Core owners in `crates/core/src/lib.rs`: `Fnv64`, `Rng`, `time::IntAcc`, `Fixed`,
`det_math`, `checked`, `alloc_probe::measure`. Any crate may use `engine-core`.

## Before code

1. `git status`; keep changes you did not make.
2. `node tools/board-slice.mjs <ROW>`; a ticked row or existing function is a NOOP.
3. Read the crate API and tests, its `docs/ARCHITECTURE.md` section and
   `docs/CODELAW.md`; name consumer, data owner, units, determinism scope, error type.

## Workflow

1. Smallest complete change; new crate only for a dependency, platform or ownership
   boundary. Test behavior and one failure, then wire a consumer; every new `pub` needs
   a caller or a boarded row.
2. `engine-core` std-only; IO/GPU/model/network outside the fixed tick; job order never
   decides sim state. Typed ids/errors; bounded buffers; validate input; no `unwrap` on
   user data.
3. `cargo check` while you work; prove with `node tools/gh-check.mjs` test and
   clippy; `rustfmt --edition 2021 --check <files>`, never `cargo fmt`. Sim changed: run
   the golden (AGENTS.md "Proof"); `--verify` prints `d57b5c8ae3096614`.

`references/workflow.md`.
## Rules

- **Errors.** Typed enums with `Display`; `?` over `unwrap`; `expect` names the invariant.
- **Numbers.** `From`/`TryFrom`, not `as`; `total_cmp`; units in names; no `mul_add` in
  sim.
- **Hot paths.** No alloc, `format!`, lock or `dyn` per entity per tick; prove with
  `alloc_probe::measure`.
- **Shape.** Functions <100 lines, files <1,500; params struct over 7;
  `pub(crate)` unless needed; doc every `pub`.
- **Exceptions.** `#[expect(lint, reason = "...")]`, never `#[allow]`; `// det-ok:` and
  `// alloc-ok:` reviewed.
- **Comments.** Why, not history; no board ids or "was X".
- **Tests.** Each fails on a plausible defect; never restate a constant.
- **Ponytail.** Shortest diff; deferred work `ponytail: <path>`; never cut validation
  or tests.

`references/rules.md`.

## Area rules

- **Simulation** (`docs/HASH-LAW.md`): fixed dt, `Rng::fork`, stable order, NaN guards;
  no map order, clock or pointer in hashed state; deterministic math only.
- **Scene, editor, commands** (`docs/EDITOR_AI_CONTROL.md`): ids, handles and digests
  differ; every edit through the command service; a stale revision writes nothing.
- **Render** (`docs/VISUAL-TARGET.md`): V1 to V5, right-handed +Y up meters; linear vs
  sRGB; resize, minimize, device loss; pixels never hashed.
- **Audio and real time:** the mix callback never allocates, locks, logs or blocks.
- **AI** (`docs/EDITOR_AI_CONTROL.md`): engine-owned types; providers are adapters; model
  output is a validated proposal; replay uses recorded actions.

`references/areas.md`.

## More rules

- **Dependencies.** `deny.toml`, a row in `docs/DEPENDENCIES.md`, `workspace = true`,
  heavy ones behind a feature; donor code allowed; no hand-rolled crypto.
  `references/dependencies.md`.
- **Docs.** For a stranger and an AI calling MCP; every sentence true of the code;
  name the row that finishes the rest. `references/docs.md`.
- **Cross-crate refactor.** Crates in play, out of scope, batches, one CI and rollback line per
  batch; order security, breaking bugs, architecture, performance, style; one at a time.
  `references/refactor.md`.
- **Cargo.** Features additive; profiles only in the root; `release` keeps
  `overflow-checks = true`; nearer `.cargo/config.toml` wins; no `Cargo.lock` edits;
  integration tests are modules of `crates/<c>/tests/it/`.
  `references/cargo-repo.md` (+ `cargo-cheatsheet`, `cargo-patterns`).
