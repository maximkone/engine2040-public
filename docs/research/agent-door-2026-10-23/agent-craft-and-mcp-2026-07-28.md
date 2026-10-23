# Agent craft and the e2040 door, wave 2026-10-23

Sits beside the sibling card in this folder (`README.md`, three shipped-engine
steals). That card asks what shipped engine MCP repos have that we do not. This
one asks a different question: what do **OpenChamber, the MCP 2026-07-28
revision, the agent papers with real numbers, and local serving** hand us, and
which of those fit a weak model driving `e2040-mcp`.

Method: every claim about our tree is a grep or a read at `9a6289a57` (the
commit this card was written on), quoted with its path and line. Every claim
about the outside world is a page read on 2026-10-23, URL given. Licences were
read from the repo's own licence file, not from memory.

Labels: **measured** = read from our file or a fetched page; **donor claim** =
the page's own sentence, no number of ours; **third-party report** = somebody
else's benchmark on their machine; **estimate** = arithmetic on measured
numbers, not a run.

## The one-line answer

Our door is already on the current protocol (`V_2026_07_28` at `mcp.rs:1456`,
`server/discover` tested in `tests/it/mcp_stdio.rs:240`), so the gap is no longer
"which spec revision". The gap is **trust and cost**: the door has no way to ask
the caller one question mid-call (`InputRequired` 0 hits in `crates/endpoint`),
no way to hand back a large receipt by reference instead of truncating it
(`ResourceLink` 0 hits, captions cut at `FRAME_CAPTION_MAX_CHARS` 2048), and no
reliability number at all (`pass^k` 0 hits in `crates/playtest`). Those three are
small, cheap, and each is exactly what a weak model needs. Everything else here
is second.

## 1. OpenChamber: the surface, and what to copy

OpenChamber is MIT (`github.com/openchamber/openchamber`, README badge; 12,079
stars at read time), an "agentic development environment" around OpenCode. It is
`openchamber/openchamber`, and it states it is independent of the OpenCode team
— the `maximkone` attribution in the brief is wrong.

What its agent tool surface offers (all from
<https://docs.openchamber.dev/agent-control-tool/>):

| surface | verbs it exposes | copy into our door? |
|---|---|---|
| control | list projects, search models, create/follow-up a session, fork, create a worktree session, `status`, `messages` | **partly**: one multiplexed verb with an `action` enum (we already do this for `play.run`, `play.rs:126-139`) |
| schedule | create/change/run/pause/delete a scheduled task | **no**: our door has no wall clock and the tick must not wait (AGENTS rule 3) |
| knowledge | notes, todos, plans; compare-and-swap on write; ready-todos with dependencies; `## Steps` progress | **yes, the shape**: CAS + dependency-ready query (steal 4) |
| browser | open/snapshot/click/type/scroll/capture | **already have the picture half**: `capture` returns PNG + caption (`play.rs:1466`) |

URLs: <https://docs.openchamber.dev/agent-control-tool/>,
<https://docs.openchamber.dev/notes-todos-plans/>,
<https://docs.openchamber.dev/worktrees/>,
<https://github.com/openchamber/openchamber>.

Three safety rules its docs state, which we already match: the tool "cannot
delete sessions or worktrees, register project paths, run arbitrary shell
commands, or call arbitrary URLs" (the same list as our CUT line in
`docs/plan/ai-door.md:63-65`); uncommitted work is not copied into a new
worktree; a busy checkout gets an offer, not a silent move.

**What not to copy: worktrees.** OpenChamber's best-known feature is one
worktree per session. `AGENTS.md` rule 5 says one tree, one branch, no
worktrees. The need underneath it — a session that cannot step on another
session's files — we already answer with `scene_tools` single-flight and the
AX14 twelve-proposer race test (`docs/plan/ai-native.md:62`).

The verb shape worth stealing outright: a knowledge write carries the text you
last read and is refused if it changed (`expected` compare-and-swap). That is
the guard our `tx.preview` → `tx.apply` approval gives a scene, applied to prose.

## 2. MCP 2026-07-28: what changed, and what we already have

Source: <https://modelcontextprotocol.io/specification/2026-07-28/changelog>.
The revision is stateless: no `initialize` handshake, no `Mcp-Session-Id`,
versions in `_meta`, `server/discover` mandatory; `subscriptions/listen`
replaces the HTTP GET stream and `resources/subscribe`; results carry a required
`resultType`; list results carry `ttlMs` and `cacheScope`; MRTR
(`InputRequiredResult`) replaces server-initiated requests; tasks moved to an
extension `io.modelcontextprotocol/tasks` with `tasks/get` + `tasks/update`.

Where we stand, by grep:

| feature | our state (measured) |
|---|---|
| `server/discover` | present and tested: `tests/it/mcp_stdio.rs:238-240`, `tests/door_parity.rs:147` |
| `resultType` | present: `mcp.rs:1500` `result_type: Some(ResultType::COMPLETE)` |
| `ttlMs` / `cacheScope` | present for ≥ 2026-07-28 clients, deliberately `0` because `api.load` moves the list (`mcp.rs:1494-1505`) |
| structured content + `outputSchema` | present: `packs/mod.rs:437` `tool.output_schema = T::output_schema()`, `play.rs:1469` sets `structured_content` |
| progress notifications | present: `mcp.rs:1212-1423` (`ProgressNote`, one note before a long call, 0 tool-budget chars) |
| cancellation | present as a **tool action**, not a protocol notification: `PlayAction::Cancel` (`play.rs:126`), `cancel()` (`play.rs:910`); `notifications/cancelled` 0 hits |
| **MRTR / elicitation** | **absent**: `elicit\|Elicit` 0 hits in `crates/endpoint`; `InputRequired` 0 hits; every handler returns `Complete` |
| **resource links in results** | **absent**: `ResourceLink\|resource_link` 0 hits; results carry text and image only (`play.rs:1466-1467`) |
| sampling | absent, and that is correct: the spec **deprecates** Roots, Sampling and Logging (changelog "Deprecated" 1) |
| tasks extension | absent; `play.run` multiplexes start/poll/cancel in one tool (`play.rs:16`, `:47`) |

What MRTR unlocks for a weak model: a weak model does not read a refusal message
and retry correctly. It stops. Today `tx.apply` without an approval is a
`McpToolError` with a recovery word (`tool_capabilities.rs::message_carries_recovery`),
which is the best a stateless error can do; with `input_required` the **client**
asks the human and retries the same call, so the round trip costs zero model
turns and the model never has to parse "single-use approval". That is the
largest reliability win available to us per line of code.

What resource links unlock: `play.run` truncates its caption at 2048 chars
(`play.rs:80`) and perf rows at 160 (`packs/perf.rs:399`). A weak model cannot
tell "this is everything" from "this was cut". A `resource_link` block pointing
at `e2040://capture/<digest>` makes truncation explicit and lets the client fetch
the whole body only when it wants it — the same progressive-disclosure move
`api.search`/`api.load` already makes for the tool list.

## 3. Papers and posts with evidence

| claim | number | source | what we take |
|---|---|---|---|
| consistency collapses across trials | gpt-4o pass^1 ≈ 61 % τ-retail, ≈ 35 % τ-airline; pass^8 ≈ 25 % retail | τ-bench, arXiv:2406.12045 (abstract + §1) | a `pass^k` column, not `pass@k` (steal 3) |
| intrinsic self-correction does not work | "LLMs struggle to self-correct ... without external feedback, and at times, their performance even degrades" (ICLR 2024) | arXiv:2310.01798 | never ship "check your own work"; ship a verb the model must call (steal 6) |
| subagents buy breadth, at a price | +90.2 % over single-agent on their internal research eval; 15× chat tokens; token use alone explains 80 % of the BrowseComp variance; rewriting a tool description cut task time 40 % | <https://www.anthropic.com/engineering/multi-agent-research-system> | subagents only for independent work; tool descriptions are a performance knob (steal 5) |
| code-mode collapsing | tool definitions 150,000 → 2,000 tokens, "98.7 %" | <https://www.anthropic.com/engineering/code-execution-with-mcp> | the door does the search/load half; the missing half is reference-not-payload results (steal 2) |
| compaction and just-in-time retrieval | compaction = summarize near the limit, keep the five most recent files; subagent summaries are "often 1,000-2,000 tokens" | <https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents> | our loop has no compaction pass: `compact\|summar` in `crates/provider/src` = 4 hits, all unrelated (`cassette.rs:394` is `jcompact`, a JSON printer) |
| skills are a three-level ladder | Level 1 metadata ~100 tokens; Level 2 body under 5k tokens; Level 3 files free until read | <https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview> | `prompt_list` is the Level 1/2 shape already; the missing piece is Level 3 as **resources** the tools never link to |

Plan-and-execute versus ReAct: I found no controlled comparison I could read
that isolates the two on our kind of task, so this card makes no claim about it.
What the evidence does support is narrower and enough: external checks beat
self-critique (row 2), and consistency must be measured across trials (row 1).

## 4. Local serving on a 3050 6 GB

| option | licence | fits a 3050? | verdict |
|---|---|---|---|
| **llama.cpp** (`ggml-org/llama.cpp`, MIT, `LICENSE` read) | MIT | yes: a Q4 GGUF of a 3-4B model is ~2-3 GB | **the pick**; it is what `crates/provider/src/local.rs` already speaks (`/v1/chat/completions`, loopback only) |
| vLLM | Apache-2.0 | datacenter-shaped: xgrammar/guidance backends, paged KV, `--gpu-memory-utilization`; docs are Linux/CUDA first (`docs.vllm.ai`, read) | no: a second server for the same job |
| mistral.rs | see `EricLBuehler/mistral.rs` licence file (not fetched here) | Rust, in-process, CUDA/Metal; every published benchmark is GB10 / B200 / H100 (README tables), none for a 3050 | no for now: in-process inference in a process that must not block the tick is the wrong shape (AGENTS rule 3) |
| `ort` (ONNX Runtime) | MIT/Apache-2.0 | yes, but you own the tokenizer, the sampler and the KV cache | no: `crates/nn` owns integer nets, and a float LLM in the tick is out of scope |

Structured output is the reason to care, and llama.cpp has both halves: GBNF
grammars and JSON-Schema → GBNF conversion
(<https://raw.githubusercontent.com/ggml-org/llama.cpp/master/grammars/README.md>).
Two measured caveats from that page, worth knowing before we bet on it: the
schema is **not injected into the prompt** ("The model has no visibility into
the schema"), and `anyOf`/`oneOf` cannot mix with `properties`.

Speculative decoding exists and prints its own acceptance rate
(<https://github.com/ggml-org/llama.cpp/blob/master/docs/speculative.md>:
"draft acceptance rate = 0.57576"; default `--spec-draft-n-max 3`). On 6 GB the
draft model competes for VRAM with the target, so the honest statement is:
available, unmeasured here, and second to getting the target model's structured
output right.

Throughput on a 3050 6 GB — **nothing here is our measurement**:

- third-party report: 168 GB/s, 96-bit bus, 6 GB, "keeps a 7B model near 30
  tokens per second" (<https://localllmchecker.com/hardware/rtx-3050-6gb/>).
- third-party report: Qwen3-4B int8 with layer streaming, 4,513 MB peak VRAM,
  **~3.4 tok/s decode** on a 6 GB 3050 laptop
  (<https://dev.to/mycute_97b39c4e34027570f9/running-a-4b-model-in-6-gb-and-training-3b-parameters-on-a-laptop-4al>)
  — streaming weights from RAM, so this is the floor, not the target.
- estimate: a 4B Q4_K_M (~2.5 GB) fully resident at ~168 GB/s gives a ceiling
  near 60 tok/s and a realistic 20-35 tok/s. Mark it an estimate until we run
  `llama-bench` on the actual card.

## 5. Rust crates

| crate | version we hold | licence | note |
|---|---|---|---|
| `rmcp` | 3.4.1 (`Cargo.lock:7875`) | MIT **and** Apache-2.0 (rust-sdk `LICENSE` read: a transition, MIT for older contributions) | already has `InputRequiredResult`, `TaskAckResult`, `CancelledNotification` (`src/model.rs:4642`, `:4579`) — the missing features are in the crate, not in the proxy |
| `schemars` | 1.2.2 (`Cargo.lock:8204`), pulled by rmcp's `schemars` feature | MIT | drives every `output_schema` |
| `tokio` | 1 (workspace `Cargo.toml:207`) | MIT | only for the stdio binary behind the `mcp` feature |
| `tower` | **absent** (`Cargo.toml` has no `tower`) | — | rmcp's service layer is enough; no steal here |
| `ort`, `candle`, `llama-cpp-rs` | absent (`Cargo.toml` greps clean) | — | keep it that way (§4) |

## The steals, ranked

### 1. `tx.apply` answers `input_required` instead of refusing (S)

- What: when `tx.apply` arrives with no approval (or a stale one), return
  `resultType: "input_required"` with an `elicitation/create` form ("apply
  revision N, 3 ops?"), and on the retry read `inputResponses`. Spec:
  <https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/mrtr>.
  rmcp 3.4.1 already models it (`model.rs:4642` `InputRequiredResult`).
- What we lack, by grep: `elicit|Elicit` → **no matches** in `crates/endpoint`;
  `InputRequired|input_required` → **0 hits**; the only `result_type` we build is
  `COMPLETE` (`mcp.rs:1500`).
- Cost: S. One arm in `tx_apply`, one helper building the form from the pending
  preview, one stdio transcript test. No new verb, so **0 tool-budget chars** —
  the budget is 75,000 with the gym load measuring 74,084 (`mcp.rs:151-161`).
- Experiment: a scripted client calls `tx.apply` with no approval, asserts
  `resultType == "input_required"` and that the form names the revision, retries
  with `inputResponses`, and gets the same digest as today's two-tool path. Kill:
  any existing `tx.apply` transcript changes shape for a client that never sends
  `inputResponses` (backwards compatibility is the whole risk).

### 2. Large receipts come back as `resource_link`, not as truncated text (S-M)

- What: add a `ContentBlock::ResourceLink` (`e2040://capture/<digest>`,
  `e2040://perf/<run>`) beside the short caption. Precedent:
  <https://www.anthropic.com/engineering/code-execution-with-mcp> (98.7 % cut
  when the payload stops crossing the model).
- What we lack, by grep: `ResourceLink|resource_link` → **0 hits** in
  `crates/endpoint`; results carry `ContentBlock::image` + `ContentBlock::text`
  only (`play.rs:1466-1467`); the caption is cut at `FRAME_CAPTION_MAX_CHARS`
  2048 (`play.rs:80`), perf details at `MAX_DETAIL_CHARS` 160
  (`packs/perf.rs:399`).
- Cost: S-M. The resource family exists (4 resources, `resources.rs:15`); this
  adds a child URI per capture and a link block. Still 0 tool-budget chars,
  since resources cost none (`mcp.rs:1447-1450`).
- Experiment: one `play.run` capture returns a link; `resources/read` on it
  returns bytes whose digest equals the receipt digest; the text block shrinks
  by ≥ 50 % on a scene with more than 8 fighters. Kill: a client that ignores
  links now sees less than before.

### 3. A `pass^k` column on the eval harness (S)

- What: run each eval row k times and report the fraction of rows that pass
  **all** k, beside the mean. τ-bench's point: pass^1 61 % and pass^8 25 % are the
  same model (arXiv:2406.12045).
- What we lack, by grep: `pass_at|pass_hat|pass\^` → **no matches** in
  `crates/playtest/src`; the 23 `trials` hits are the tournament sweep
  (`tournament/sweep.rs:85`), which counts matches, not repeated runs of one task.
- Cost: S. The harness is `crates/playtest/src/eval.rs` (75 rows + external line,
  `docs/plan/ai-door.md:33`); recorded-replay backends make k runs cheap and
  deterministic.
- Experiment: 75 rows × 8 replays on the mock backend, print `pass1_permille`
  and `pass8_permille`, twice identical. Kill: the two runs differ (then the
  harness is not deterministic and that is the finding).

### 4. Knowledge verbs with compare-and-swap, and dependency-ready todos (M)

- What: `note.put` / `todo.add` / `plan.steps` where a write carries the text
  last read and is refused if it changed, plus a "ready" query (unfinished todos
  whose dependencies are done). Precedent:
  <https://docs.openchamber.dev/notes-todos-plans/> ("send the text they read
  along with a note or plan they replace whole; if it changed since, the write is
  refused").
- What we lack: `crates/endpoint/src/packs/` holds 27 files (26 packs plus
  `mod.rs`), none for notes, todos or plans (directory listing);
  `sim.plan_goals` is a planner over scene goals, not prose (`plan_goals.rs`);
  our board and ladder are markdown files the door cannot see at all.
- Cost: M. The only steal here that needs new verbs, so it pays tool-budget
  chars at the worst time (74,084 of 75,000 on the gym load). It should be **one**
  multiplexed verb (`plan.` + `action`), the way `play.run` multiplexes
  start/poll/cancel.
- Experiment: `plan.steps` read-only over three plan files, returning
  `{done, total}` per plan parsed from `- [ ]` under `## Steps`; no writes, no new
  state, 0 risk. Only if that earns its chars do the CAS writes follow.

### 5. Wire `agent_loop`, and only then talk about subagents (M)

- What: our perceive-plan-act loop exists and nothing calls it
  (`crates/provider/src/agent_loop.rs:6`: "Status: no in-repo consumer yet";
  grep over `crates` for `agent_loop|AgentLoop` outside that file = **one hit**,
  `lib.rs:32` the `pub mod`). The subagent evidence is +90.2 % for breadth-first
  work at 15× tokens (Anthropic), a tax to pay only where tasks are independent.
- What we lack: the loop is a library island; the eval harness has no "one
  agent, N door steps" arm.
- Cost: M. One binary subcommand over stdio, like the existing `e2040-mcp`
  (`docs/plan/ai-door.md:88-92`: door verbs are the only way in).
- Experiment: 20 door tasks, one `agent_loop` arm vs the same tasks split across
  3 sessions; report pass^1 per arm and total tokens (the cassette already
  records tokens). Kill: the parallel arm wins by spending > 5× tokens for < 5
  points — which would confirm the tax, not a win.

### 6. Verification is a verb the model must call, never a self-check sentence (S)

- What: the door already returns `verify` strings on build outputs (D19) and the
  `test.run` verb exists. The evidence says a model told to "check its own work"
  gets worse, not better (arXiv:2310.01798).
- What we lack: nothing in the door — this is a **prompt and workflow** change.
  The gap is in our own sessions: no eval arm compares "self-check" against
  "call `test.run`" (`docs/plan/ai-door.md:33` has the harness; no arm names a
  verifier call).
- Cost: S. Two prompt arms, one fixture, the existing harness.
- Experiment: 20 authoring tasks × 2 arms × k=5; the verifier arm must win on
  pass^5, or the prompt is not worth shipping. The cheapest experiment here, and
  it tests the card's own thesis.

## Ranked table

| # | steal | kind | our gap (grep) | cost | first experiment |
|---|---|---|---|---|---|
| 1 | `input_required` on `tx.apply` | MCP 2026-07-28 MRTR | `elicit` 0 hits, `InputRequired` 0 hits | S, 0 chars | stdio: no-approval apply → form → retry applies |
| 2 | `resource_link` receipts | MCP content types + code-mode | `ResourceLink` 0 hits, caption cut at 2048 | S-M, 0 chars | capture link read-back equals receipt digest |
| 3 | `pass^k` eval column | τ-bench | `pass_hat` 0 hits in playtest | S | 75 rows × 8, twice identical |
| 4 | knowledge verbs with CAS | OpenChamber knowledge | no notes/todos/plans pack | M, pays chars | read-only `plan.steps` on 3 plan files |
| 5 | wire `agent_loop`, then subagents | Anthropic multi-agent | `agent_loop` only `lib.rs:32` | M | 20 tasks, 1 loop vs 3 sessions, tokens + pass^1 |
| 6 | verifier call beats self-check | arXiv:2310.01798 | no eval arm | S | 20 tasks × 2 arms × 5 |

Not stolen, on purpose: OpenChamber worktrees (AGENTS rule 5), its scheduler (no
wall clock in the door), MCP sampling (deprecated in 2026-07-28), the tasks
extension (client support too new to bet on; `play.run` already polls), vLLM and
in-process Rust inference (§4).

## Proposed rows (not boarded)

- **MCP26** endpoint: `tx.apply` answers `input_required` with an approval
  elicitation and applies on the retry; every existing transcript unchanged.
  Accept: one new stdio test + `mcp_stdio::` green x2, golden unchanged.
- **MCP27** endpoint: capture and perf receipts carry a `resource_link` beside
  the caption; `resources/read` returns the same digest. Accept: link read-back
  equals the receipt digest, caption bytes halved.
- **AX18** playtest: `pass^k` columns (`pass1_permille`, `passk_permille` for
  k=8) on the EV1 report. Accept: `cargo test -p engine-playtest --lib pass_k`
  x2, identical.
- **AID2** provider: the local adapter starts on `Rung::Native` against a
  llama.cpp server that supports `json_schema`; the cassette records the rung.
  Accept: the first asked rung is `native`, malformed rate down on the recorded
  fixture. (Measured basis: `LadderPlan::default()` is `Text` at
  `ladder.rs:118-120`, wired at `pins.rs:205`; `local.rs:7` keeps the capability
  at `structured_output: false` even though `WIRE` declares `RungSupport::FULL`.)
- **MCP28** endpoint: one multiplexed `plan` verb (read-only steps first), only
  after AX15 headroom is measured again.
- **AX19** provider/playtest: wire `agent_loop` to a stdio door and add the
  single-agent eval arm.

## What I could not verify

- No number in §4 is ours: no `llama-bench` run, no 3050 in this session. The
  20-35 tok/s band is an estimate and is labelled one.
- OpenChamber is read from its README and docs only; I did not read its source,
  so "one multiplexed control verb" is the shape its docs describe, not a claim
  about its code.
- mistral.rs's licence file was not fetched (only its README); treat that row as
  unverified and do not depend on it.
- The tasks extension (`io.modelcontextprotocol/tasks`) is described in the
  changelog, but I did not read its page, so the note above it is a judgement.
- I did not measure `input_required` end to end because it does not exist here;
  the rmcp capability is read from the crate source at 3.4.1, which is not the
  same as a client that acts on it.
- The `pass^k` numbers are the paper's, on τ-bench domains; I have no evidence
  they transfer to our 75-row authoring harness, only the reason to measure.
