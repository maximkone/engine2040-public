# Speed E12 gates: must-pass vs quarantine

Date: 2026-10-06.
Status: proposal only.
Scope: test gates in `C:/engine2040`.
Change: none. No code change. No gate change.

Source of truth today: `tools/proof.mjs`.
Default gate runs workspace audit, fmt, build, tests, clippy, named hooks, deny, then proof fixtures.
Perf gate runs `#[ignore]` timing tests in release with one thread.
Mutant gate runs `tools/mutants.mjs` per changed crate.

## Must-pass

These stay in the default gate. They are fast. They are stable. They guard the hash law.

- `node tools/workspace-check.mjs`. Reason: checks metadata, edges, modules, genre ledger. Runs in seconds.
- `cargo fmt --all -- --check`. Reason: style stays clean. Runs in seconds.
- `cargo build --locked --workspace --release`. Reason: the workspace compiles. No silent skip.
- `cargo test -j4 --locked --workspace --all-targets` (default features). Reason: headless std-only build. No GPU. No net. No secrets.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` (default and all features). Reason: lint bar stays at zero.
- Split all-features legs: `--exclude engine-render` plus `-p engine-render`. Reason: same coverage, two budgets. Avoids one 60 min timeout. Render GPU chart tests run alone.
- Named hooks: `engine-endpoint --test redteam`, `engine-playtest tournament proof_gate_reproduces`, `engine-playtest keypoint`, `engine-provider --test matrix`, `engine-playtest living`, `engine-playtest spatial_eval`. Reason: acceptance hooks. They also run in the workspace legs. Names keep them visible.
- `cargo deny check` when installed. Reason: licenses and advisories. Skip with INFO when missing. Never fake a pass.
- Legacy golden x2: hash `d57b5c8ae3096614`. Reason: hash law. Two runs must match.
- Golden `engine_version` matches workspace version. Reason: a stale receipt must fail even with the right hash.
- Seeds `0`, `2`, `2040`, `999983` repeated, plus distinct receipts. Reason: same input gives same hash. Different seeds differ.
- Tape record and replay roundtrip. Reason: saved inputs replay byte exact.
- Corrupt tape fails with first bad tick. Reason: damage is found, not hidden.
- Unsupported backend fails loudly. Reason: no silent fallback.
- `node tools/rust-quality.mjs`. Reason: debt cannot rise. Runs in about 2 s.
- `node --test tools/*.test.mjs`. Reason: gate tools test themselves.
- Four fixtures listed: twin-stick, vehicle-course, traversal-course, second-game. Reason: cheap file check. Generality cannot rot.
- `cargo test --test ge1_genres` and `--test second_game` in full mode. Reason: genre generality harness. Stable fixtures.
- GE2 genre ledger re-check and GP5 empty `crates/` diff. Reason: fixtures must not need engine edits to pass.

## Quarantine

These leave the default gate. They run in `--perf`, nightly, soak, or by hand. A fail here never blocks a logic packet.

- All `#[ignore = "timing: ..."]` tests. Reason: wall clock flakes under load. They need release, one thread, and a quiet box. Examples: perf baselines, spatial budgets, audio budgets, render per-pass ms, nav rebake, siege p99.
- `game netgame` UDP loopback test. Reason: passes alone, flakes to empty under full load.
- `render` GPU submit and window execution tests. Reason: compile is must-pass. Run on GPU is not. Needs the reference GPU at boost clocks.
- PQ1 input latency suite (`engine-perf latency`). Reason: needs release clocks and headless report lines. Lives in `--perf`.
- Instruction counts (gungraun, Callgrind). Reason: Linux only. Needs valgrind, a saved `perf_gate` baseline, and a 2% budget. Debug numbers do not transfer.
- `gpu_live` truth gate. Reason: report-only until GX2-GX5 and GX7-GX10 land. It logs. It does not fail the run.
- `--mutants` gate. Reason: slow. Fails closed per crate. Run per changed crate or nightly, not per packet.
- Live provider tests: anthropic, openai, gemini, ollama, local sidecar, remote host. Reason: needs tokens or servers. Secrets never gate a local run.
- `provider transport` TLS stub test. Reason: needs a test CA. No minter in the graph.
- `rules tests/data` writer. Reason: writes gold once by hand on purpose. Not a per-run check.
- `rules budget` probe and strict timing. Reason: tuning charts for a quiet PC. Not a gate.
- `intake_verb` real props test with `E2040_MAKE_RUN`. Reason: needs external Kaggle output. Manual only.
- Hour-long online soak and 10 min siege p99. Reason: too long for a packet gate. Soak lane only.
- `game practice` 1 s shader-edit visibility. Reason: wall clock demo. Run once by hand.

## Rules for moving a test

- A quarantine test joins must-pass only with numbers, hash x2, and a quiet-box repeat.
- A timing test never joins must-pass in debug or parallel mode.
- A GPU test never joins must-pass without a reference-GPU manifest.
- A live or secret test never joins must-pass.
- A held moonshot never joins either list until its kill rule is met.

## Proposal

- Keep `tools/proof.mjs` default as the must-pass list above.
- Keep `tools/proof.mjs --perf` as the quarantine runner.
- Keep `tools/proof.mjs --mutants` as a separate nightly or changed-crate lane.
- No files change in this note. Next step needs owner word.
