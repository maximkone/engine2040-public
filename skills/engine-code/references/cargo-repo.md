# Cargo facts that bit this repo

Full text behind the cargo rule in `SKILL.md` More rules. Generic behavior:
`references/cargo-cheatsheet.md`; failure text and its fix: `references/cargo-patterns.md`.

- Features unify: a shared crate builds once from the union of requested features.
  Every feature must be additive.
- `[profile.*]` is read only from the workspace root manifest. Dependency profiles are
  ignored. The root has `dev` (opt-level 1, dependencies 3, 13 sim crates 2),
  `release`, `profiling`, `ship`, `web`. `.cargo/config.toml` adds `fast-test` and sets
  `test` to `debug = 0`.
- `release` here sets `overflow-checks = true`. Cargo's default is off. Sim code panics
  instead of wrapping.
- The nearer `.cargo/config.toml` wins. Member config files in a workspace are not
  read.
- `Cargo.lock` pins revisions. Never hand-edit it. `cargo update -p <name>` refreshes
  one entry.
- Targets are auto-discovered (`src/bin/`, `examples/`, `tests/`). `required-features`
  hides a bin: `staged_fight` and `arena` need `--features app`.
- Root `Cargo.toml` is a package plus workspace with `default-members = ["."]`. A bare
  `cargo build` builds only the root. Use `-p <crate>` or `--workspace`.
- A virtual manifest (no `[package]`) needs `resolver = "2"` set explicitly. This root
  sets it.
- Integration tests are modules of one binary in `crates/<c>/tests/it/`. A new
  top-level `tests/*.rs` fails `tools/workspace-check.mjs`.

Sources: the Cargo book (rust-lang/cargo).
