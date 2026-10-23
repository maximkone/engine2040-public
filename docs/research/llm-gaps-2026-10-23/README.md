# LLM / agent-side gap audit (2026-10-23)

**Verdict.** The MCP door is real and better than most engines: 99 verbs, typed refusals, undo,
resources, prompts, progress, push. The model side is a library, not a runtime: no shipped binary
ever sends a model request, and the request type cannot carry scene content. So every "AI" number in
the tree is a fixture; nothing here has been measured against a real scene.
Third line: the agent loop, router, prompt assets, quota and reader are built and have no consumer
(`git grep -rln` finds one file each); the door's own call log has no arguments, so an agent run
cannot be replayed.

Counts: 99 reachable verbs = 13 core + 86 pack verbs in 19 packs (95 under the test feature set);
35 prompts (30 skills + 5 scenarios, 0 arguments); 5 resources (4 roots + live scene) + 4 templates;
7 provider adapters (mock, local, remote, OpenAI, Anthropic, Gemini, Ollama); 41 eval-harness tests
(25+8+5+3) over 25 tasks × 3 backends = 75 rows; 1 committed LLM cassette (a 50-question × 2-arm
TSV, one free model) and 0 provider cassette files; 291 provider + 935 endpoint + 521 playtest +
110 nn/distill test fns. No cargo line was run for this audit; every claim is a path or a grep.

## Capability matrix

Class: **A** missing totally · **B** exists, no consumer · **C** wired and tested · **N** deliberate no

| # | Capability | Standard 2026 | Ours | Class | Smallest build | Risk |
|---|---|---|---|---|---|---|
| 1 | Content in a model request | prompt carries the scene brief / images | `provider/vendor.rs:442` sends `task`, `revision`, digest hex only; `pins.rs:55` tells the model "never invent an object id you were not given" | **A** | one bounded `body` field + `scene.brief` feed | no live model can act; all numbers fake |
| 2 | Tools + annotations + outputSchema | `readOnlyHint`, `outputSchema`, `structuredContent` | `endpoint/packs/mod.rs:149` (annotation required), `mcp.rs:293` (schema in budget), `play.rs:1447` (image block) | C | – | – |
| 3 | Strict structured output | `json_schema`, forced tool, repair | `provider/ladder.rs:1-23` four rungs, `vendor.rs` strict parser | C | – | – |
| 4 | Parallel tool calls | client sends many; server answers concurrently | one `Mutex<ServerState>` (`mcp.rs:1082`) serializes every call | B | read-only path without the lock | multi-agent waits |
| 5 | Resources, templates, subscribe | list/read/templates/subscribe + listen | `resources.rs:36-56`, `mcp.rs:1622-1732` | C | – | text/plain only |
| 6 | Prompt arguments + completions | `arguments`, `completion` | 35 prompts, `get_prompt` ignores args (`mcp.rs:1776`), `completions` absent (0 hits) | B | parse args, one completion hook | prompts are static text |
| 7 | Progress | monotonic `progress` + `total` + message | `mcp.rs:1250`/`:1258` send 0.0 then 1.0, no total | C | `with_total` + real ticks | long builds look hung |
| 8 | Cancellation | `notifications/cancelled` | `mcp.rs:1726` `context.cancelled()` on listen; jobs have cancel | C | – | – |
| 9 | Pagination | `cursor` on every list | `scene.read` cursor; `tools/list` `next_cursor: None` (`mcp.rs:1604`) | B | cursor over sorted names | grows past 75k chars |
| 10 | Transport + auth | streamable HTTP + OAuth 2.1 | stdio only (`bin/e2040-mcp.rs:77`); remote shim is in-process (`remote_shim.rs:1`) | **A** | HTTP transport + token gate (`transport.rs` has the gates) | hosted agents cannot reach it |
| 11 | Long-running tasks | MCP `tasks` primitive | app-level `play.run` job start/poll/cancel; no `tasks` (0 grep) | B | map job state to a task | AX28 open, non-standard |
| 12 | Model call in a shipped path | provider SDK / agent loop | 0 hits for `AiRequest`/`ProviderAdapter` in `crates/game` and `crates/editor`; only playtest eval and fixtures | **A** | one call site + one door verb | "AI-native" unproven |
| 13 | Bounded loop over door tools | plan-act with budget, retries | `provider/agent_loop.rs:6` "no in-repo consumer"; `endpoint/assistant.rs` 39 tests, no caller | B | wire to stdio door | exists on paper only |
| 14 | Eval harness with datasets | tasks, pass@k, baseline diff | `playtest/eval.rs` 25 tasks × 3 backends; `eval/scorecard.rs:1-7`; trace import/export | C | run it in CI against a baseline | mock-only rows |
| 15 | Win rate for model tasks | model A vs B over N seeds | win rates exist for bots (`playtest/winrates.rs`, `packs/sweep.rs`); 0 `win_rate` hits in eval | **A** | one tournament arm per model | model quality unranked |
| 16 | Cassettes / replay of a live model | recorded + replayed in CI | `provider/cassette.rs:1-24`, 24 tests; 0 committed provider cassettes; 1 spatial TSV | B | record one model × one task set | stale on model change |
| 17 | Cost / token accounting | per call cost + budget cap | `protocol.rs:96-135` (`Usage`, `Budget`), `quota.rs`, scorecard cost | C | wire `QuotaBook` to adapters (no consumer) | spend invisible |
| 18 | Model routing / fallback | route by class, fall back | `router.rs:1-17` (scorecard) + `fallback.rs`; fallback used only in `playtest/door.rs:293` | B | wire `router.route` into adapter choice | one model for all tasks |
| 19 | Prompt versioning | versioned prompts + eval gate | `provider/prompt.rs` `PromptAsset`/`score_permille`, 11 pub fns, no consumer | B | gate the shipped system prompts | prompt edits untested |
| 20 | Memory across sessions | durable typed memory | 0 hits; `agent_loop.rs:270` `MemoryDoor` is a test door; OPT49 open | **A** | OPT49 `provider/memory.rs` | every run starts blind |
| 21 | RAG / embeddings | embed + retrieve | `policy.rs:22` boolean only; 0 hits for vector/retrieve in provider | **A** | local embed store | no project recall |
| 22 | Vision input | image parts in the request | 0 image hits in adapters; `VISION_MODEL_PREFIX` only gates (`core.rs:175`); door *outputs* PNG (`play.rs:1447`) | **A** | image part + base64 bound | cannot see its own frame |
| 23 | Voice in/out | STT/TTS | none (`audio` is synthesis only) | **A** | out of scope | – |
| 24 | Local inference | llama.cpp/Ollama sidecar | `ollama.rs`, `local.rs`, `arbiter.rs` (16 tests), `game/pressure_watch.rs:18` feeds VRAM | C | bind one real local model | GPU share untested live |
| 25 | Guardrails / red team | injection + adversarial suite | `injection/mod.rs` 5 categories, 12 tapes + 10 evasions, `sandbox.rs:1-8`, `tool_capabilities.rs:118` | C | claim gate AD21 | a claim can be unbacked |
| 26 | Sandboxed code execution | exec box | none by design: no exec verb, `packs/mod.rs:143` forbids the words | N | – | – |
| 27 | Traces / OpenTelemetry | spans, replayable trace | `audit/mod.rs:1-15` in-memory chain (1024, no args); 0 OTel hits | B | persist audit with args | cannot replay a session |

## MCP surface

What the door actually answers (13 core, `mcp.rs:169`, plus packs):
`scene.read`, `tx.preview`, `tx.apply`, `tx.undo`, `play.run`, `capture`, `api.search`, `api.describe`,
`api.call`, `api.load`, `test.run`, `sim.build_encounter`, `game.build`; packs add 86 more
(`scene.*`, `tx`, `play.step`, `seat.*`, `view.*`, `gym.*`, `anim.*`, `material.*`, `asset.*`,
`physics.*`, `net.*`, `perf.*`, `web.*`, `rules.*`, `session.*`, `codex.*`, `replay|verify.*`).

- Present beyond tools: 5 resources + 4 templates (`resources.rs:36-56`), 35 prompts (`skills.rs:1`),
  `tools/list_changed` + `resources/subscribe` + `subscriptions/listen` (`mcp.rs:1543-1550`, `:1702`),
  progress (`:1250`), cancellation (`:1726`), typed refusals with recovery (`mcp_refusal.rs`).
- Sampling is **absent on purpose** (`docs/AI-NATIVE.md:243`): the 2026 spec deprecates it, and the
  engine never borrows the client's model. Not a gap.
- Missing: prompt `arguments` and `prompts/complete` (`mcp.rs:1776` reads name only); tool/resource
  pagination (`:1604`); `roots`, `elicitation`, `logging/setLevel`, `tasks` (all 0 hits); any
  non-stdio transport and any auth (`Cargo.toml:203` pulls `transport-io` only).
- The one measurement that disagrees with itself: `docs/book/door-reference.md:5` prints
  "13 core tools and 86 verbs in 19 packs" and `:3294` "99 tools parsed", while the code pin says
  95 + feature deltas (`tests/it/tool_count_pin.rs:57`). Both are right for their feature set, but
  the page is **stale today**: `node tools/door-reference.mjs --check` prints
  `docs/book/door-reference.md is stale` (exit 1). One command fixes it; nobody ran it.

## Model runtime

- The contract is a privacy envelope, not a prompt: `AiRequest` carries only digest refs
  (`protocol.rs:83-126`), and `vendor.rs:442-459` renders `task=… revision=… contexts=N digest=…`.
  `pins.rs:55` says so to the model. A live adapter therefore cannot name a real object id.
- Even the eval fabricates it: `playtest/src/door.rs:276-280` passes `digest: [3u8; 32]`. So a "live"
  eval row asks a real model about nothing. This is the single biggest hole.
- Content-bearing code exists but is unwired: `provider/reader.rs:1-18` (capture + brief → recorded
  read) has no caller outside itself.
- Real parts, proven by tests: four-rung ladder, strict grammar, two retries with a shared 429 gate
  (`retry.rs:1-15`), 7 adapters behind one conformance suite, `Usage` that never claims unknown as 0.
- Unwired islands: `router.rs` (scorecard routing, 0 consumers), `prompt.rs` (versioned prompt asset
  + eval gate, 0 consumers), `quota.rs` (keys, limits, receipts, 0 consumers), `assistant.rs`
  (39 tests, no binary, no door verb, `backend.rs:3-17` reads fixtures only).
- The running game uses provider only for VRAM pressure types (`game/pressure_watch.rs:18-22`);
  its announcer is a static table (`game/src/announcer.rs:129`). No live line, no live model.

## Agent loop and eval

- `provider/agent_loop.rs` is a correct bounded loop (8 default steps, 2000 tokens, stuck/unknown
  refusal, transcript checkpoint) with 6 tests and, by its own header, no consumer. PV7 landed the
  harness; nothing loops against the door.
- `playtest/eval.rs` is the strongest AI asset in the tree: 25 tasks × 3 backends, 75 rows,
  trajectory check (read before write, `eval/trajectory.rs:1-9`), utility, retrieval dilution,
  trace export/import with a version, scorecard with per-suite pass rate and tokens.
- But: every row runs mock/local-mock by default; live is opt-in (`E2040_EVAL_LIVE`, `door.rs:1-20`)
  and, per above, contentless. No pass@k arithmetic, no `win_rate` in eval, no committed baseline
  card, no CI gate. `scorecard.rs:6` is an `--ignored` test.
- Narrated replay exists once: `fixtures/spatial-gold/llm/minimax-m3.1-flash-free.tsv` (100 rows,
  v1 format, `cargo test -p engine-playtest --lib spatial_llm_replay`). `docs/plan/spatial-llm-ab.md:3`
  still says "no model figure exists yet". That file is the only real model evidence in the repo.

## Memory and RAG

- Nothing durable: no vector store, no embedding call, no cross-session record (0 hits for
  `embedding_model`, `vector_search`; `policy.rs:22` is a capability bool).
- OPT49 is the open row: typed memory with a write gate and the tape hash it came from. Until it
  lands, `agent_loop` "remembers" the whole transcript in RAM and drops it at process exit.
- The door can emit the raw material (`scene.diff`, `session.activity`, `ai.decisions`), but no verb
  reads history back into a prompt context.

## Multimodal

- Outbound images work: `play.step` and a wired capture return a PNG content block (`play.rs:1447`,
  covered by a test at `:2270`). `look.judge` scores pixels (9 items, SSIM included).
- Inbound images do not: 0 hits for `image_url`, `input_image`, `ContentBlock::Image` in
  `crates/provider`. `VISION_MODEL_PREFIX` only lets a scripted test pretend (`core.rs:175`).
- No model-as-judge: ARX05 is open; `look.judge` is arithmetic, not a VLM. No voice anywhere.

## Local inference

- The strongest local story in the tree, and still not exercised end to end: `OllamaAdapter` +
  `LocalAdapter` (loopback sidecar) + `Arbiter` (GPU/CPU/deferred/remote under the game's p95 and
  VRAM feed) + `nn` int8 `.e2nn` runtime + `distill` int8 student export (burn-ndarray, CPU only).
- `nn/src/lib.rs:3-5` says no tick binds a net yet; `distill` "any teacher" is a trait, and 0 hits
  for `engine_provider` in `crates/distill`, so no provider→distill bridge exists.

## Safety and guardrails

- Strong: typed refusal + recovery on every call; capability cards; tier gate and payload scan
  (`sandbox.rs:1-8`); 5-category injection screen wired into `scene.place`; 12 hostile tapes plus 10
  crafted evasions; append-only audit chain with a pinned head.
- Weak: the audit record keeps tool, revision, outcome and detail, **not the arguments**
  (`audit/mod.rs:147`), and it is in memory only (cap 1024). So "what did the agent do" is answerable
  in aggregate, not re-runnable — the PF9/HX3 hole is still open.
- Missing: a claim gate (AD21: an agent's "landed" is not checked against a journal key), a
  per-caller capability token (EA6), and any check on model *output* beyond the proposal grammar
  (no moderation, no banned-content screen).
- Sandboxed execution is not a gap: the door has no exec/shell/file verb, and the pack registry
  refuses those words by name (`packs/mod.rs:143`).

## Docs freshness

- `llms.txt` is path-clean (`node --test tools/for-ai.test.mjs` 6/6 pass) but thin: it never names
  the provider crate, the eval harness, cassettes, resources/prompts, or `docs/AI-NATIVE.md`;
  `docs/systems/ai-door.md` is not linked at all. An AI landing there learns the door, not the AI stack.
- `docs/systems/ai-door.md:8` still lists MCP16 and PV7 as open; both rows are ticked
  (`node tools/board-slice.mjs MCP16|PV7` shows `[x]`). Stale status line.
- `docs/book/door-reference.md` fails its own freshness check (above). Rows BG1 and AD9 describe
  code that is present (`mcp.rs:1543`, `view.rs` `door.status`) but are still open — board hygiene,
  not a capability gap.

## Top 5 this month

1. **Make the request carry content** (new row; nothing on the board covers it). Add one bounded
   body/brief field to `AiRequest`, feed it `scene.brief`, and prove one real model call against the
   arena with a committed cassette that replays byte-identical. Why first: without it every other
   AI claim is a script. Cost S-M. Kill line: a body over its byte cap, or a replay that differs.
2. **One eval gate in CI with a real number** (D18). Run the 75-row matrix on the runner, write the
   scorecard, diff it against a committed baseline, add pass@k, and treat `unknown` as not-pass.
   Why: the harness is done and unused; a number nobody reruns rots. Cost S.
3. **Wire the loop and the call log** (PV7 output; PF9/HX3). `agent_loop` over the stdio door on 20
   tasks, the door writing `{method, args, revision, receipt}` per call to a file the engine replays
   with one step per tick. Why: it is the only way an agent run becomes evidence, and both halves
   exist unwired. Cost M.
4. **Typed memory with a write gate** (OPT49, open). Facts keyed by the tape hash they came from,
   one ablatable layer per test, retrieval that does not resend the transcript. Why: today a session
   is amnesiac, and the prompt grows monotonically. Cost M.
5. **Inbound vision** (ARX05 for the judge half). Send the capture as an image part, bound the bytes,
   and let one model judge a frame set against a rubric. Why: the door already shows frames; the
   model side cannot see one. Cost M.

Runner-up (small, do with #1): prompt `arguments`, `prompts/complete`, tool-list pagination, and an
honest `progress` total — all one-line additions at the door.

## Not gaps (do not "fix")

Sampling (deprecated, `AI-NATIVE.md:243`), code execution (refused by design), a model inside the
tick (the engine's bet), and vendor SDK types in contracts. Keep those.
