# Intelligence systems: door, providers, proof

Models propose; deterministic code disposes. Providers are adapters
(mock, local, remote, ONNX later) with deadlines, cancellation,
bounded queues, and offline fallback. Model output is validated
(schema, ranges, references, revision), spatial-checked, fork
play-tested, then applied through the command service. Replay uses
recorded accepted actions and never calls a model. Full signatures
live in rustdoc: `cargo doc -p <crate> --no-deps`.

## The door and the providers

- `engine-endpoint` owns the twenty-tool stdio MCP door
  (`crates/endpoint/src/bin/e2040-mcp.rs`, `mcp.rs`,
  `scene_tools.rs`, `play.rs`, `log_tools.rs`, `insight/`). See the
  [AI door page](ai-door.md) for the tool table, the edit loop, the
  `play.step` session pattern, and the Insight search tools. To drive
  a full session from a client, see
  [Drive it from MCP](recipes-drive-from-mcp.md).
- `engine-provider` owns the request/result/event/capability contract
  plus mock, local, remote, OpenAI-compatible, Anthropic, Gemini, and
  Ollama adapters (each passing the conformance suite), the announcer
  and taunts with scripted fallback, and the assistant loop skeleton
  (transcript, inspect via `scene.brief`, proposal to `tx.preview`
  plus spatial checks, fork play-test before apply, apply with
  approval plus undo and provenance receipt). Live tests are opt-in
  by environment variable; recorded fixtures run offline.

## Observe, prove, playtest, tune

- `engine-observe` owns typed events, ring buffers, spans, the
  supervisor and fault catalog (bounded heal policies applied at
  tick boundaries and recorded in the tape), and the `log.digest`
  plus `log.query` door tools (ring cap 256, page cap 128).
- `engine-proof` owns the retroactive trace matching the live hash,
  the chaos suite with heal records, the deterministic fuzz harness,
  the format museum, workflow harnesses, bug bundles with divergence
  replay, soak, and the truth gate (the window build fails when a
  ticked render pass still returns `Unsupported` on a device).
- `engine-playtest` owns the replayable agent with bug tapes,
  reachability/win-rate/budget verifiers, coalescing fork triggers,
  causal replay (smallest cause plus first divergent tick), the
  30-file keypoint suite, and the balance-tournament gate. See the
  [verifiers page](verifiers.md) for the three runnable gates.
- `engine-perf` owns the budget gate (deterministic release-mode
  workloads plus a counting-allocator test for zero steady-state
  allocs per tick), the profiler on observe spans plus GPU
  timestamps, latency metering, and arena allocation-shape baselines.
- `engine-nn` owns the integer `.e2nn` runtime; `engine-distill`
  owns the reproducible trainer and student/script/teacher
  tournament; the ONNX converter is int8-QDQ.
- `engine-app` owns composition and lifecycle; `engine-game` owns
  the arena app as a headless data path, the loopback LAN session
  with AI fill, and practice mode.

Run the three verifier gates (also wired into
`node tools/proof.mjs`):

```powershell
cargo test --locked -p engine-playtest keypoint
cargo test --locked -p engine-playtest tournament::tests::proof_gate_reproduces
cargo test --locked -p engine-endpoint --test it redteam::
```

Run the whole harness when ready: `node tools/proof.mjs` (add
`--skip-build` for fixtures only, `--perf` for release timing).

## Reference

- `crates/endpoint/src/mcp.rs`, `crates/endpoint/src/startup.rs`
- `crates/provider/`, `crates/observe/`, `crates/proof/`, `crates/playtest/`, `crates/perf/`
- `crates/nn/`, `crates/distill/`, `crates/app/`, `crates/game/`
- `tools/proof.mjs`, `docs/EDITOR_AI_CONTROL.md`
