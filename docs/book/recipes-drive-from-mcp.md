# Recipe: drive it from MCP

Drive the whole engine from any MCP client (Claude Code, OpenCode,
Cursor) with one config line and twenty tools — no shell, no file
writes, no code execution. Every mutation goes preview, apply, undo
through the command service. The tool table and error shape live on
the [AI door page](ai-door.md); this recipe wires them into a
session. Full signatures live in rustdoc:
`cargo doc -p engine-endpoint --no-deps`.

## 1. Serve the project folder

The binary is `crates/endpoint/src/bin/e2040-mcp.rs`; startup
wiring is `crates/endpoint/src/startup.rs` (`parse_args`,
`build_server`, `USAGE`, `StartupError`). It serves one project
folder over stdio; the folder holds the durable `gen-*.edoc`
generations and a fresh folder starts empty. With no DIR, the
current directory is the project.

One-line client config (from `USAGE`, verified on disk):

```json
{"command": "e2040-mcp", "args": ["--project-dir", "/path/to/project"]}
```

Print the help headlessly (needs the `mcp` feature; gate: the
plain build refuses the binary naming the required feature):

```powershell
cargo run --locked -p engine-endpoint --features mcp --bin e2040-mcp -- --help
```

Expected: the `usage: e2040-mcp [--project-dir DIR] [DIR]` text
plus the one-line config above. Exit 2 on CLI misuse, 1 on a bad
project dir or transport failure; stdout carries only MCP
JSON-RPC frames.

## 2. The edit loop over the door

The only write path is `scene.read`, `tx.preview`, `tx.apply`,
`tx.undo` (see the [AI door](ai-door.md) recipe 1):

1. `scene.read` (`{"limit": 64, "cursor": 0}`) for the revision.
2. `tx.preview` with that revision, a fresh idempotency key, and
   1–256 ops — dry-run diff plus a single-use approval, writes
   nothing. Add a weapon or a map template here (see
   [Add a weapon](recipes-add-weapon.md) and
   [New map](recipes-new-map.md)).
3. `tx.apply` with the same ops plus the minted approval —
   capability-gated, revision-checked, journaled; returns
   `revision`, `key`, `undo_token`.
4. `tx.undo` with `{"undo_token": <key>}` reverts as a new
   revision.

Refusals name recovery: `stale_revision` (re-read, re-preview),
`forged_approval` / `expired_approval` / `reused_approval`
(re-preview), `duplicate_key` (fresh key), `op_not_allowed`
(names granted classes), `spatial_rejected` (names the failed
check).

## 3. Play-test the proposal before it lands

- `play.run`: headless arena match job (start, poll, cancel).
- `play.step`: live duel step — inputs for N ticks, state digest
  plus a real PNG every step. Same seed plus same tape twice
  gives the same digests and PNG bytes.
- `scene.spatial` over the proposed batch, `scene.brief` for the
  bounded digest, `sim.plan_goals` for the classical planner,
  `log.digest` / `log.query` over the observe ring (cap 256,
  page cap 128), and the Insight tools (`api.search`,
  `api.describe`, `code.callers`, `asset.refs`) to find the
  owning file and board row first.

Verify the door pieces headlessly (verified this wave on the
[verifiers page](verifiers.md)):

```powershell
cargo test --locked -p engine-endpoint --test it redteam::
cargo test --locked -p engine-playtest keypoint
```

Expected: red-team twelve tapes refused or contained with a
logged reason and no secret in any log; keypoints green. Then
run the proof harness legs that wire them by name:
`node tools/proof.mjs --skip-build` for fixtures only.

Gate notes: the stdio door, edit loop, spatial checks, and
software-raster PNGs are headless. GPU captures, windowed play,
and reference-GPU look gates need a device and are marked
`(reference GPU)` on the board — not claimed here. Board rows
MCP5 (the binary), MCP3 (brief/spatial/place), and EA1a–e (the
proposal loop) own this recipe.

## Reference

- `crates/endpoint/src/bin/e2040-mcp.rs`, `crates/endpoint/src/startup.rs`
- `crates/endpoint/src/mcp.rs` — `tool_names`, budgets, `McpToolError`
- `crates/endpoint/src/play/play_session.rs` — `PlaySession`, `play_step_with`
