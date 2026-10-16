# cargo-book cheatsheet

One page per area. Every line below is replayed by `scripts/cargo_book.py`
against real `cargo` runs; see `references/pairs.md` for the pair each line
comes from.

## Lockfile

- `cargo new hello_world` then `cargo run` prints `Finished` and
  `Hello, world!`, exit 0. (cb-p01)
- `Cargo.toml` is loose (a git URL with no revision tracks the default
  branch tip); `Cargo.lock` pins the exact revision so the next checkout
  builds the same SHA. (cb-p02)
- `cargo update` refreshes every entry; `cargo update <name>` refreshes one.
  Never hand-edit the lockfile. (cb-p02)
- Unknown lock names fail: `did not match any packages`, exit 101. (cb-p02)

## Features

- Shared crates build once from the union of requested features; watch for a
  single `Compiling shared` line. (cb-p11)
- Features must be additive: no feature may switch other behavior off.
  (cb-p06)
- `cargo run -q` prints `basic`; `cargo run -q --features extra` prints
  `extra on`. (cb-p06)
- Union, additive, flag in one rule: shared crates build once from the
  union of requested features; every feature stays additive;
  `--features extra` selects one. (cb-p06, cb-p11)
- Unknown features fail: `does not contain this feature`, exit 101. (cb-p06)

## Profiles

- `dev` (default), `release` (`--release`), `test` (`cargo test`), `bench`
  (`cargo bench`). (cb-p04)
- Only the workspace root manifest sets `[profile.*]`; dependency settings are
  ignored; `[config]` or environment values override the manifest. (cb-p09)
- dev: `opt-level = 0`, `debug = true`, `debug-assertions` on,
  `overflow-checks` on; prints `[unoptimized + debuginfo]`. (cb-p09)
- release: `opt-level = 3`, `debug = false`, both checks off; prints
  `[optimized]` into `target/release/`. (cb-p04)
- Unknown profiles fail: ``profile `nosuch` is not defined``, exit 101.
  (cb-p04)
- Defaults side by side: dev uses `opt-level = 0`, release uses
  `opt-level = 3`. (cb-p04, cb-p09)

## Workspaces

- Virtual manifest: `[workspace]` plus `members`, no `[package]`, explicit
  `resolver = "2"`. (cb-p05)
- `cargo build --workspace` prints `Compiling a` and `Compiling b`; one
  lockfile lives at the root. (cb-p05)
- Missing members fail: `failed to read` plus the member path, exit 101.
  (cb-p05)
- Member `.cargo/config.toml` files are not read. (cb-p12)

## Config

- Order outward from the build directory: `./.cargo/config.toml`, each
  parent, then `$HOME/.cargo/config.toml` on Unix; deeper wins, home is the
  lowest priority. (cb-p08)
- `[build] target-dir` in the project config moves all artifacts there.
  (cb-p08)
- Broken `--config` TOML fails before compiling, exit 101. (cb-p08)

## Targets and tests

- `src/main.rs`, `src/lib.rs`, `src/bin/`, `examples/`, `benches/`,
  `tests/` are found without declarations; `autobins`, `autoexamples`,
  `autotests`, `autobenches` turn kinds off. (cb-p07)
- `autobins = false` with no `[[bin]]` fails: `no targets specified in the
  manifest`, exit 101. (cb-p07)
- Failing assert prints `test result: FAILED. 0 passed; 1 failed`, exit 101;
  fix the expected value (`5` to `4`). (cb-p03)
- No manifest nearby fails: `could not find` plus `Cargo.toml`, exit 101.
  (cb-p10)
