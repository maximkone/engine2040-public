---
name: engine-code
description: "Use when you write Rust in an engine2040 crate: a module, crate, manifest, feature, dependency, public API, rustdoc, refactor or donor port. Reuse ladder, rules, areas."
---

# Write engine code

Other skills: `rust-safety`, `tick-and-runtime`, `test-and-proof`/`review`,
`github-check`/`commit-and-board`, `determinism-debug`.
Detail: `references/rules.md`, `references/workflow.md`, `references/areas.md`.

## Reuse first

Exists? Reuse, std, allowed installed crate, new code. `git grep` it first.
Core owners in `crates/core/src/lib.rs`: `Fnv64`, `Rng`, `time::IntAcc`, `Fixed`,
`det_math`, `checked`, `alloc_probe::measure`. Any crate may use `engine-core`.

## Before code

1. `git status`; keep changes you did not make.
2. `node tools/board-slice.mjs <ROW>`; a ticked row or existing function is a NOOP.
3. Read the crate API and tests, its `docs/ARCHITECTURE.md` section and `docs/CODELAW.md`;
   name consumer, data owner, units, determinism scope, error type.

## Workflow

1. Smallest complete change; a new crate only for a dependency, platform or ownership
   boundary. Test behavior and one failure, then wire a consumer; every new `pub` needs a
   caller or a boarded row.
2. `engine-core` std-only; IO/GPU/model/network outside the fixed tick; job order never
   decides sim state. Typed ids/errors; bounded buffers; validate input; no `unwrap` on
   user data.
3. `cargo check` while you work; prove with `node tools/gh-check.mjs` test and clippy;
   `rustfmt --edition 2021 --check <files>`, never `cargo fmt`. Sim changed: run the
   golden (AGENTS.md "Proof"); `--verify` prints `d57b5c8ae3096614`.

## Rules

- **Errors.** Typed enums with `Display`; `?` over `unwrap`; `expect` names the invariant.
- **Numbers.** `From`/`TryFrom`, not `as`; `total_cmp`; units in names; no `mul_add` in sim.
- **Hot paths.** No alloc, `format!`, lock or `dyn` per entity per tick; prove with
  `alloc_probe::measure`.
- **Clone.** A `.clone()` that clears a borrow error is a design bug: split the borrow, take
  the field, index by id, or `mem::take`. A deliberate clone states its reason in one line.
  Never clone sim state to dodge `&mut`.
- **Public shape.** A new `pub` derives `Debug, Clone, PartialEq`, plus `Copy, Eq, Hash,
  Default` where all are true; conversions `as_` (free), `to_` (copies), `into_` (consumes);
  the doc line names panics.
- **Shape.** Functions <100 lines, files <1,500; params struct over 7; `pub(crate)` unless
  needed; doc every `pub`.
- **Exceptions.** `#[expect(lint, reason = "...")]`, never `#[allow]`; `// det-ok:` and
  `// alloc-ok:` reviewed.
- **Comments.** Why, not history; no board ids or "was X".
- **Tests.** Each fails on a plausible defect; never restate a constant.
- **Ponytail.** Shortest diff; deferred work `ponytail: <path>`; never cut validation or tests.

## Area rules

`references/areas.md` holds the full text; the line that bites in each:

- Simulation (`docs/HASH-LAW.md`): fixed dt, `Rng::fork`, stable order, NaN guards; no map
  order, clock or pointer in hashed state; `+ - * /`, `sqrt`, `Fixed`, `det_math` only.
- Scene, editor, commands (`docs/EDITOR_AI_CONTROL.md`): ids, handles and digests differ;
  every edit through the command service; a stale revision writes nothing.
- Render (`docs/VISUAL-TARGET.md`): right-handed +Y up meters; explicit linear vs sRGB;
  resize, minimize, device loss; pixels never hashed. See `wgpu-render`.
- Audio and real time: the mix callback never allocates, locks, logs or blocks.
- AI (`docs/EDITOR_AI_CONTROL.md`): engine-owned types; providers are adapters; model
  output is a validated proposal; replay uses recorded actions. See `ai-door`.

Dependencies, docs, refactor and cargo: `references/dependencies.md`, `references/docs.md`,
`references/refactor.md`, `references/cargo-repo.md` (+ `cargo-cheatsheet`, `cargo-patterns`).
Additive features; profiles only in the root; `release` keeps `overflow-checks = true`; no
`Cargo.lock` edits; integration tests are modules of `crates/<c>/tests/it/`.
