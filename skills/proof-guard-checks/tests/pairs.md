# Pairs for proof-guard-checks (guard replies via checkBash, root C:/engine2040)

## Case 1: smith wants full proof on smith-tools-r14

- Without skill: `node tools/proof.mjs` as write seat.
- Without answer: `blocked: tools/proof.mjs builds and tests the whole workspace; golden: cargo run -p engine2040 --release (final_hash=d57b5c8ae3096614)`.
- With skill: `node tools/proof.mjs --skip-build` as write seat.
- With answer: `go` (no block; fixtures-only proof allowed per seat-guard.mjs:500).
- Winner: with (1-0).

## Case 2: planner wants the golden hash on planner-queue-r4

- Without skill: `cargo run -p engine2040 --release` as write seat.
- Without answer: `this seat never calls cargo: only coders, judges and the lead build; file a packet instead`.
- With skill: file a packet for the golden run; locally run `node tools/proof.mjs --skip-build`.
- With answer: `go` (no block; golden stays with code/read seats per seat-guard.mjs:491).
- Winner: with (2-0).

## Case 3: runner wants workspace tests on runner-proof-r1

- Without skill: `cargo test --workspace` as code seat.
- Without answer: `blocked: workspace-wide cargo; seats run one crate: cargo test -p <crate> --lib <filter>`.
- With skill: code seat runs `cargo test -p engine-proof --lib golden_hash_untouched`; run seat rebuilds with `cargo build --release -p engine-game --bin staged_fight`.
- With answer: `go` for both crate-scoped lines (per seat-guard.mjs:499 and :208).
- Winner: with (3-0).

Score: with-skill wins 3 of 3.
