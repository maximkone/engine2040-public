---
name: cargo-book
description: "Use when configuring Cargo manifests, features, profiles, workspaces, or config for Rust builds: lockfile rules, feature unification, profile defaults, virtual workspaces, and config hierarchy with the exact commands to run."
version: 0.1.0
author: skillworks
license: Apache-2.0
---

# Cargo builds from the book

Rules distilled from the Cargo book (rust-lang/cargo, bb2126cf) for Rust builds: what Cargo.toml declares, what Cargo.lock pins, which features unify, which profile compiles, which workspace member builds, and which config file wins. Each rule names the exact command and the output fragment that proves it. Detail lives in `references/cheatsheet.md`, `references/glossary.md`, and `references/patterns.md`. The runnable proof of every rule is `references/pairs.md`, replayed by `scripts/cargo_book.py`.

## Lockfile pins the exact build

- `cargo new hello_world` writes a manifest with `[package]` name, version, and edition plus `src/main.rs` printing `Hello, world!`; `cargo run` then prints `Finished` and `Hello, world!` with exit 0 [src: references/pairs.md#cb-p01]
- `Cargo.toml` describes dependencies loosely (a git URL with no revision means the latest commit on the default branch); the first build writes `Cargo.lock` pinning the exact revision, so a second checkout builds the same SHA [src: references/pairs.md#cb-p02]
- Opt into newer libraries with `cargo update` (every entry) or `cargo update regex` (one entry); never hand-edit `Cargo.lock`, and never delete the manifest to upgrade [src: references/pairs.md#cb-p02]
- Updating a name no entry carries fails with `did not match any packages` and exit 101 [src: references/pairs.md#cb-p02]

## Features unify to a single copy

- When two crates enable different features of one shared crate, Cargo builds the union of all requested features into a single copy: one `Compiling shared` line serves both users [src: references/pairs.md#cb-p11]
- Every feature must be additive: enabling a feature must not disable functionality, and any combination of features must stay safe to enable [src: references/pairs.md#cb-p06]
- Select features on the command line with `--features extra`: a plain `cargo run -q` prints `basic` while `cargo run -q --features extra` prints `extra on` [src: references/pairs.md#cb-p06]
- Naming a feature the package does not contain fails with `does not contain this feature` and exit 101 [src: references/pairs.md#cb-p06]

## Profiles: dev by default, release on request

- The four built-in profiles are `dev`, `release`, `test`, and `bench`: plain `cargo build` uses `dev`, `cargo build --release` uses `release`, `cargo test` uses `test`, `cargo bench` uses `bench` [src: references/pairs.md#cb-p04]
- Cargo reads `[profile.*]` only from the manifest at the root of the workspace; profile settings written in dependencies are ignored, while a profile definition from `[config]` or an environment variable overrides the manifest [src: references/pairs.md#cb-p09]
- The `dev` defaults are `opt-level = 0`, `debug = true`, `debug-assertions` on, and `overflow-checks` on; the build prints ``Finished `dev` profile [unoptimized + debuginfo]`` [src: references/pairs.md#cb-p09]
- The `release` defaults are `opt-level = 3`, `debug = false`, `debug-assertions` off, and `overflow-checks` off; the build prints ``Finished `release` profile [optimized]`` and writes `target/release/` [src: references/pairs.md#cb-p04]
- Naming a profile that is not defined fails with ``profile `nosuch` is not defined`` and exit 101 [src: references/pairs.md#cb-p04]

## Workspaces share one lockfile

- A virtual manifest holds `[workspace]` with `members` but without a `[package]` section; use it when no single package is primary, and set `resolver = "2"` explicitly since no edition implies it [src: references/pairs.md#cb-p05]
- `cargo build --workspace` prints `Compiling a` and `Compiling b` and keeps the shared lockfile at the workspace root [src: references/pairs.md#cb-p05]
- A member that is missing fails the whole build with `failed to read` naming the member manifest and exit 101 [src: references/pairs.md#cb-p05]
- Member-level config files inside a workspace are not read; the root config decides [src: references/pairs.md#cb-p12]

## Config: nearer files win, home is last

- Starting from the build directory Cargo reads `.cargo/config.toml` in that directory and in every parent, then `$HOME/.cargo/config.toml` on Unix; keys set in several files merge with the deeper directory winning and the home directory at the lowest priority [src: references/pairs.md#cb-p08]
- A project `.cargo/config.toml` carrying `[build] target-dir` redirects artifacts there: the binary lands under the custom directory [src: references/pairs.md#cb-p08]
- A `--config` value that is not valid TOML fails before any compile with exit 101 [src: references/pairs.md#cb-p08]

## Targets are automatic until disabled

- `src/main.rs` is a binary, `src/lib.rs` a library, `src/bin/` holds more binaries, and `examples/`, `benches/`, `tests/` follow the same convention: no declaration needed [src: references/pairs.md#cb-p07]
- The four keys `autobins`, `autoexamples`, `autotests`, and `autobenches` switch auto-discovery off per kind [src: references/pairs.md#cb-p07]
- With `autobins = false` and no explicit target, `src/main.rs` is ignored and the manifest fails with `no targets specified in the manifest` and exit 101 [src: references/pairs.md#cb-p07]

## Failing tests and empty folders

- A unit test asserting `assert_eq!(add(2, 2), 5)` fails as `test result: FAILED. 0 passed; 1 failed` with exit 101; the one-line fix is `assert_eq!(add(2, 2), 4)` [src: references/pairs.md#cb-p03]
- Building or running where no manifest exists fails with `could not find` naming `Cargo.toml` and exit 101 [src: references/pairs.md#cb-p10]
- Every manifest carries its `[package]` identity (`name` plus `version`); `cargo new` writes it and the build reads it [src: references/pairs.md#cb-p10]
