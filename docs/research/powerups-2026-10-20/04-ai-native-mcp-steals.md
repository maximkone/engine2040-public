# 04 - AI-native and MCP: repos that make the engine stronger

Answer: the AI-native side of engine2040 does not need another engine. It needs four
things the tree still does not have, and four repos outside the door have each one.
(1) A task suite where the referee is a program, not a model: `GamePhanesStudio/GamePhanes`
(GameForgeBench, MIT) scores 81 game-engine tasks from a deterministic native probe with
"no LLM judge in the scoring path", and `FreedomIntelligence/gamecraft-bench` (Apache-2.0)
does the same with a build gate plus a formula over per-requirement scores. (2) A way to keep
a 66,500-char tool list from growing: `dcc-mcp/dcc-mcp-core` (MIT, Rust) moves read-only
views out of `tools/list` into MCP resources and ships a measured projection for the rest,
and `microsoft/mcp-for-beginners` (MIT) carries the cursor pagination and progress-notification
shapes. (3) A job and task shape that matches the protocol we locked: the final `SEP-2663`
tasks extension plus a worked reference server, and `dcc-mcp-core`'s job poller with one
status vocabulary and disk-backed counters. (4) A repo map and a merge window:
`fkenmar/atlas` (MIT, Rust) is Aider's repo map unbundled with an exact-token budget ladder,
and `redhat-et/ripwire` (Apache-2.0) has a cross-branch conflict oracle and a diff whose unit
is the statement, not the line. `wd30x/loom-engine` (MIT/Apache-2.0) is the closest donor of
all: a Rust engine written to be driven by an agent, where every edit re-runs the recorded
session and the whole file format set ships as JSON Schema. Two gaps are already closed in
our tree and are listed under Rejections instead: `tune.sweep` (D20) exists as
`crates/playtest/src/tournament.rs` `sweep_difficulty_curve` plus `render_sweep_csv` and the
staged undo in `crates/editor/src/tuning.rs`; only the door verb name is missing, so no donor
is needed. Wave-function collapse is also already landed (`crates/world/src/wfc.rs`, LADDER
line 282), so `mxgmn/WaveFunctionCollapse` and `krychu/wfc` are not re-mined.

## Ranked finds

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Game-agent benchmark whose score comes from a deterministic runtime probe, not a judge | `GamePhanesStudio/GamePhanes` https://github.com/GamePhanesStudio/GamePhanes read live 2026-10-20 | MIT (LICENSE read live; `Copyright (c) 2026 GamePhanes contributors`) | D19, D18, HS20 | `release/tasks/godot-replay-orbit-camera/rubric.yaml` (real_behavior_contract + behavior_scoring dimensions state_transition/boundary_behavior/runtime_observation + `no_op_requirement` + `static_checks_role: supporting_only`), `release/tasks/godot-replay-orbit-camera/tests/orbit_camera_probe.gd` (headless probe printing sentinel lines), `release/tasks/godot-replay-orbit-camera/tests/test_outputs.py`, `release/tasks/godot-replay-orbit-camera/tests/test.sh` (writes `reward.json`), `release/tasks/godot-replay-orbit-camera/task.toml`. OBSERVED | M | The probe needs a real runtime in the task's own engine (Godot here); if our door cannot host the probe in the target project, the referee half dies and only the rubric shape survives | PORT |
| 2 | 140 game-generation tasks with a launch gate, a hidden rubric and a score formula | `FreedomIntelligence/gamecraft-bench` https://github.com/FreedomIntelligence/gamecraft-bench read live 2026-10-20 | Apache-2.0 (LICENSE read live) | D18, HS20, D19 | `tasks/example/tests/rubric.json` (`build_check.cmd` gate, per-requirement `agg` max/mean, `score_formula`), `gamecraft_bench/verifier/score.py` (build check first, per-requirement max over demos, formula eval clamped to [0,1]), `gamecraft_bench/verifier/replay.py` (Xvfb + xdotool + ffmpeg, one mp4 per trace), `tasks/<family>-<name>/task.toml`. OBSERVED. The multimodal LLM judge needs OPENAI/ANTHROPIC keys, so judge-scored runs are impossible on free models; only the gate, the trace replay and the formula are portable | M | If the judge is the only path to a number, drop the row: we will not pay for it | PORT (gate + traces + formula only; the judge half is RESEARCH-ONLY) |
| 3 | A Rust game engine built to be driven by an agent: every format is text, every edit re-runs the session | `wd30x/loom-engine` https://github.com/wd30x/loom-engine read live 2026-10-20 | MIT (LICENSE-MIT read live) + Apache-2.0 (LICENSE-APACHE in root) | D19, D16 | `crates/loom_mcp/src/server.rs` (one INSTRUCTIONS constant naming the whole loop; `edit_node` re-runs the session from tick 0 to the current tick on the recorded input after each edit, i.e. the edit returns its own check), `crates/loom_engine/src/schema.rs` (JSON Schema draft 2020-12 for scene file, clip file, state dump, trace line and events, committed as `schema/loom.schema.json`), `README.md` (headless, fixed 60 Hz, RON/JSON only). OBSERVED | M | If our scene format cannot be emitted as JSON Schema from the types, D16's resource half has no donor and the row falls back to hand-written schema text | PORT |
| 4 | Gateway-native MCP resources, so read-only views stop costing tool definitions | `dcc-mcp/dcc-mcp-core` https://github.com/dcc-mcp/dcc-mcp-core read live 2026-10-20 | MIT (LICENSE read live) | D16 (tool-text-budget half), D18 | `crates/dcc-mcp-gateway/src/gateway/native_resources/mod.rs` ("Promoting them to resources keeps the gateway tool surface lean - clients that need the view fetch it on demand via `resources/read` instead of paying for tool definitions in every `tools/list`"; one `ROOT_URI` + `pointer()` + `parse()` + `build_payload()` per family, about 10 lines per family), `.../native_resources/instances.rs`, `.../native_resources/catalog.rs`. OBSERVED | S | If the four D16 resources cannot be fetched by `resources/read` without a tool call, the lean-surface win is zero and the row dies | PORT |
| 5 | Measured projection for a long catalogue: compact fields, capped page, and a recorded reason to keep the summary readable | `dcc-mcp/dcc-mcp-core` (same repo, different file) https://github.com/dcc-mcp/dcc-mcp-core read live 2026-10-20 | MIT | D16 (tool-text budget), OPT29 | `crates/dcc-mcp-skills/src/catalog/list_projection.rs` (`DEFAULT_LIST_SKILLS_LIMIT` 25, `MAX_LIST_SKILLS_LIMIT` 50, `DEFAULT_SUMMARY_CHARS` 200, `COMPACT_FIELDS` = 8 routing fields; the file's own comment records that trimming the summary to 120 chars saved about 21% per page but forced an extra `get_skill_info` round trip that cost far more than the ~500 tokens saved, and that the 13-field legacy set is still ~20% heavier). OBSERVED | S | If a truncated summary makes the agent make one more round trip than it would have made anyway, the projection is a loss and it dies | PORT |
| 6 | Cursor pagination for list methods and the progress-notification and cancellation shapes | `microsoft/mcp-for-beginners` https://github.com/microsoft/mcp-for-beginners read live 2026-10-20 | MIT (LICENSE read live) | D16 (progress + 4 resources), AX28 (cancel half) | `04-PracticalImplementation/pagination/README.md` (cursor pagination on `tools/list`, `resources/list`, `prompts/list`, `resources/templates/list`; `nextCursor` semantics), `05-AdvancedTopics/mcp-protocol-features/README.md` (progress notification with progressToken/progress/total/message; cancellation via `ctx.is_cancelled` returning partial work). OBSERVED | S | If `tools/list` cannot carry a cursor in rmcp 3.4.1, the pagination half dies and only the progress shape survives | PORT |
| 7 | The final MCP tasks extension, plus a worked reference server for it | `modelcontextprotocol/modelcontextprotocol` https://github.com/modelcontextprotocol/modelcontextprotocol `seps/2663-tasks-extension.md` read live 2026-10-20; `rohitg00/ai-engineering-from-scratch` https://github.com/rohitg00/ai-engineering-from-scratch `certifications/mcpa/lessons/21-long-running-work-and-tasks/code/main.py` read live 2026-10-20 | Apache-2.0 for new contributions / MIT for older ones (LICENSE read live, transition notice); reference server MIT (LICENSE read live) | AX28 | SEP text: extension id `io.modelcontextprotocol/tasks`, methods `tasks/get` + `tasks/update` + `tasks/cancel`, a `resultType: "task"` discriminator, server-directed creation, and the removal of the blocking `tasks/result` trap. Reference code: the client-capability check on `_meta`, error `-32021` (client did not declare the extension) and `-32022` (unsupported protocol version), protocol version `2026-07-28`, `server/discover`, and the task snapshot fields (taskId, status, createdAt, lastUpdatedAt, ttlMs, pollIntervalMs, statusMessage, inputRequests, result). OBSERVED. CORRECTION to `docs/research/steals-2026-10-12/aidoor.md` item 5, which used the older 2025-11-25 experimental shape | S | If rmcp 3.4.1 cannot mint a `resultType: "task"` response, AX28 falls back to the hand-rolled start/poll shape and this row dies | PORT |
| 8 | One job registry with one status vocabulary and counters that read from disk, not from cache | `dcc-mcp/dcc-mcp-core` https://github.com/dcc-mcp/dcc-mcp-core read live 2026-10-20 | MIT | AX28, D16 | `crates/dcc-mcp-job/src/poller.rs` (six statuses pending/running/completed/failed/cancelled/interrupted plus `TERMINAL_JOB_STATUSES`; disk-backed counters so a cached `current: 0` cannot beat frames already on disk, `COUNTER_SOURCE_REPORTED` vs `COUNTER_SOURCE_DISK`; `PollContract { owner, tool, argument_field }`; the file's own history: untracked job types made `--wait` impossible and status queries minted fresh jobs for unknown ids). OBSERVED | S | If our `play.run` job keeps two status vocabularies (one in `crates/endpoint/src/play.rs`, one on the wire), the row dies | PORT |
| 9 | Cross-branch conflict oracle, a diff whose unit is the statement, and an edit-time contract check | `redhat-et/ripwire` https://github.com/redhat-et/ripwire read live 2026-10-20 | Apache-2.0 (LICENSE read live) | FU2, D19 | `src/mergescout.h` (per-symbol body-hash overlap between two refs: a changed symbol on two arms is a true conflict, different symbols in one file is textual risk, landing order is greedy fewest-conflicts-first with ties broken by ref name ascending, the dirty tree joins as an implicit arm), `src/slicediff.h` (def-use diff at a revision versus now, aligned on role and never on line or text, "absence is disclosed, never silently empty": `comparable="0"` and the legend says an empty diff is not evidence of no change), `src/editcheck.h` (working-tree symbol against the git-HEAD baseline, parameters and definition count as the contract). C++23: ideas only, reimplement in Rust. OBSERVED | M | If our scene diff is still a row-by-id snapshot compare with no statement-level unit, the merge window stays a diff and the row dies | PORT |
| 10 | Aider's repo map unbundled: token-budgeted, ranked, degrading in a recorded order, with an exploration-token benchmark | `fkenmar/atlas` https://github.com/fkenmar/atlas read live 2026-10-20 | MIT (LICENSE read live) | OPT29, D18 | `src/budget.rs` (exact BPE counts via tiktoken cl100k; degradation ladder: drop private symbols, then drop parameter names and keep types, then collapse the lowest-ranked files into a directory-grouped footer; `DEFAULT_BUDGET` 2048; deterministic ordering by score desc then path asc), `src/rank.rs` (personalized PageRank over the import graph, damping 0.85, L1 convergence 1e-9, test files weighted 0.25), `benchmark/README.md` (`exploration_tokens` = input-side tokens up to and including the turn of the first file edit; 20 verified questions; 85,670 to 29,781 median tokens, 3 turns to 1). OBSERVED. Our `crates/endpoint/src/insight/index.rs:356` states "no `syn`: the endpoint headless graph stays dependency-free" and has no rank and no budget, so the map is a real gap | M | If the map needs a tree-sitter grammar we cannot license-check into the endpoint crate, the row dies and the budget idea has to be hand-rolled over the existing text index | PORT |
| 11 | Every measurement is also a check: diff in buckets, with an `expect` that turns it into a verdict | `MalloyTheDev/aseprite-mcp` https://github.com/MalloyTheDev/aseprite-mcp read live 2026-10-20 | MIT (LICENSE read live) | D19, FU2 | `src/aseprite_mcp/tools/inspect.py` (`assess_sprite` at line 519 measures the drawing; `diff_sprites` at line 992 returns four buckets that sum to `changed_pixels`, and the `expect` parameter at line 999 turns the measurement into a verdict at line 1120), `README.md` ("An agent that cannot see the screen can diff two frames, score palette conformance, and verify a loop"). OBSERVED. The pixels are not ours; the shape is: measure, bucket, expect, verdict | S | If our diff cannot bucket the change into named categories, the `expect` verdict has nothing to grade and the row dies | PORT |
| 12 | Project memory for an agent as typed plain-text notes that know when they went stale | `sanjuraw/narrowbit-memory` https://github.com/sanjuraw/narrowbit-memory read live 2026-10-21 | MIT (LICENSE read live) | D18 | `src/notes.ts` (`MEMORY_TYPES` = fact, decision, constraint, convention, failure, bug, command, environment; `staleFilesOf` recomputes a file hash and reports the note as possibly out of date; failures score higher because dead ends are expensive to rediscover; `renderMemory` prints the stale warning), `README.md` (no model call, one `.md` per note under `.narrowbit/memory/`, MCP tools `memory_recall`, `memory_remember`, `memory_list`, `memory_resolve`). OBSERVED. Zero stars, early 0.1 preview: take the shape, not the trust | S | If a memory note cannot carry the tape hash D18 asks for, it is prose and the row dies | PORT |

## Rejections

- `modelcontextprotocol/rust-sdk` `crates/rmcp/src/task_manager.rs` - already mined in
  `docs/research/steals-2026-10-12/aidoor.md` item 5; only its new SEP-2663 shape is a
  correction, folded into row 7.
- `modelcontextprotocol/servers` and `modelcontextprotocol/conformance` - already in
  `docs/research/LADDER.md` line 70.
- `tugcantopaloglu/godot-mcp`, `Coding-Solo/godot-mcp`, `chongdashu/unreal-mcp`,
  `ahujasid/mcp-for-blender` - named in RULES.md, skipped.
- `mxgmn/WaveFunctionCollapse` and `krychu/wfc` - WFC already landed (`crates/world/src/wfc.rs`,
  LADDER line 282), so no new row.
- `tune.sweep` (D20) needs no donor - `crates/playtest/src/tournament.rs:1609`
  `sweep_difficulty_curve` and `:1658` `render_sweep_csv` already produce
  `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials`, and
  `crates/editor/src/tuning.rs` holds the staged undoable tune. Only the door verb is missing.
- Agent memory (D18, first half) is not `crates/ai/src/memory.rs`: that file is per-NPC mind
  memory inside the tick, not a project memory the door serves. Row 12 is the donor.
- `zacklocx/shader-agent` (MIT, read live 2026-10-20) - best free-model route found (Ollama
  plus deterministic offline GLSL drafts and license-aware reference filtering), but its target
  is GLSL/Shadertoy/LÖVE 2D while our material stack is WGSL; the transferable parts
  (reference licence filter, preflight gate) can be re-mined from
  `shader_agent/reference_index.py` and `shader_agent/quality.py` when a shader row opens.
- `dcc-mcp/dcc-mcp-houdini` (MIT) and `dcc-mcp/dcc-asset-geospatial` (MIT) - Houdini is
  proprietary and paid, so the host path is research-only; the geospatial skills pull OSM
  (ODbL, share-alike on derived databases) and Overture over the network, which our asset
  provenance gate would have to police. Keep both as reads, not ports.
- `waynchi/gamedevbench` (Apache-2.0, read live 2026-10-20) - 333 tasks, but its leaderboard
  numbers need paid models and its harness is Godot-specific; `gamedevbench/src/provider_proxy.py`
  (allowlisted provider domains on port 443 only, behind a network namespace with no route to
  the host) is the one reusable idea, and it is a security idea, not an AI-native one.
- `adrian-wulf/gamewache` (MIT, read live 2026-10-20) - deterministic blackbox input replay
  (`.gwrec`), crash sentinel with engine tick and spatial position, live parameter tweaker with
  min/max/step. All three overlap what our tape, trace export and staged tune already do.
- `SylphxAI/repomap` (MIT, read live 2026-10-20) - change impact with direct callers, depth-2
  reach and the test file to run; same shape as ripwire row 9 and atlas row 10, one of the
  three is enough.
- `MalloyTheDev/aseprite-mcp` `src/aseprite_mcp/tools/` (163 tools) - the tool count and the
  batch runner are the one thing we must not copy: our tool list is already 66,500 chars
  (`crates/endpoint/src/mcp.rs:148` `TOOL_LIST_BUDGET_CHARS`).
- `Fulviuus/defold-mcp` (LICENSE file read gave no text on 2026-10-20, so the licence is
  unknown) - dropped on licence grounds alone.

## Top 3

1. `GamePhanesStudio/GamePhanes` - https://github.com/GamePhanesStudio/GamePhanes - MIT -
   closes D19 and D18 (programmatic referees, no LLM judge) and feeds HS20.
2. `wd30x/loom-engine` - https://github.com/wd30x/loom-engine - MIT/Apache-2.0 - closes D19
   (every edit re-runs the recorded session) and D16 (every format as JSON Schema).
3. `dcc-mcp/dcc-mcp-core` - https://github.com/dcc-mcp/dcc-mcp-core - MIT - closes D16
   (MCP resources plus the measured list projection that shrinks a 66,500-char tool list) and
   AX28 (one job status vocabulary with disk-backed counters).
