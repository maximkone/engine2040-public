# Changelog

One `## <version>` section per release. The newest heading is the workspace
version in `Cargo.toml`; `node tools/release.mjs --check` holds the two together
with `Cargo.lock`.

## 0.1.0

Start. The engine as it stands today, at workspace version 0.1.0.

- Rust workspace in `C:\engine2040`: a headless, deterministic fixed-step sim.
  Engine crates live in `crates/`; the root package `engine2040` runs the
  legacy null fixture.
- Golden hash `d57b5c8ae3096614` (seed 2040, 600 ticks; scope in
  `docs/HASH-LAW.md`). It moves only with an explicit receipt.
- Proof commands:
  - `cargo run -p engine2040 --release --quiet` prints
    `ticks=600 final_hash=d57b5c8ae3096614`.
  - `node tools/proof.mjs` runs the real-seed and recorded-input replay proofs
    (the golden itself is checked by the `release-proof` job in CI with
    `node tools/proof.mjs --skip-build`).
- Gates: `node tools/workspace-check.mjs`, `node tools/tools-index.mjs --check`,
  `node tools/docs-links.mjs`, `node tools/rust-quality.mjs`.
- An SBOM is written by `node tools/sbom.mjs` (`target/reports/sbom.cdx.json`);
  `node tools/release.mjs --tag` makes the local `v0.1.0` tag.
