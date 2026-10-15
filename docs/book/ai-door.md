# The AI door: twenty MCP tools

The engine's AI door is a stdio MCP server (`crates/endpoint/src/bin/e2040-mcp.rs`)
that exposes exactly twenty tools — no shell, no file writes, no code
execution. Every mutating call goes through the editor command service
(validate → preview → atomic apply → journal → undo), so a denied, stale, or
spatially-rejected call writes nothing.

The router holds exactly the tools named by
`pub fn tool_names() -> [&'static str; 20]` in `crates/endpoint/src/mcp.rs`,
and the discovery surface (names, descriptions, input/output schemas) must
stay under `TOOL_LIST_BUDGET_CHARS: usize = 32_768` characters. That budget is
why transport DTOs carry no field docs: the schemas generate from the types,
never hand-written twice.

Errors are one shape: a machine `code` plus a `message` that always names the
legal values (`pub struct McpToolError` in `crates/endpoint/src/mcp.rs`).
There is no stringly-typed failure anywhere on the door.

## The twenty tools

| Tool | Read-only | What it does |
|---|---|---|
| `scene.read` | yes | Bounded snapshot: revision plus one page of stable-id rows (`SceneReadOutput`) |
| `scene.brief` | yes | Bounded brief text plus digest (`SceneBriefOutput`) |
| `scene.spatial` | yes | Check findings, optionally over a proposed batch (`SceneSpatialOutput`) |
| `tx.preview` | yes | Dry-run diff with no side effects; mints a single-use approval (`TxPreviewOutput`) |
| `tx.apply` | no | Atomic capability-gated apply; idempotency-keyed, revision-checked (`TxApplyOutput`) |
| `tx.undo` | no | Revert the newest batch as a new revision (`TxUndoOutput`) |
| `scene.place` | no | Relation intent (`"next to wall:north"`) through the command gate (`ScenePlaceOutput`) |
| `play.run` | no | Headless arena match job: start, poll, cancel (`PlayRunOutput`) |
| `play.step` | no | Live duel step: inputs for N ticks, state digest plus a real PNG (`PlayStepOutput`) |
| `capture` | yes | Renderer receipt — or structured `unsupported_capture` when unwired |
| `log.digest` | yes | Stable digest of the observe ring |
| `log.query` | yes | Cursor-paged observe events |
| `api.search` | yes | BM25 search over the Engine Insight index (`ApiSearchOutput`) |
| `api.describe` | yes | One full index entry: signature, docs, units, owner, callers (`ApiDescribeOutput`) |
| `code.callers` | yes | Reverse lookup of items mentioning a path (`CodeCallersOutput`) |
| `asset.refs` | yes | Code and scenes touching one asset (`AssetRefsOutput`) |
| `rpc.discover` | yes | Self-description: tool manifest with schemas (`DiscoverOutput`) |
| `scene.watch` | yes | Read-only scene subscription (`WatchOutput`) |
| `scene.query` | yes | Spatial questions with reasons (`SceneQueryOutput`) |
| `sim.plan_goals` | yes | Classical planner over encounter goals (`PlanGoalsOutput`) |

## Recipe 1: the edit loop

The only write path is preview → apply → (undo). Approvals bind to the
session, the current revision, the canonical hash of the exact batch, and an
expiry equal to the current revision — any interleaved write voids
outstanding approvals.

1. `scene.read` for the revision:
   `{"limit": 64, "cursor": 0}` → `{"revision": 3, "total": 12, ...}`.
2. `tx.preview` with that revision, a fresh idempotency key, and 1–256 ops:
   `{"expected_revision": 3, "idempotency_key": 41, "ops": [...]}` →
   `effects`, `resulting_revision`, and an `approval` for `tx.apply`.
   Preview stages plus spatial-checks on a private copy and writes nothing.
3. `tx.apply` with the same ops and the minted approval:
   `{"expected_revision": 3, "idempotency_key": 41, "ops": [...],
   "approval": {...}}` → landed receipt with `revision`, `key`,
   and `undo_token`.
4. `tx.undo` with `{"undo_token": 41}` reverts as a new revision.
   A token that does not match the newest batch key fails with
   `undo_mismatch`; omitting it undoes the newest batch.

Typical refusals and their recovery: `stale_revision` (re-read, re-preview),
`forged_approval` / `expired_approval` / `reused_approval` (re-preview),
`duplicate_key` (fresh key for a new batch), `op_not_allowed` (names the
granted classes), `spatial_rejected` (names the failed check), `bad_id` /
`bad_pose` / `empty_batch` / `too_many_ops` (fix the batch).

## Recipe 2: the `play.step` session pattern

`play.step` holds a live duel across calls and answers every step with a
state digest plus a real capture PNG. Two patterns exist; prefer the session.

**Session (stateful, for turn-by-turn play):**

```rust
use engine_endpoint::play::play_session::{PlaySession, PlayStepArgs};
```

- `pub fn new(seed: u64, fighters: u8, half_extent_m: f32) -> Result<Self, McpToolError>`
  opens a duel (`fighters` in `2..=MAX_FIGHTERS`, `half_extent_m` in
  `5.0..=100.0` meters; anything else names the legal values).
- `pub fn cursor(&self) -> u32` is the ticks simulated so far.
- `pub fn advance(&mut self, inputs: &[PlayInputDto], ticks: u32, width: Option<u32>, height: Option<u32>) -> Result<PlayStepOutput, McpToolError>`
  takes inputs for the next `ticks` ticks. New inputs must land in
  `cursor..cursor + ticks`, sorted, with no duplicates; a refusal moves
  neither the cursor nor the tape.

Twenty steps of a duel is twenty `advance` calls of 2 ticks: each returns
`tick`, `digest` (a decimal string), `fighters` (hp, stamina, phase, position
and yaw per fighter), `events` (the newest hits, blocks and kills with their
ticks), `inputs_applied`, a `summary` line, a capture `manifest`, and
`png_bytes`. Same seed plus same tape twice gives the same
digests and the same PNG bytes — the acceptance test plays exactly this and
checks every PNG decodes and passes the pixel-variance gate.

**Stateless (one prefix, one receipt):**

- `ServerState::play_step` (the MCP tool path) validates, brings the
  session's live bout to `upto_tick` (only the new ticks when the tape extends
  the last one), digests the last checkpoint, and captures. The hook, the
  bout and the re-query cache all belong to that session.
- `pub fn play_step_with(args: &PlayStepArgs, hook: Option<StepCaptureHook>) -> Result<PlayStepOutput, McpToolError>`
  is the same step as a one-shot with an explicit capture hook for hosts and
  tests; it keeps nothing between calls.

Capture plumbing: `pub type StepCaptureHook = fn(&CaptureArgs, digest: u64, tick: u32) -> Result<CaptureOutput, McpToolError>;`.
A GPU host hands its hook to a session once, while building it, with
`ServerState::with_step_capture_hook` (or `PlaySession::new_with_hook`); no
hook lives in a process-wide static. Without a hook the software
rasterizer still answers a real PNG — `play.step` never returns
`unsupported_capture` (only the bare `capture` tool does that when unwired).
Software receipts cap at 512×512 pixels (`MAX_SOFTWARE_PIXELS`); a wired GPU
callback covers the full `CAPTURE_MAX_DIM` of 2048. Matches cap at
`MAX_PLAY_TICKS: u32 = 3600` ticks and `MAX_PLAY_INPUTS: usize = 2048` inputs.

Step refusals: `bad_ticks` (`upto_tick` outside `1..=3600`), `bad_fighters`,
`bad_extent`, `too_many_inputs`, `stale_tick` (behind the live cursor),
`invalid_tape`, `unsorted_tape`, `duplicate_input`, `bad_size`,
`black_capture` (flat frames never pass), `bad_receipt` (a hook receipt that
is not a PNG or mismatches dimensions).

## Recipe 3: searching the engine (Insight tools)

`api.search` ranks the Engine Insight index with BM25 in plain Rust — name
weight 10, path weight 5, docs weight 1 — ties broken on path text:

`pub fn search(index: &Index, query: &str, limit: u32) -> Result<Vec<SearchHit>, InsightError>`
in `crates/endpoint/src/insight/search.rs`. Limits run `1..=32`, default 8
(`SEARCH_DEFAULT_LIMIT`); blank queries fail as `empty_query`, over-limits as
`bad_limit`.

The agent loop: `api.search` for a ranked hit list, `api.describe` for one
full entry (signature, docs, units, file, board-row owner, callers),
`code.callers` for the reverse graph, `asset.refs` for everything touching
one asset stem. Unknown paths fail as `unknown_path` with a search-first
recovery. The index rebuilds incrementally per file
(`pub fn rebuild_one(&mut self, path: &str, krate: &str, text: &str) -> RebuildReceipt`)
and matches a full rebuild exactly.

## Redteam notes (RV1)

Every wave replays the red-team tape suite in
`crates/endpoint/tests/it/redteam.rs` over `fixtures/redteam/` (twelve tapes:
prompt injection through briefs, asset names, and scene text; forged
applies; tool-result widening; out-of-scope shell/file-write calls;
oversize transport; batch overflow; direct secret exfiltration).

The contract: every case is refused or contained with a logged reason, and
no secret appears in any log. Adding a case is adding one fixture file —
no harness change. The suite runs inside the proof gate:

```sh
cargo test --locked -p engine-endpoint --test it redteam::
```

## Reference

Full signatures live in rustdoc (`cargo doc -p engine-endpoint --no-deps`);
the files that own this page:

- `crates/endpoint/src/mcp.rs` — `tool_names`, `TOOL_LIST_BUDGET_CHARS`,
  `ServerState`, `McpToolError`, the four tx/scene tools
- `crates/endpoint/src/scene_tools.rs` — `TOOL_SCENE_BRIEF`, `TOOL_SCENE_SPATIAL`,
  `TOOL_SCENE_PLACE`, budgets (`SCENE_BRIEF_MAX_BUDGET: u32 = 16384`,
  `SCENE_SPATIAL_MAX_FINDINGS: u32 = 256`, `MAX_PLACE_WANTS: usize = 16`)
- `crates/endpoint/src/play.rs` — `PlayJobs::play_run`, `capture_unsupported`,
  `capture_with`, `MAX_PLAY_TICKS`, `MAX_PLAY_INPUTS`
- `crates/endpoint/src/play/play_session.rs` — `PlayStepTool`, `PlaySession`,
  `play_step_with`, `StepCaptureHook`
- `crates/endpoint/src/log_tools.rs` — `TOOL_LOG_DIGEST`, `TOOL_LOG_QUERY`,
  `LOG_RING_CAP: usize = 256`, `LOG_QUERY_MAX_ROWS: u32 = 128`
- `crates/endpoint/src/insight/` — `Index`, `search`, `callers_of`,
  `asset_refs`, the four Insight tools
