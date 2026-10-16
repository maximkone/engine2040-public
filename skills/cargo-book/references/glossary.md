# cargo-book glossary

Short meanings as the Cargo book uses them. Quoted lines are the exact
command or output fragment the skill checks.

- manifest: the `Cargo.toml` file of one package (`[package]` name plus
  version); written by `cargo new`, read by every build.
- lockfile: `Cargo.lock`, written by Cargo, pinning the exact revision of
  every dependency so rebuilds share the same SHA.
- feature: a named additive option in `[features]`; users select it with
  `--features extra`, and Cargo builds the union once as a single copy.
- profile: a named compiler preset (`dev`, `release`, `test`, `bench`);
  `dev` prints `[unoptimized + debuginfo]`, `release` prints `[optimized]`.
- workspace: several packages built together; `cargo build --workspace`
  prints one `Compiling` line per member and shares the root lockfile.
- virtual manifest: a `[workspace]` manifest with `members` but without a
  `[package]` section, with `resolver = "2"` set explicitly.
- target: one build output (binary, library, example, bench, test);
  `src/main.rs` is found by auto-discovery unless `autobins = false`.
- config: `.cargo/config.toml` files from the build directory outward to
  `$HOME/.cargo/config.toml`; the deeper file wins, home is lowest priority.
- resolver: the version (`"2"`) choosing how features unify and how
  dependencies resolve inside a workspace.
- target-dir: the `[build] target-dir` config value redirecting all build
  artifacts into a custom directory.
