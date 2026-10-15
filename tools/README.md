# tools/
Node scripts for repo checks and sprint plumbing. No game or engine code lives here.
Start with `node tools/doctor.mjs --help` for repo health first.
Stranger picks (all take --help, exit 0):
- `node tools/board-slice.mjs --help` to read one board row.
- `node tools/workspace-check.mjs --help` to audit after board edits.
- `node tools/crate-map.mjs --help` to map crates to dirs.
- `node tools/packet-gate.mjs --help` to gate a packet diff.
Each script has a `*.test.mjs` mate next to it; a few audit `*.test.mjs` files have no live script (listed below).
Helpers live in `tools/vendor/`; retired ones live in `docs/archive/`.
Keep scripts small and portable; docs stay in `docs/`.

## Live tools

76 live scripts. 81 dead tools (39 A-L plus 42 M-Z) were deleted 2026-10-10 and are not listed. Short lines; the script header is the full doc.

- `tools/affected.mjs` (+ `tools/affected.test.mjs`): node script; see its header.
- `tools/argv.mjs` (+ `tools/argv.test.mjs`): node script; see its header.
- `tools/bench-compare.mjs` (+ `tools/bench-compare.test.mjs`): compare paired perf runs.
- `tools/board-memo.mjs` (+ `tools/board-memo.test.mjs`): read the board memo.
- `tools/board-slice.mjs` (+ `tools/board-slice.test.mjs`): read one board row.
- `tools/cast-safe.mjs` (+ `tools/cast-safe.test.mjs`): node script; see its header.
- `tools/clean.mjs` (+ `tools/clean.test.mjs`): node script; see its header.
- `tools/clean-run.mjs` (+ `tools/clean-run.test.mjs`): node script; see its header.
- `tools/compile-cache.mjs` (+ `tools/compile-cache.test.mjs`): node script; see its header.
- `tools/crate-map.mjs` (+ `tools/crate-map.test.mjs`): map each crate to its package, dir and job.
- `tools/desync-diff.mjs` (+ `tools/desync-diff.test.mjs`): diff a desync to the first bad tick.
- `tools/doctor.mjs` (+ `tools/doctor.test.mjs`): repo health first.
- `tools/drift-check.mjs` (+ `tools/drift-check.test.mjs`): node script; see its header.
- `tools/drr-queue.mjs` (+ `tools/drr-queue.test.mjs`): node script; see its header.
- `tools/duration-fmt.mjs` (+ `tools/duration-fmt.test.mjs`): node script; see its header.
- `tools/fighter-count.mjs` (+ `tools/fighter-count.test.mjs`): node script; see its header.
- `tools/finish.mjs` (+ `tools/finish.test.mjs`): node script; see its header.
- `tools/flaky-quarantine.mjs` (+ `tools/flaky-quarantine.test.mjs`): quarantine flaky tests.
- `tools/fnv1a64.mjs` (+ `tools/fnv1a64.test.mjs`): node script; see its header.
- `tools/frame-moved.mjs` (+ `tools/frame-moved.test.mjs`): node script; see its header.
- `tools/frame-triage.mjs` (+ `tools/frame-triage.test.mjs`): triage frame captures.
- `tools/frame-v2v3.mjs` (+ `tools/frame-v2v3.test.mjs`): node script; see its header.
- `tools/gateway-health.mjs` (+ `tools/gateway-health.test.mjs`): check the gateway is up.
- `tools/gh-search.mjs` (+ `tools/gh-search.test.mjs`): search GitHub code.
- `tools/gltf-intake.mjs` (+ `tools/gltf-intake.test.mjs`): check a glTF file on intake.
- `tools/hash-util.mjs` (+ `tools/hash-util.test.mjs`): hash helpers.
- `tools/inbox-open.mjs` (+ `tools/inbox-open.test.mjs`): list open inbox rows.
- `tools/lane-board.mjs` (+ `tools/lane-board.test.mjs`): retired lane board read.
- `tools/lanes.mjs` (+ `tools/lanes.test.mjs`): retired lanes read.
- `tools/license-gate.mjs` (+ `tools/license-gate.test.mjs`): gate Cargo licenses.
- `tools/mcp-install.mjs` (+ `tools/mcp-install.test.mjs`): install the MCP binary.
- `tools/mp-evidence.mjs` (+ `tools/mp-evidence.test.mjs`): node script; see its header.
- `tools/mp6-check.mjs` (+ `tools/mp6-check.test.mjs`): multiplayer check.
- `tools/mutants.mjs` (+ `tools/mutants.test.mjs`): node script; see its header.
- `tools/pack-clean.mjs` (+ `tools/pack-clean.test.mjs`): node script; see its header.
- `tools/packet-gate.mjs` (+ `tools/packet-gate.test.mjs`): gate a packet file.
- `tools/part-score.mjs` (+ `tools/part-score.test.mjs`): score board parts.
- `tools/plan-atlas.mjs` (+ `tools/plan-atlas.test.mjs`): read the plan atlas.
- `tools/pool-queue.mjs` (+ `tools/pool-queue.test.mjs`): node script; see its header.
- `tools/presence.mjs` (+ `tools/presence.test.mjs`): list live sessions.
- `tools/proof.mjs` (+ `tools/proof.test.mjs`): golden hash check.
- `tools/q1-check.mjs` (+ `tools/q1-check.test.mjs`): node script; see its header.
- `tools/queue-cap.mjs` (+ `tools/queue-cap.test.mjs`): check queue caps and packet roles.
- `tools/ra8-check.mjs` (+ `tools/ra8-check.test.mjs`): node script; see its header.
- `tools/ra8-manifest.mjs` (+ `tools/ra8-manifest.test.mjs`): node script; see its header.
- `tools/research-cap.mjs` (+ `tools/research-cap.test.mjs`): check the research card cap.
- `tools/rust-quality.mjs` (+ `tools/rust-quality.test.mjs`): score Rust quality.
- `tools/seal-keys.mjs` (+ `tools/seal-keys.test.mjs`): run cargo legs with seal keys.
- `tools/seat-ledger.mjs` (+ `tools/seat-ledger.test.mjs`): node script; see its header.
- `tools/secret-gate.mjs` (+ `tools/secret-gate.test.mjs`): block secret leaks.
- `tools/ship.mjs` (+ `tools/ship.test.mjs`): node script; see its header.
- `tools/showcase.mjs` (+ `tools/showcase.test.mjs`): node script; see its header.
- `tools/showcase-gate.mjs` (+ `tools/showcase-gate.test.mjs`): gate the showcase.
- `tools/sim-source-scan.mjs` (+ `tools/sim-source-scan.test.mjs`): scan sim sources for banned calls.
- `tools/sort-once.mjs` (+ `tools/sort-once.test.mjs`): node script; see its header.
- `tools/sprint-lock.mjs` (+ `tools/sprint-lock.test.mjs`): hold one sprint per tree.
- `tools/sprint-loop.mjs` (+ `tools/sprint-loop.test.mjs`): run the sprint loop.
- `tools/steal-land.mjs` (+ `tools/steal-land.test.mjs`): land a steal packet.
- `tools/steal-next.mjs` (+ `tools/steal-next.test.mjs`): pick the next steal.
- `tools/web-search.mjs` (+ `tools/web-search.test.mjs`): web search.
- `tools/weekly-clip.mjs` (+ `tools/weekly-clip.test.mjs`): cut the weekly clip.
- `tools/workspace-check.mjs` (+ `tools/workspace-check.test.mjs`): audit after board edits.

Test-only mates with no live script (audit pins kept on purpose):

- `tools/argv-contract.test.mjs`
- `tools/board-slice-cases.test.mjs`
- `tools/compare-split.test.mjs`
- `tools/cover-gap.test.mjs`
- `tools/drift-check-catch.test.mjs`
- `tools/float-gate.test.mjs`
- `tools/frame-check.test.mjs`
- `tools/ibl-presets.test.mjs`
- `tools/ibl-vectors.test.mjs`
- `tools/index-check.test.mjs`
- `tools/knobs-check.test.mjs`
- `tools/ladder-split.test.mjs`
- `tools/lanes-cold.test.mjs`
- `tools/lanes-warm.test.mjs`
- `tools/plan-index.test.mjs`
- `tools/pool-queue-batch.test.mjs`
- `tools/rng-vectors.test.mjs`
- `tools/rollback-vectors.test.mjs`
- `tools/seat-guard.test.mjs`
- `tools/ship-dupe-gap.test.mjs`
- `tools/skin-vectors.test.mjs`
- `tools/sky-preset.test.mjs`
- `tools/sky-presets.test.mjs`
- `tools/sky-presets3.test.mjs`
- `tools/spawn-check.test.mjs`
- `tools/stale-refs.test.mjs`
- `tools/stale-targets.test.mjs`
- `tools/w4-compile-cache-plan.test.mjs`
