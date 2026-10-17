# tools/
Node scripts for repo checks and sprint plumbing. No game or engine code lives here.
Start with `node tools/doctor.mjs --help` for repo health first.
Stranger picks (all take --help, exit 0):
- `node tools/board-slice.mjs --help` to read one board row.
- `node tools/workspace-check.mjs --help` to audit after board edits.
- `node tools/crate-map.mjs --help` to map crates to dirs.
Each script has a `*.test.mjs` mate next to it; a few audit `*.test.mjs` files have no live script (listed below).
Helpers live in `tools/vendor/`; retired ones live in `docs/archive/`.
Keep scripts small and portable; docs stay in `docs/`.

## Live tools

76 live scripts. 81 dead tools (39 A-L plus 42 M-Z) were deleted 2026-10-10 and are not listed. Short lines; the script header is the full doc. The list is pinned by `tools/drift-check.test.mjs` (README lists every live `tools/*.mjs`).

- `tools/affected.mjs` (+ `tools/affected.test.mjs`): node script; see its header.
- `tools/arch-fitness.mjs` (+ `tools/arch-fitness.test.mjs`): check the crate graph stays acyclic.
- `tools/argv.mjs` (+ `tools/argv.test.mjs`): node script; see its header.
- `tools/beat-lint.mjs` (+ `tools/beat-lint.test.mjs`): gate the ladder beat chart.
- `tools/bench-compare.mjs` (+ `tools/bench-compare.test.mjs`): compare paired perf runs.
- `tools/bins.mjs` (+ `tools/bins.test.mjs`): check release binaries are present and fresh.
- `tools/board-memo.mjs` (+ `tools/board-memo.test.mjs`): read the board memo.
- `tools/board-slice.mjs` (+ `tools/board-slice.test.mjs`): read one board row.
- `tools/books.mjs` (+ `tools/books.test.mjs`): search the engine and Rust books.
- `tools/cargo-guard.mjs` (+ `tools/cargo-guard.test.mjs`): cap cargo time and messages.
- `tools/cast-safe.mjs` (+ `tools/cast-safe.test.mjs`): node script; see its header.
- `tools/city-osm.mjs` (+ `tools/city-osm.test.mjs`): convert an offline OSM slice to glTF.
- `tools/clean.mjs` (+ `tools/clean.test.mjs`): node script; see its header.
- `tools/compile-cache.mjs` (+ `tools/compile-cache.test.mjs`): node script; see its header.
- `tools/cook-char.mjs` (+ `tools/cook-char.test.mjs`): check the character intake manifest.
- `tools/crate-map.mjs` (+ `tools/crate-map.test.mjs`): map each crate to its package, dir and job.
- `tools/desync-diff.mjs` (+ `tools/desync-diff.test.mjs`): diff a desync to the first bad tick.
- `tools/docs-coverage.mjs` (+ `tools/docs-coverage.test.mjs`): ratchet per-crate missing-docs.
- `tools/doctor.mjs` (+ `tools/doctor.test.mjs`): repo health first.
- `tools/drr-queue.mjs` (+ `tools/drr-queue.test.mjs`): node script; see its header.
- `tools/duration-fmt.mjs` (+ `tools/duration-fmt.test.mjs`): node script; see its header.
- `tools/farm.mjs` (+ `tools/farm.test.mjs`): run heavy checks on the build farm.
- `tools/feature-unify.mjs` (+ `tools/feature-unify.test.mjs`): check the workspace feature union.
- `tools/fetch-content.mjs` (+ `tools/fetch-content.test.mjs`): download listed content files.
- `tools/fighter-count.mjs` (+ `tools/fighter-count.test.mjs`): node script; see its header.
- `tools/finish.mjs` (+ `tools/finish.test.mjs`): node script; see its header.
- `tools/flaky-quarantine.mjs` (+ `tools/flaky-quarantine.test.mjs`): quarantine flaky tests.
- `tools/fnv1a64.mjs` (+ `tools/fnv1a64.test.mjs`): node script; see its header.
- `tools/frame-moved.mjs` (+ `tools/frame-moved.test.mjs`): node script; see its header.
- `tools/frame-triage.mjs` (+ `tools/frame-triage.test.mjs`): triage frame captures.
- `tools/frame-v2v3.mjs` (+ `tools/frame-v2v3.test.mjs`): node script; see its header.
- `tools/gateway-health.mjs` (+ `tools/gateway-health.test.mjs`): check the gateway is up.
- `tools/gh-check.mjs` (+ `tools/gh-check.test.mjs`): run one narrow cargo command on GitHub; returns at 100 s (exit 3, then `--wait`), sends GPU/window/Windows-only lines and failed runs to the PC by itself.
- `tools/gh-search.mjs` (+ `tools/gh-search.test.mjs`): search GitHub code.
- `tools/gltf-intake.mjs` (+ `tools/gltf-intake.test.mjs`): check a glTF file on intake.
- `tools/golden-matrix.mjs` (+ `tools/golden-matrix.test.mjs`): compare tapes across machines.
- `tools/hash-util.mjs` (+ `tools/hash-util.test.mjs`): hash helpers.
- `tools/iqa-gate.mjs` (+ `tools/iqa-gate.test.mjs`): gate capture-pair image similarity.
- `tools/level-check.mjs` (+ `tools/level-check.test.mjs`): gate the hand-placed arena layout.
- `tools/license-gate.mjs` (+ `tools/license-gate.test.mjs`): gate Cargo licenses.
- `tools/mcp-install.mjs` (+ `tools/mcp-install.test.mjs`): install the MCP binary.
- `tools/mp-evidence.mjs` (+ `tools/mp-evidence.test.mjs`): node script; see its header.
- `tools/mp6-check.mjs` (+ `tools/mp6-check.test.mjs`): multiplayer check.
- `tools/mutants.mjs` (+ `tools/mutants.test.mjs`): node script; see its header.
- `tools/p4-netsim.mjs` (+ `tools/p4-netsim.test.mjs`): model 64 clients online.
- `tools/part-score.mjs` (+ `tools/part-score.test.mjs`): score board parts.
- `tools/pe-exports.mjs` (+ `tools/pe-exports.test.mjs`): read a Windows PE export table.
- `tools/plan-atlas.mjs` (+ `tools/plan-atlas.test.mjs`): read the plan atlas.
- `tools/playtest-log.mjs` (+ `tools/playtest-log.test.mjs`): check a playtest CSV log.
- `tools/pool-queue.mjs` (+ `tools/pool-queue.test.mjs`): node script; see its header.
- `tools/potato-check.mjs` (+ `tools/potato-check.test.mjs`): run the potato gate.
- `tools/product-check.mjs` (+ `tools/product-check.test.mjs`): check the built game.
- `tools/proof.mjs` (+ `tools/proof.test.mjs`): golden hash check.
- `tools/q1-check.mjs` (+ `tools/q1-check.test.mjs`): node script; see its header.
- `tools/ra8-check.mjs` (+ `tools/ra8-check.test.mjs`): node script; see its header.
- `tools/ra8-manifest.mjs` (+ `tools/ra8-manifest.test.mjs`): node script; see its header.
- `tools/rdc-diff.mjs` (+ `tools/rdc-diff.test.mjs`): pixel-diff a RenderDoc replay.
- `tools/replay-to-mp4.mjs` (+ `tools/replay-to-mp4.test.mjs`): render a replay tape to video.
- `tools/rerun-timeline.mjs` (+ `tools/rerun-timeline.test.mjs`): scrub a per-tick timeline.
- `tools/rust-quality.mjs` (+ `tools/rust-quality.test.mjs`): score Rust quality.
- `tools/seal-keys.mjs` (+ `tools/seal-keys.test.mjs`): run cargo legs with seal keys.
- `tools/ship.mjs` (+ `tools/ship.test.mjs`): node script; see its header.
- `tools/showcase.mjs` (+ `tools/showcase.test.mjs`): node script; see its header.
- `tools/sim-source-scan.mjs` (+ `tools/sim-source-scan.test.mjs`): scan sim sources for banned calls.
- `tools/sort-once.mjs` (+ `tools/sort-once.test.mjs`): node script; see its header.
- `tools/steal-land.mjs` (+ `tools/steal-land.test.mjs`): land a steal packet.
- `tools/tape-checksum.mjs` (+ `tools/tape-checksum.test.mjs`): checksum a tape plus divergence.
- `tools/wasm-gate.mjs` (+ `tools/wasm-gate.test.mjs`): ratchet the calls that type-check for wasm32 but break in a browser.
- `tools/web-search.mjs` (+ `tools/web-search.test.mjs`): web search.
- `tools/wiring-cov.mjs` (+ `tools/wiring-cov.test.mjs`): list functions the arena never runs.
- `tools/wiring-scan.mjs` (+ `tools/wiring-scan.test.mjs`): list pub items with no user.
- `tools/workspace-check.mjs` (+ `tools/workspace-check.test.mjs`): audit after board edits.

Test-only mates with no live script (audit pins kept on purpose):

- `tools/argv-contract.test.mjs`
- `tools/board-slice-cases.test.mjs`
- `tools/ci.test.mjs`
- `tools/compare-split.test.mjs`
- `tools/cover-gap.test.mjs`
- `tools/evidence-pack.test.mjs`
- `tools/float-gate.test.mjs`
- `tools/frame-check.test.mjs`
- `tools/ibl-presets.test.mjs`
- `tools/ibl-vectors.test.mjs`
- `tools/index-check.test.mjs`
- `tools/knobs-check.test.mjs`
- `tools/ladder-split.test.mjs`
- `tools/plan-index.test.mjs`
- `tools/pool-queue-batch.test.mjs`
- `tools/rng-vectors.test.mjs`
- `tools/rollback-vectors.test.mjs`
- `tools/ship-dupe-gap.test.mjs`
- `tools/skin-vectors.test.mjs`
- `tools/skills-index.test.mjs`
- `tools/sky-preset.test.mjs`
- `tools/sky-presets.test.mjs`
- `tools/sky-presets3.test.mjs`
- `tools/spawn-check.test.mjs`
- `tools/stale-refs.test.mjs`
- `tools/stale-targets.test.mjs`
- `tools/toolchain-pin.test.mjs`
- `tools/w4-compile-cache-plan.test.mjs`
