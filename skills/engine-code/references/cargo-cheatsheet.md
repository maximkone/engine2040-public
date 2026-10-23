# Cargo cheatsheet

Generic Cargo behavior (Cargo book). Repo specifics are in `SKILL.md` Cargo rule.

## Lockfile

- `cargo new hello_world` then `cargo run` prints `Finished` and `Hello, world!`, exit 0.
- `Cargo.toml` is loose (a git URL with no revision tracks the default branch tip). `Cargo.lock` pins the exact revision, so the next checkout builds the same SHA.
- `cargo update` refreshes every entry. `cargo update <name>` refreshes one. Never hand-edit the lockfile.
- An unknown lock name fails: `did not match any packages`, exit 101.

## Features

- Shared crates build once from the union of requested features. Look for one `Compiling shared` line.
- Features must be additive. No feature may switch other behavior off.
- `cargo run -q` prints `basic`. `cargo run -q --features extra` prints `extra on`.
- An unknown feature fails: `does not contain this feature`, exit 101.

## Profiles

- `dev` (default), `release` (`--release`), `test` (`cargo test`), `bench` (`cargo bench`).
- Only the workspace root manifest sets `[profile.*]`. Dependency settings are ignored. `[config]` or environment values override the manifest.
- Cargo default dev: `opt-level = 0`, `debug = true`, `debug-assertions` on, `overflow-checks` on. Prints `[unoptimized + debuginfo]`.
- Cargo default release: `opt-level = 3`, `debug = false`, both checks off. Prints `[optimized]` into `target/release/`.
- An unknown profile fails: ``profile `nosuch` is not defined``, exit 101.
- A custom profile needs `inherits`.

## Workspaces

- Virtual manifest: `[workspace]` plus `members`, no `[package]`, explicit `resolver = "2"`.
- `cargo build --workspace` builds every member. One lockfile lives at the root.
- A missing member fails: `failed to read` plus the member path, exit 101.
- Member `.cargo/config.toml` files are not read.

## Config

- Order outward from the build directory: `./.cargo/config.toml`, each parent, then `$HOME/.cargo/config.toml`. Deeper wins. Home is the lowest priority.
- `[build] target-dir` in the project config moves all artifacts.
- Broken `--config` TOML fails before compiling, exit 101.

## Targets and tests

- `src/main.rs`, `src/lib.rs`, `src/bin/`, `examples/`, `benches/`, `tests/` are found without declarations. `autobins`, `autoexamples`, `autotests`, `autobenches` turn kinds off.
- `autobins = false` with no `[[bin]]` fails: `no targets specified in the manifest`, exit 101.
- A failing assert prints `test result: FAILED. 0 passed; 1 failed`, exit 101.
- No manifest nearby fails: `could not find` plus `Cargo.toml`, exit 101.
