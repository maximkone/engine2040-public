---
name: engine-code
description: Use when writing or reviewing engine2040 Rust code - crates, Cargo manifests, tests, determinism, wgpu rendering, scene/editor/command code, gameplay AI, AI provider adapters, spatial checks or adding a dependency.
---

# Write engine code

Read the affected crate's public API and tests, and grep the crate's section of `docs/ARCHITECTURE.md` (not the whole file). Also read
and `docs/CODELAW.md`. Run `git status` first and preserve changes you did not
make.

Before any code: `node tools/board-slice.mjs <ROW>`. A ticked row, or a named function that already exists, means NOOP: say so and stop.

## Workflow

1. Name the consumer, data owner, units, determinism scope and error type.
2. Smallest complete change. Ladder: does it need to exist → reuse existing
   code → std → an allowed installed crate → new code. Prefer a module; add a
   crate only for a real dependency, platform or ownership boundary.
3. Typed IDs and errors, bounded buffers, validated external input (sizes,
   ranges, NaN, versions). No `unwrap` on IO or user data.
4. `engine-core` stays std-only. IO, GPU, model and network work stays out of
   the fixed tick; job completion order never decides simulation state.
5. Test the behavior and at least one failure case, then wire the real
   consumer. A trait that compiles is not integration. Before reporting
   DONE, `git grep` every new pub name: a caller outside tests must exist.
   Zero callers outside tests means stop — wire one real caller (tick,
   submit, encoder) or board its consumer row, with a behavior test proving
   the call path. Never resend the module whole; send one fix one-off.
   Fail: `reflect_screen_for_forward` tested, never called by submit (RT19
   x2); `apply_poise_kind` callers only tests (GP30 x2). Pass: arena hit
   pipeline reads `capture_bias` after each landed blow.
   Finish the capability in one packet; if it cannot land whole, report
   exactly what remains instead of a partial.
6. Run `rustfmt --edition 2021 --check <owned .rs>` (fix by hand, never `cargo fmt`), `cargo test -p <crate> --lib <filter>` (the filter names your new tests) and
   `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`. In a
   wave, edit only the packet's owned paths in `C:\engine2040` (one tree,
   master) and build only your crate; a "Blocking waiting for file lock" line
   is another cargo build in `target/`. Never commit. Golden check:
   `cargo run -p engine2040 --release` prints `final_hash=d57b5c8ae3096614`.
7. Obey the slim law in `docs/CODELAW.md`: one owner per type, a consumer
   outside tests, a stated budget, no steady-state allocation per tick.
8. Self-review per the canonical bar in `skills/engine-rust-quality/SKILL.md`
   (`docs/RUST-QUALITY.md` background), and paste its RESULT lines into the
   report. Then reread your diff as its reviewer: would a top engine team merge it?

## Top-tier checklist (every new line)

- **Reuse first:** `git grep` the behavior before writing it. Hashing is
  `engine_core::{Fnv64, hash_bytes, splitmix64}`, zero-alloc proofs use
  `engine_core::alloc_probe::measure` (dev-dependency feature `test-util`).
  Any crate may depend on `engine-core`.
- **Errors:** typed enums with `Display`; `?` over `unwrap`; `expect` names
  the proven invariant; panics only for bugs. Binaries may add context.
- **Numbers:** `From`/`TryFrom` or a named, tested conversion instead of
  `as`; `total_cmp` for float order; units in names or newtypes.
- **Hot paths:** no allocation, `format!`, lock or `dyn` per entity per
  tick or frame; buffers owned by the system and reused; slices in, slices
  out; `sort_unstable` for keys; prove it with `alloc_probe::measure`.
- **Shape:** functions under 100 lines, files under 1,500; a params struct
  over 7 arguments; no free function that only forwards; `pub(crate)` unless
  a consumer needs it; docs on every `pub` item (units, errors, determinism).
- **Exceptions:** canonical bar in `skills/engine-rust-quality/SKILL.md` (`#[expect]` only, never `#[allow]`);
  `#[ignore = "<why>"]`; `// det-ok:` / `// alloc-ok:` waivers are reviewed.
- **Comments:** why, not history: no board IDs, wave numbers or "was X";
  the board and git carry history.
- **Tests:** each fails on a plausible defect (oracle, negative case,
  property over seeds), never restates a constant.

When the user says "ponytail": shortest diff that works, one check, and mark
deferred work `ponytail: <path>`. Never cut validation, determinism or tests.

## Adding a dependency

A new Cargo dependency's license must pass `deny.toml`. Donor source code from
any repo may be ported directly (owner 2026-10-10); no EVIDENCE row. Prefer small crates without C toolchains and put heavy ones
behind a feature. Do not edit a manifest yourself: report the dependency, and the lead adds it with its `docs/DEPENDENCIES.md` row and `cargo deny check`.

## Area rules

**Simulation** (`docs/HASH-LAW.md`): fixed dt, explicit seeded streams
(`Rng::fork`), stable iteration order, NaN guards. No `HashMap` order, wall
clock or pointer values in hashed state. New fixtures get their own version
and a non-aliasing seed expansion. Rollback multiplayer needs identical
results on every player's PC: simulation code uses only `+ - * /`, `sqrt`,
`Fixed` or deterministic math, never `sin`, `exp`, `powf` and friends.

**Scene, editor, commands** (`docs/EDITOR_AI_CONTROL.md`): persistent IDs,
generational handles and content digests are different things. One schema
feeds serializer, inspector and AI tools. Every edit, whether from inspector,
gizmo, script or AI, goes through the command service: validate → preview
without side effects → atomic apply → journal → undo. A stale revision or bad
ID writes nothing. Play runs in a child process.

**Render** (`docs/VISUAL-TARGET.md`): work stage by stage, V1 to V5.
Right-handed, +Y up, meters; explicit linear vs sRGB; handle resize,
minimize and device loss. Pixels never enter the simulation hash.

**Audio and real time:** the mix callback never allocates, locks, logs or
blocks; prove it with `alloc_probe::measure` over the callback body.

**AI** (`docs/EDITOR_AI_CONTROL.md`, `docs/RESEARCH-VERDICT.md`): engine-owned request,
result and capability types; providers are adapters (mock, local, remote,
ONNX later) with deadlines, cancellation, bounded queues and offline fallback.
Model output is a proposal: validate schema, ranges, references and revision,
run `engine-spatial` checks, then apply through the command service. Models
give labels and intent; deterministic code owns geometry. Replay uses recorded
accepted actions and never calls a model.

## Report

If simulation code changed, run `cargo test -p engine-proof --lib golden_hash_untouched` and report it. `-p` is the package name in `crates/<dir>/Cargo.toml`.

Changed paths, commands with results, the legacy hash when simulation code
changed, changes the orchestrator must apply, and what is still unverified.
