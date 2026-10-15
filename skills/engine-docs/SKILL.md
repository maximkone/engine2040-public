---
name: engine-docs
description: Use when writing engine2040 docs - rustdoc on public items, runnable examples, mdBook guides and recipes, missing_docs coverage. Loaded by the builder for DOCS packets.
---

# Engine docs (from builder)

Docs are read by a stranger building a game and by an AI agent calling the engine through MCP. Write for both.

- **Read the code first.** Every sentence true of the code on disk. Never document a planned or `Unsupported` path as working; say what it does today and name the board row that finishes it.
- **Public items:** one-line summary, then units (meters, ticks, permille), ownership, threading, determinism scope, error cases and panics. Match the module header style already used (`crates/arena/src/fork.rs` is the model).
- **Examples:** every public entry point gets a short runnable example (doctest or `examples/`). It must compile and run: `cargo test --doc -p <crate>` and `cargo run --example <name>` (`-p <crate>`).
- **Manual:** the mdBook under `docs/book/` holds guides: quick start, one page per system, recipes ("add a weapon", "new map", "drive it from MCP"). Link to rustdoc, never copy signatures by hand.
- **Coverage:** raise `#![warn(missing_docs)]` crate by crate as the packet says; report `cargo doc --no-deps` warnings before and after.
- Research how the best engine docs teach (Bevy book, Godot docs, Unreal API pages) with websearch; steal structure, never text.

Return changed paths, commands with real output, coverage numbers before/after, and what is still undocumented.
