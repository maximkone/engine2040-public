---
name: proof-guard-checks
description: "Use when a seat guard blocks tools/proof.mjs, cargo run -p engine2040 or workspace cargo in engine2040: run the allowed crate-scoped proof instead and file a packet for the rest."
---

# Proof guard checks

Top loop failure (26x in last 300 keeper lines): `blocked: tools/proof.mjs builds...` (.opencode/seat-guard.mjs:500). Write/run/research seats never run workspace builds. Stay in pwsh at repo root.

## Allowed proof per seat

- Write (smith/miner/skill-hunter/planner): never cargo (seat-guard.mjs:491). Fixtures only: `node tools/proof.mjs --skip-build`. Golden stays with coders/judges.
- Run (runner/perf): only the `rebuild:` line from `node tools/bins.mjs` (seat-guard.mjs:208): `cargo build --release -p engine-game --bin staged_fight`, nothing chained. Then use the built binary.
- Code/read (builder/judge/lead): one crate: `cargo check -p <crate>`, then `cargo test -p <crate> --lib <filter>` (seat-guard.mjs:499). Example: `cargo test -p engine-proof --lib golden_hash_untouched`.

## Never run

- `node tools/proof.mjs` without `--skip-build` (blocked:500).
- `cargo run -p engine2040 --release` on write/run/research seats (blocked:491).
- `cargo ... --workspace|--all` (blocked:499). `cargo fmt` without `--check`: use `rustfmt --edition 2021 <paths>` (blocked:496).
- `node tools/mutants.mjs`: lead only (blocked:501). `cargo clean`, `git stash`, `git add -A`: never (seat-guard.mjs:185-199).

## Before RESULT

- Report command, exit code, output path. No owned paths means `RESULT: BLOCKED - no-owned-paths <id>`, no retry.
