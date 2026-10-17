# engine2040

Rust-first game engine aiming for Flax-class depth in third-person action
games — strong 3D visuals, a practical editor, physics, destruction,
multiplayer — with provider-agnostic AI in gameplay and authoring. The proof
game is a multiplayer Roman arena on Windows (`docs/FOCUS.md`).

Run: `cargo run -p engine2040 --release` prints the final hash.
Proof: `node tools/proof.mjs --skip-build` (hash x2) and `node tools/workspace-check.mjs`.

Crates follow explicit acyclic dependencies. Provider SDKs, GPU resources and
blocking IO stay outside `engine-core` and the authoritative tick. Live AI
proposes validated actions; replay consumes the recorded actions without models.

Project license: Apache-2.0. Dependency licenses follow `deny.toml` and provenance
policy; visual references and researched algorithms do not imply copied code.
