# Docs

Full text behind the docs rule in `SKILL.md` More rules.

Readers are a stranger building a game and an AI calling the engine through MCP. Write
for both.

- Every sentence is true of the code on disk. Never document a planned or `Unsupported`
  path as working. Say what it does today and name the board row that finishes it.
- Public items: one-line summary, then units (meters, ticks, permille), ownership,
  threading, determinism scope, errors and panics. Match the module header style
  (`crates/arena/src/fork.rs` is the model).
- Every public entry point gets a short runnable example (doctest or `examples/`).
  `node tools/gh-check.mjs cargo test --doc -p <crate>` and
  `cargo run -p <crate> --example <name>`.
- The mdBook in `docs/book/` holds guides: quick start, one page per system, recipes.
  Link to rustdoc. Never copy signatures by hand.
- Coverage: raise `#![warn(missing_docs)]` crate by crate. Report
  `cargo doc --no-deps` warnings before and after. `broken_intra_doc_links` is a
  workspace rustdoc lint.
