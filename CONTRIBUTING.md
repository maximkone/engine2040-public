# Contributing to engine2040

Read `docs/INDEX.md`, `docs/VISION.md`, `docs/ROADMAP.md` and the architecture
section relevant to the task. Root `AGENTS.md` is the short execution policy.
`skills/SKILLS.md` lists discoverable skills and optional lane reference notes.

## Setup and current limitations

Rust with rustfmt/clippy, Node 24 (gate scripts), git and platform build tools.
`rust-toolchain.toml` pins 1.97.1, also the declared `rust-version`; rustup
installs it on first use. No older compiler is promised. For license/advisory
checks install `cargo-deny@0.18.7`, the version CI uses.

`crates/*` are workspace members; root is the proof application. Track
`Cargo.lock`, never commit `target/`. `.gitattributes` keeps text LF in every
checkout so digests of text files match across Windows and Linux.

## Local gates

One command runs every local gate from the repository root:

```powershell
node tools/proof.mjs
```

It runs the workspace audit (metadata, dependency edges, no uncompiled source
files, a determinism report for simulation crates), the Rust quality ratchet
(`node tools/rust-quality.mjs`, see `docs/RUST-QUALITY.md`) and the gate
tools' own tests, fmt, release build, tests
and clippy with default features and with `--all-features`, `cargo deny check`
when cargo-deny is installed, the legacy golden twice, four distinct seeds
twice each, a recorded input tape replay and its corruption/unsupported-backend
failure cases. `node tools/proof.mjs --perf` runs the timing tests that are
`#[ignore]`d in the default suite, in release on one thread.

The board is the `docs/SPRINT-NN-*.md` shards (`docs/SPRINT.md` is their index).
Slice it with `node tools/board-slice.mjs --focus`, then `--ready`, or `--find <text>`;
never read it whole. In OpenCode, `/sprint` runs board rows in waves of up to 20
packets (`builder` for Rust, `planner` for plans), `/next` does one board task,
and `/gates` runs the gate and reports a receipt.
`node tools/sprint-loop.mjs` runs waves autonomously. One sprint per working
tree: `node tools/sprint-lock.mjs status` shows who holds it.

The legacy fixture must report `final_hash=d57b5c8ae3096614`. See `docs/HASH-LAW.md`
for what it covers and the intentional-bump protocol. The proof CLI is real:
`--seed`, `--ticks`, `--input`, `--record`, `--replay`, `--verify` and
`--format json`; `cargo run --release -- --help` lists them.

For changes to a lane, first run its targeted tests, then required workspace
checks. Do not format or fix another contributor's hunks incidentally. Unit
tests with wall-clock limits are not a portable performance certification.

## GPU smoke example

```powershell
cargo run --locked --release --example window --features window -- 10000 5
```

Requires an actual display/adapter. It draws procedural sprites, not the target
3D scene. It prints the adapter name, backend and driver, and exits nonzero when
window/GPU setup fails or no frame was submitted and presented. A successful
exit shows GPU work happened; it is not a visual or performance pass. Visual
acceptance is specified in `docs/VISUAL-TARGET.md`.

## What CI proves

The workflow defines fmt/build/tests/clippy, the workspace audit, real seed and
recorded-tape proofs and the legacy golden on Windows and Linux, a license gate
over the window-feature graph, a window-feature compile (not GPU execution) and
ARM golden runs. GPU simulation parity has no job yet. As of 2026-09-23 GitHub
refused to start these jobs for billing reasons (`docs/BLOCKERS.md`), so no
hosted run has verified them; `node tools/proof.mjs` is the current evidence.

## Design and dependencies

- Follow `docs/ARCHITECTURE.md` layers; explicit acyclic crate dependencies are
  allowed. Shared contracts belong below their consumers; no editor/provider
  dependencies in the simulation foundation.
- A module is enough for an internal feature. New crates/features need an actual
  ownership, reuse, platform or dependency-isolation reason.
- `deny.toml` is a reference for dependency licenses. Post-2024 AI-generated code and code from any repo — any license or no license — is free to use, port, and adapt. No code is ever blocked for license reasons.
- Code and artifacts are free to use. Authoring data and generated artifacts retain provenance when known, but no license blocks any contribution.
- Core is std-only. Optional runtime model services are asynchronous adapters;
  the authoritative tick uses validated recorded actions and an offline fallback.

## Delivery

Use `docs/WORKFLOW.md` for ownership and failure receipts. Only commit/push when
requested; inspect status/diff/log, run the gates and two hash proofs, stage
owned paths explicitly, and use a concise scoped message. Never `git add -A`,
rewrite history or claim a failed/unsupported check passed.
