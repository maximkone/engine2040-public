# Sources and licences (read 2026-10-06)

The skill text and every example are written fresh. The Cargo book files
below were read to check each rule. Only short command lines and output
fragments are quoted, each with its file.

- The Cargo Book, rust-lang/cargo, commit bb2126cffae48394a37db8728dc50a17bbfe54d4 on master (pushed 2026-10-03), read 2026-10-06 from a local download in `work/cargo-book/src/` (git-ignored, never committed). Upstream https://github.com/rust-lang/cargo, verified live 2026-10-06.
  - Licence: Apache-2.0 (LICENSE-MIT and LICENSE-APACHE in the repo root; API record carries the Apache-2.0 SPDX, re-read live 2026-10-06 via gh api). Permissive: no NonCommercial or ShareAlike restriction, attribution via the credit line below. Commit bb2126cf re-read live 2026-10-06 (pushed 2026-10-03); all 12 file SHAs rechecked 2026-10-06 against the board pins.
  - Files used: getting-started-first-steps.md, guide-cargo-toml-vs-cargo-lock.md, guide-creating-a-new-project.md, guide-dependencies.md, guide-project-layout.md, guide-tests.md, guide-working-on-an-existing-project.md, reference-config.md, reference-features.md, reference-manifest.md, reference-profiles.md, reference-workspaces.md.
  - Compiler outputs quoted in patterns.md and pairs.md were measured on this PC with cargo 1.97.1 and rustc 1.97.1 on 2026-10-06, not copied from the book.
- The trial sheet `evals/cargo-book_trials.jsonl` (12 tasks) pins each task to its book section via its `locator` field at commit bb2126cf.
- This skill is original work under Apache-2.0. It is free to use and share.

Chunk manifest for `distill check` without `--work`: no `NNNN.txt` chunk refs are used; every SKILL.md rule is covered by the 12 trial rows above (`evals/cargo-book_trials.jsonl`).

Credit line for THIRD_PARTY_NOTICES.md: cargo-book (Apache-2.0 Cargo book, rust-lang/cargo pinned bb2126cf read 2026-10-06; own words and own examples, short quotes only).
