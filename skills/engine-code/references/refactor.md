# Cross-crate refactor

Full text behind the refactor rule in `SKILL.md` More rules.

Before the first edit, write down:

1. The crates in play, and what is out of scope (generated files, vendor drops,
   `Cargo.lock`, local cargo config).
2. Batches. Each names its crates and files. No touched crate is missing.
3. One CI line and one rollback line per batch, for example
   `node tools/gh-check.mjs cargo test -p engine-net` and "revert `quic.rs`".
4. Order: security, breaking bugs, architecture, performance, style last.

Land one batch at a time. Restate the out-of-scope list at each batch.
