# GH-M: what a game engine should expose as tools to an agent, and how it stays deterministic

**Lane answer (short).** Every engine that has actually shipped a door for an agent ends up with the
same three answers, and none of them is "one verb per action". (1) The door stays small and the
*engine* grows the list: the game registers its own typed commands at runtime
(`Ladvien/bevy_debugger_mcp`, a small fixed set - its README says seven tools, OBSERVED) or the operations live as *actions* behind one
tool (`satelliteoflove/godot-mcp`: 21 tools, 86 actions, OBSERVED). (2) The read side and the write
side are two different surfaces, not one surface with annotations: a mutating call is only a success
when its per-item result list is clean *and* a readback agrees (`Chrrxs/robloxstudio-mcp`,
OBSERVED), and one engine ships a read-only edition where the mutating verbs are simply absent
(OBSERVED). (3) Determinism comes from the clock and the log, not from the model: the game is frozen
between two tool calls and stepped in bounded slices with the inputs riding inside the step window
(`satelliteoflove/godot-mcp`, OBSERVED), and the agent's own calls are recorded as a file the engine
replays itself, one step per frame, so that replaying a session and saving it gives back the same
file (`Ladvien/bevy_debugger_mcp` `scene.rs`, OBSERVED). Our tree already has the pieces of (3) on
the input side — the seat records validated inputs so an agent run is replayable
(`crates/game/src/agent_seat.rs:8-9`, OBSERVED) — but it has **no log of door calls at all**
(`git grep -n -i -E "fn (record_call|journal_push|push_call|log_call)" -- crates/` = no hits today,
OBSERVED), so the engine cannot re-run what the agent did. That is the hole the top row closes.

Row ids: `OPT29`, `AX23`, `AX28`, `PF9`, `EG35` and `D19` are on the board; `SP62`, `HX3` are board
rows in `docs/SPRINT-18-gaps.md` / `docs/SPRINT-29-hunt.md` (prefixes outside the lane list, named
anyway because they are real rows); `D16` is closed (2026-10-21) so it is not used. Every donor
licence was read live today from the file at the repo root. No donor bytes are vendored: we port the
idea and write our own code. Donor benchmarks and donor numbers are the donor's own words, never our
runtime result.

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A small fixed tool set where the *game* registers its own typed commands, and a mistyped command name is answered with the nearest real one | `Ladvien/bevy_debugger_mcp` https://github.com/Ladvien/bevy_debugger_mcp read 2026-10-21 (README.md, `crates/bevy_debugger_bevy/src/` listing read live) | MIT OR Apache-2.0 (LICENSE-MIT + LICENSE-APACHE read live 2026-10-21) | OPT29 (agent onboarding budget), AX23 | `crates/bevy_debugger_bevy/src/commands.rs` (the typed command registry; the file itself was **not** read today - the README states every command carries a required `example` string, validated at registration) and README.md ("the same seven tools drive any game that registers some"; the plugin registers `bevy_debugger/status`, `commands`, `screenshot`, `capture`, `scene`, `time`, `quit`, `input`, `guide`, `guide+watch` and lets a game add its own via `add_debug_command`) | M | If our door must publish one verb per game action to keep every schema typed, the seven-tool shape dies and we keep the flat verb list | PORT |
| 2 | The agent's run is a file the *engine* replays itself, one step per frame, and the file is a fixed point (replaying a session and saving it gives the same file) | `Ladvien/bevy_debugger_mcp` https://github.com/Ladvien/bevy_debugger_mcp read 2026-10-21 (`crates/bevy_debugger_bevy/src/scene.rs` read live) | MIT OR Apache-2.0 (read live) | PF9 (replay visual debugger), HX3 (the AI bill of materials). PF9's rerun sink exists (`crates/observe/src/rerun_sink.rs`) but records the *sim*, not the agent's door calls | `crates/bevy_debugger_bevy/src/scene.rs`: `SceneStep {method, params}`, `load()` (a wrong `format`/`version` is refused with one error line and nothing replays, never a panic), `start_playback` (`BEVY_DEBUGGER_SCENE` env), `replay_next_step` (one step per frame, only while the world says it is ready), `run_step` (a step that is not a *recorded* command is refused, "so a scene file cannot call a read-only query") | M | If a door call cannot be recorded as `{method, params}` without also snapshotting the world, or if replaying the log does not give back the same tape hash, the row dies | PORT |
| 3 | A mutating call is a success only when its per-item result list is clean **and** a separate small readback agrees; transport 200 is not success; never retry a batch blindly | `Chrrxs/robloxstudio-mcp` https://github.com/Chrrxs/robloxstudio-mcp read 2026-10-21 (`docs/large-inputs.md` read live) | MIT (LICENSE read live 2026-10-21) | D19 (verifier output in every build verb), AX23 | `docs/large-inputs.md`: "Successful execution and successful result delivery are separate facts. Keep mutation responses small and verify effects with a separate small readback."; "HTTP 200 or a normal MCP result is not all-writes success. Require `summary.failed === 0` and inspect every per-property entry in `results`, then read back the intended values."; the protocol error `-32600` with `code`, `bytes`, `limitBytes`, `transportStage`, `executionOutcome: "not_executed"`; limits counted in **encoded bytes** (80 MiB stdio line, 50 MiB HTTP body, 64 MiB transport, 16 KiB envelope reserve), not source characters | M | If our mutating verbs cannot return a per-item outcome list plus a readback, the rule has nothing to grade and the row dies | PORT |
| 4 | The same door shipped as a **read-only edition**, where the mutating verbs are absent rather than annotated | `Chrrxs/robloxstudio-mcp` https://github.com/Chrrxs/robloxstudio-mcp read 2026-10-21 (README.md read live) | MIT (read live) | no row, propose a new row **DOOR-READONLY** (a door build whose published surface cannot mutate) | README.md ("Inspector edition ... Read-only access to the DataModel. No DataModel or script edits ... Install only one variant at a time") | S | If our read-only profile still publishes the mutating verbs (even with `read_only_hint`), the split is decoration and the row dies | PORT |
| 5 | One `verify` call: recompile, extract the errors, read the console errors; and one authoring call = exactly one undo step | `isuzu-shiranui/UnityMCP` https://github.com/isuzu-shiranui/UnityMCP read 2026-10-21 (README.md read live) | MIT (LICENSE read live) | D19 | README.md: "`verify` (再コンパイル → エラー抽出 → コンソールのエラー)"; "オーサリングのツールは、呼び出し 1 回が Undo 1 操作にまとまります" (one authoring call is one undo step); the tool attribute `Idempotency = McpIdempotency.Safe` on the example tool | S | If our build verbs cannot run their own check in the same call, the row dies and D19 falls back to a separate verb | PORT |
| 6 | The engine process serves MCP itself (no second process, no marshalling), so a tool call cannot observe a half-stepped world | `isuzu-shiranui/UnityMCP` https://github.com/isuzu-shiranui/UnityMCP read 2026-10-21 (README.md read live) | MIT (read live) | SP62 (`view.observe` and `view.laws`), EG35 (a render door) | README.md ("MCP クライアントは、Editor 自身が公開する Streamable HTTP エンドポイント ... 別プロセスの MCP サーバーはありません") and `docs/architecture.md` (named in the README, not read today) | M | If the door must stay a separate process (our `e2040-mcp` is stdio in its own crate and the seat is a socket), this is a note, not a port | RESEARCH-ONLY (our seat's tick window already blocks a stale read: `crates/endpoint/src/packs/play.rs:344` "The window ran past the tick before its input arrived") |
| 7 | A tool list that shrinks to the group the client asks for, and tools that appear only when the capability exists | `isuzu-shiranui/UnityMCP` https://github.com/isuzu-shiranui/UnityMCP read 2026-10-21 (README.md read live) | MIT (read live) | OPT29, AX28 | README.md: "MCP の URL に `?group=diagnostics,authoring` のようにグループを付けると、`tools/list` がそのグループだけを返します"; and the conditional surface (the Timeline tools appear only if `com.unity.timeline` is installed, `test_run` only if the test framework is) | S | If `tools/list` cannot be filtered per client, or if a capability-gated tool still has to be published to be usable, the row dies | PORT |
| 8 | The door lease: one client at a time, the newcomer is refused instead of displacing the incumbent, and a silent incumbent is declared stale so the next client takes over | `satelliteoflove/godot-mcp` https://github.com/satelliteoflove/godot-mcp read 2026-10-21 (`docs/architecture.md` read live). **CORRECTION**: this repo is the donor behind our LF16 trio, named in `docs/plan/ai-door.md:130` and `docs/SPRINT-18-gaps.md:1647`; the facts below are new | MIT (LICENSE read live) | no row, propose a new row **SEAT-LEASE** (one agent at a time; a silent agent loses the lease) | `docs/architecture.md`, section "One client at a time": the second client is rejected with WebSocket close code `4001` instead of hijacking the session; the incumbent is stale after **45 s** of silence or a dropped TCP connection; rejected clients retry with backoff | S | If our seat keeps only "one client at a time" with no expiry, a crashed agent can hold the seat forever; if we cannot tell silence from a live agent, the row dies | PORT |
| 9 | The game clock is frozen between two tool calls and stepped in bounded slices; `step_until` re-evaluates a predicate every frame and stops the frame it holds, with a safety cap | `satelliteoflove/godot-mcp` https://github.com/satelliteoflove/godot-mcp read 2026-10-21 (`docs/architecture.md` read live). **CORRECTION**: the trio is ours (LF16, ticked), the freeze/thaw pair and the freeze-over-the-game's-own-pause rule are not | MIT (read live) | AX28, SP62 | `docs/architecture.md`, section "Deterministic game time": "freeze / thaw pause and resume under agent control, layered correctly over the game's *own* pause state"; "step advances a bounded slice of game time (or an exact frame count), then re-freezes"; "step_until re-evaluates a GDScript predicate every frame and stops the frame it holds, with a safety cap"; "Input timelines ride *inside* the step window"; "between any two tool calls, the game is exactly where the agent left it ... observation never races gameplay" | S | If the live seat cannot be frozen (only tapes can be replayed), the half dies and only the freeze/thaw wording survives; if `step_until` cannot be given a general predicate over the live bout, that half dies | PORT (the inputs-inside-the-window half is already ours: `crates/endpoint/src/packs/play.rs:107` `inputs: Option<Vec<PlayInputDto>>`) |
| 10 | Reflection-generic verbs plus discovery verbs, so the door stays small as the content grows; and a write that does not compile is refused before it lands | `beckettlab/beckett-godot-mcp` https://github.com/beckettlab/beckett-godot-mcp read 2026-10-21 (README.md read live) | MIT (LICENSE read live) | OPT29, D19 | README.md: "exposes **reflection-generic** tools that work on *any* class via `ClassDB` - instead of hundreds of hand-coded per-domain wrappers (an anti-pattern: LLMs degrade past ~40 tools)"; the discovery verbs `find_classes` / `describe_class` / `find_methods` followed by `describe_object` / `set_property` / `call_method`; "validate-before-write": `write_script` / `script_patch` "parse the code first and refuse to write what doesn't compile (closing the #1 AI-on-Godot failure: hallucinated GDScript)" | M | If our `api.search` / `api.describe` / `api.call` cannot reach a new content type without a new verb, the reflection half dies; if a build verb cannot parse before it writes, the validate half dies | PORT (the reflection verbs already exist in our tree: `crates/endpoint/src/tools.rs` `api.call`, `api.describe`, `api.search` — the new part is validate-before-write and the donor's "~40 tools" figure) |
| 11 | Six tools and one typed dispatch, so the published count stays fixed while the actions grow; and a server `instructions` string that names the loop | `Roblox/studio-rust-mcp-server` https://github.com/Roblox/studio-rust-mcp-server read 2026-10-21 (`src/rbx_studio_server.rs` read live). **CORRECTION** of `docs/research/hunt-2026-10-21/agent-4-ai-door.md:133`, which dropped this repo as a "verb-count arms race": it publishes **six** tools, and the count is the point | MIT (LICENSE read live, "Copyright 2025 Roblox Corporation") | AX23, OPT29 | `src/rbx_studio_server.rs`: `ToolArgumentValues` is one enum and every tool funnels into `generic_tool_run` (one FIFO queue + one output map + a UUID per call, so a caller can always be answered); the `ServerInfo.instructions` string names the loop and the order ("You must aware of current studio mode before using any tools ... Prefer using start_stop_play tool instead run_script_in_play_mode"); `get_studio_mode` is the gate every other verb needs | S | If one dispatch behind our typed verbs cannot hold our per-verb schemas and receipts, the row dies and the flat verb list stays | PORT (Rust + rmcp, the same stack as ours) |
| 12 | A threat model that names every guard layer "an accident guard, not a security boundary", lists the known bypasses next to each layer, and keeps an append-only machine audit log | `wgt19861219/godot-mcp-enhanced` https://github.com/wgt19861219/godot-mcp-enhanced read 2026-10-21 (`docs/THREAT_MODEL.md`, `LICENSE` read live). **CORRECTION** of `docs/research/hunt-2026-10-21/agent-4-ai-door.md:129`, which recorded the licence as NOASSERTION: the root `LICENSE` is **MIT** (read live today) | MIT (LICENSE read live 2026-10-21). **Risk line**: that LICENSE carries third-party notices and names one ported slice (an untrusted-envelope and file_guard mechanism, under 50 lines, credited to `NPGameDev/godot-mcp-server`) whose licence it marks unconfirmed - so that slice is ideas-only for us, the document shape is fine | HX3, no row, propose a new row **DOOR-TRUST** | `docs/THREAT_MODEL.md`: the trust table (who is trusted and how much), the per-layer verdict ("accident guard", "accident guard + local single-user hard isolation", "not a security boundary"), the listed bypasses (string-concatenation past a regex blacklist, reflection, the TOCTOU window between realpath and the write), and the machine audit log (`~/.godot-mcp/machine-audit.jsonl`, atomic append) | S | If a layer of ours is really a security boundary, this document lies about it; the row dies unless every layer we claim is an accident guard | PORT (document shape only) |

## Rejections

- `IvanMurzak/Unity-MCP` (Apache-2.0, LICENSE read live 2026-10-21) - already on the skip list
  (`docs/research/steals-2026-10-18/05-ai-native.md:4`); its "AI skills + CLI" shape overlaps the 33
  skills we already publish as MCP prompts.
- `IvanMurzak/Godot-MCP` - covered by the same "IvanMurzak" entry on that skip list, same reason.
- `AnkleBreaker-Studio/unity-mcp-server` - LICENSE read live: "AnkleBreaker Open License v1.0", not a
  standard permissive licence; and it advertises 268 tools.
- `CoderGamester/mcp-unity` - the licence file is `LICENSE.md` (not `LICENSE`) and it is MIT (read live
  today); nothing past the licence was read, so no mechanism is claimed here.
- `FunplayAI/funplay-unity-mcp` (MIT, LICENSE read live) - the headline tool is `execute_code`;
  arbitrary-code verbs are refused by our design (`docs/research/steals-2026-10-12/aidoor.md` item 5).
- `youichi-uda/unity-mcp-pro` (147 tools) and `youichi-uda/godot-mcp-pro` (162 tools) - the same
  verb-count arms race rejected in `docs/research/hunt-2026-10-21/agent-4-ai-door.md:133`.
- `NPGameDev/godot-mcp-toolkit` (MIT, LICENSE read live) - the "always-on core plus on-demand groups"
  is what our packs already do; the "150+ operations / 112 tools" split is a donor count, not a
  mechanism we lack.
- `regiellis/godot-mcp-go` (MIT) - the same surface in Go; no new mechanism.
- `yurineko73/Godot-MCP-Native` (MIT) - the in-process door again, in GDScript; row 6 covers it.
- `killop/puerts-unity-mcp` (MIT) - a JS runtime inside the Unity editor so tools can be written in
  TypeScript; our door is Rust, so nothing to port.
- `remiphilippe/mcp-unreal` (Apache-2.0, LICENSE read live) - Unreal is **PROPRIETARY**: research
  only, nothing read past the licence.
- `kinkirill/bevy_manual_mcp` (MIT) and `Nihilantropy/godot-mcp-docs` (MIT) - the engine manual and
  the engine docs as MCP tools; we already serve `e2040://crate-map` and `e2040://door-reference`
  (`docs/plan/finish-line.md`, 2026-10-21 progress), so no gap.
- `Erodenn/godot-mcp-runtime` (MIT) - a "zero-footprint" runtime driver; no mechanism we lack.
- `MineDojo/Voyager` (MIT, LICENSE read live; `voyager/env/bridge.py` and `voyager/agents/critic.py`
  read live) - already known and skipped (`docs/research/steals-2026-10-18/05-ai-native.md:4`,
  card C3). Its action space is *code*, which our door refuses.
- `balrog-ai/BALROG` (MIT) - already known and skipped; it is a benchmark client over six games,
  not an engine door.
- `SWE-agent/SWE-agent` (MIT) and `UKGovernmentBEIS/inspect_ai` (MIT) - already known and skipped
  (same file, same line).
- `All-Hands-AI/OpenHands` (MIT, LICENSE read live) - the `openhands/events` path did not resolve on
  the contents API today, so nothing past the licence was read; not re-mined.
- `google-deepmind/melting-pot` - the repository did not resolve under `farama-foundation` today; not
  read, so not a claim.

## Top 3

1. `Ladvien/bevy_debugger_mcp` - https://github.com/Ladvien/bevy_debugger_mcp - MIT OR Apache-2.0 -
   closes **PF9** and **HX3**: the agent's calls are a file (`format`, `version`, `steps[]`) the engine
   replays itself one step per frame, a replayed session saves back to the same file, and a step
   that is not a recorded command is refused so the file cannot carry a read-only query. Our tree has
   no call log at all today, so this is the row that makes the door's own history replayable.
2. `Chrrxs/robloxstudio-mcp` - https://github.com/Chrrxs/robloxstudio-mcp - MIT - closes **D19** and
   **AX23**: a mutating call is a success only when its per-item result list is clean and a separate
   small readback agrees; transport success is not success; and the same door ships as a read-only
   edition (new row **DOOR-READONLY**) where the mutating verbs are absent rather than annotated.
3. `isuzu-shiranui/UnityMCP` - https://github.com/isuzu-shiranui/UnityMCP - MIT - closes **D19**
   (`verify`: recompile, extract errors, read the console, in one call; one authoring call is one
   undo step) and **OPT29** (`tools/list` filtered by group; tools that exist only when the
   capability exists).
